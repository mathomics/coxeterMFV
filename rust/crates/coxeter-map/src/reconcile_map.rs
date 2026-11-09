//! Bringing a saved map up to date with the model it is opened over (port of
//! `src/map/reconcile-map.js`).
//!
//! A map is a drawing of a model at the moment it was saved. Two things move
//! on after that. The model does: a reconstruction that is still being worked
//! on loses a reaction here and renames a metabolite there between one export
//! and the next. And this program does: a map remembers how its compartments
//! were nested when it was drawn, and that nesting comes from the model's
//! compartment tree, which a later build may read differently (a map can
//! have the intermembrane space written `i` drawn as a box beside the cell,
//! where the tree puts it around the matrix).
//!
//! Neither is a reason to turn a map away. What is drawn and still exists is
//! kept exactly where it was put; what the model no longer has is taken off
//! the map; and compartments are moved to where the model nests them, taking
//! their contents along. Everything done is reported, so nothing changes
//! silently.
//!
//! Pure geometry and membership: the model and the LP are never touched.

use std::collections::HashSet;

use coxeter_core::boundaries::{SubtreeCache, reaction_boundary_keys};
use coxeter_core::community::{CommunityModel, LINK_COMPARTMENT};
use coxeter_core::compartments::{Names, cell_compartment_name, cell_compartment_visual_id, membrane_style};
use coxeter_core::js;
use coxeter_core::mapdata::{MapCell, MapCompartment, NodeKind, Point, Rect};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};

use crate::drag::{Placeable, clamp_node_position, update_connected_segments};
use crate::editable_map::{EditableMap, is_metabolite_kind};
use crate::geometry_edit::edit_geometry_rect;
use crate::place_reaction::PlaceOptions;
use crate::reaction_edit::{closest_boundary_point, straighten_linear_reactions};

/// The band a compartment is given around the one it encloses. A metabolite
/// has to fit in it clear of both membranes, which takes 42 units; a little
/// over that leaves room to put it down without touching either.
const WRAP_SHELL: f64 = 52.0;
const MIN_RING: f64 = 44.0;
const CHILD_PAD: f64 = 14.0;

// Contents

/// What `pruneMapToModel` did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PruneReport {
    /// Reactions the model no longer has, taken off the map.
    pub dropped: Vec<String>,
    /// Reactions drawn afresh because a metabolite they were drawn with is gone.
    pub redrawn: Vec<String>,
    /// Reactions left as they are, whose drawn metabolites differ from the model's.
    pub changed: Vec<String>,
    /// Free-standing metabolites the model has lost, removed.
    pub nodes: usize,
}

impl PruneReport {
    /// The JSON the JavaScript report object would print.
    pub fn to_json(&self) -> Value {
        json!({ "dropped": self.dropped, "redrawn": self.redrawn, "changed": self.changed, "nodes": self.nodes })
    }
}

/// `pruneMapToModel(map, community, {keepSpecies})`: take off the map what the
/// model no longer has.
///
/// A reaction that is gone is removed, with the nodes only it used. A
/// reaction that is still there but was drawn with a metabolite the model no
/// longer knows has changed its stoichiometry; it is drawn afresh in the same
/// place, because a curve to a metabolite that does not exist cannot be kept.
/// A reaction whose drawn metabolites merely differ from the model's is left
/// as it is and named.
///
/// Species in `keep_species` are switched off, not gone: what they have drawn
/// is missing from the community only while they are out, and is kept. That
/// is their own `sp@…` reactions and metabolites, and also the shared-medium
/// nodes those reactions are drawn to and the community exchanges on those
/// nodes.
pub fn prune_map_to_model(map: &mut EditableMap, community: &CommunityModel, keep_species: &HashSet<String>) -> PruneReport {
    let mut report = PruneReport::default();
    let reactions = &community.merged.reactions;
    let metabolites = &community.merged.metabolites;
    let kept = |id: &str| !keep_species.is_empty() && keep_species.contains(id.split('@').next().unwrap_or(""));
    let mut kept_nodes: HashSet<String> = HashSet::new();
    let mut kept_reactions: HashSet<String> = HashSet::new();
    if !keep_species.is_empty() {
        for seg in &map.g.segments {
            if let Some(n) = seg.node_id.as_deref().filter(|n| !n.is_empty()) {
                if kept(&seg.reaction_id) {
                    kept_nodes.insert(n.to_string());
                }
            }
        }
        for seg in &map.g.segments {
            // Community-level only (`EX_x_s`): a missing `sp@…` reaction of a
            // species that is still on is really gone, even when it shares
            // the node.
            let Some(n) = seg.node_id.as_deref().filter(|n| !n.is_empty()) else { continue };
            if kept_nodes.contains(n) && !reactions.contains_key(&seg.reaction_id) && !seg.reaction_id.contains('@') {
                kept_reactions.insert(seg.reaction_id.clone());
            }
        }
    }

    for id in map.reaction_ids() {
        if reactions.contains_key(&id) || kept(&id) || kept_reactions.contains(&id) {
            continue;
        }
        map.remove_reaction(&id);
        report.dropped.push(id);
    }

    // reaction id -> merged ids of its metabolite nodes
    let mut drawn_with: IndexMap<String, IndexSet<String>> = IndexMap::new();
    for seg in &map.g.segments {
        let Some(n) = seg.node_id.as_deref().filter(|n| !n.is_empty()) else { continue };
        let Some(merged) = map.g.nodes.get(n).and_then(|node| node.merged_id.clone()).filter(|m| !m.is_empty()) else { continue };
        drawn_with.entry(seg.reaction_id.clone()).or_default().insert(merged);
    }

    for (id, drawn) in &drawn_with {
        let Some(rxn) = map.g.reactions.get(id) else { continue };
        let Some(model_rxn) = reactions.get(id) else { continue };
        let broken = drawn.iter().any(|mid| !metabolites.contains_key(mid));
        if broken {
            let at = Point::new(rxn.x, rxn.y);
            map.remove_reaction(id);
            if map.add_reaction(community, id, &PlaceOptions { anchor: Some(at), ..Default::default() }).is_some() {
                report.redrawn.push(id.clone());
            } else {
                report.dropped.push(id.clone());
            }
            continue;
        }
        let wanted: Vec<&String> = model_rxn
            .metabolites
            .keys()
            .filter(|mid| metabolites.get(*mid).map(|m| m.compartment.as_str()) != Some(LINK_COMPARTMENT))
            .collect();
        if wanted.len() != drawn.len() || wanted.iter().any(|mid| !drawn.contains(*mid)) {
            report.changed.push(id.clone());
        }
    }

    // What is left are metabolites standing on their own that the model has lost.
    for id in map.node_ids() {
        let Some(node) = map.g.nodes.get(&id) else { continue };
        let merged = node.merged_id.as_deref().unwrap_or("");
        if !is_metabolite_kind(&node.kind) || merged.is_empty() || metabolites.contains_key(merged) || kept(merged) || kept_nodes.contains(&id) {
            continue;
        }
        map.delete_node(&id);
        map.retain_segments(|seg| seg.node_id.as_deref() != Some(id.as_str()));
        report.nodes += 1;
    }
    let nodes = &map.g.nodes;
    let rxns = &map.g.reactions;
    map.g.medium.nodes.retain(|id| nodes.contains_key(id));
    map.g.medium.exchanges.retain(|id| rxns.contains_key(id));
    report
}

// Compartments

/// A compartment put around another one.
#[derive(Debug, Clone, PartialEq)]
pub struct Wrapped {
    pub cell: String,
    pub id: String,
    pub around: Vec<String>,
    /// It was not drawn at all and had to be created.
    pub created: bool,
}

/// A compartment moved to where the model nests it.
#[derive(Debug, Clone, PartialEq)]
pub struct MovedCompartment {
    pub cell: String,
    pub id: String,
    /// None: it now stands on its own beside the cell.
    pub into: Option<String>,
}

/// A compartment that could not be put right, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct LeftCompartment {
    pub cell: String,
    pub id: String,
    pub reason: String,
}

/// What `reconcileCompartments` did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShellReport {
    pub wrapped: Vec<Wrapped>,
    pub moved: Vec<MovedCompartment>,
    pub left: Vec<LeftCompartment>,
    /// Reactions drawn afresh in their new place.
    pub reseated: Vec<String>,
    /// Metabolites pulled back inside their compartment.
    pub pulled: usize,
}

impl ShellReport {
    /// The JSON the JavaScript report object would print.
    pub fn to_json(&self) -> Value {
        json!({
            "wrapped": self.wrapped.iter().map(|w| {
                let mut m = Map::new();
                m.insert("cell".into(), json!(w.cell));
                m.insert("id".into(), json!(w.id));
                m.insert("around".into(), json!(w.around));
                if w.created {
                    m.insert("created".into(), json!(true));
                }
                Value::Object(m)
            }).collect::<Vec<_>>(),
            "moved": self.moved.iter().map(|m| json!({ "cell": m.cell, "id": m.id, "into": m.into })).collect::<Vec<_>>(),
            "left": self.left.iter().map(|l| json!({ "cell": l.cell, "id": l.id, "reason": l.reason })).collect::<Vec<_>>(),
            "reseated": self.reseated,
            "pulled": self.pulled,
        })
    }
}

/// A reaction taken off to be drawn afresh.
struct Pending {
    id: String,
    at: Point,
}

fn take_off(map: &mut EditableMap, pending: &mut Vec<Pending>, ids: impl IntoIterator<Item = String>) {
    for id in ids {
        let Some(rxn) = map.g.reactions.get(&id) else { continue };
        pending.push(Pending { id: id.clone(), at: Point::new(rxn.x, rxn.y) });
        map.remove_reaction(&id);
    }
}

fn comp_index(cell: &MapCell, id: &str) -> Option<usize> {
    cell.compartments.iter().position(|c| c.id == id)
}

/// Put `outer` around `inner`, which stands in `parent`.
fn wrap(map: &mut EditableMap, ci: usize, pending: &mut Vec<Pending>, outer: &str, inner: &str, parent: &str) {
    let cell_id = map.g.cells[ci].id.clone();
    // What reaches into the outer compartment was drawn to wherever it used
    // to be, or to nowhere; it is drawn again once the two are in place.
    let touching = reactions_touching(map, &cell_id, outer);
    take_off(map, pending, touching);
    let (oi, ii) = {
        let cell = &map.g.cells[ci];
        (comp_index(cell, outer).expect("outer drawn"), comp_index(cell, inner).expect("inner drawn"))
    };
    let bx = map.g.cells[ci].compartments[ii].rect;
    {
        let cell = &mut map.g.cells[ci];
        cell.compartments[oi].rect = bx;
        cell.compartments[oi].parent_id = Some(parent.to_string());
        cell.compartments[ii].parent_id = Some(outer.to_string());
        set_depths(cell);
    }
    map.refresh_boundaries();
    // The inner one gives up a band along its edge, as far as what is drawn
    // in it allows.
    edit_geometry_rect(map, Some(&cell_id), inner, inset(&bx, WRAP_SHELL));
    // Where its contents would not let it, the outer one grows instead, if
    // its neighbours leave it the room.
    let cell = &map.g.cells[ci];
    let siblings: Vec<Rect> = cell
        .compartments
        .iter()
        .enumerate()
        .filter(|(k, c)| c.parent_id.as_deref() == Some(parent) && *k != oi)
        .map(|(_, c)| c.rect)
        .collect();
    let parent_rect = cell.compartments[comp_index(cell, parent).expect("parent drawn")].rect;
    let limit = inset(&parent_rect, 6.0);
    for side in ["left", "right", "top", "bottom"] {
        let cell = &map.g.cells[ci];
        let short = MIN_RING - ring_width(&cell.compartments[oi].rect, &cell.compartments[ii].rect, side);
        if short <= 0.0 {
            continue;
        }
        let grown = grow(&cell.compartments[oi].rect, side, short);
        if contains(&limit, &grown) && !siblings.iter().any(|c| overlaps(&grown, c, 4.0)) {
            map.g.cells[ci].compartments[oi].rect = grown;
        }
    }
    map.refresh_boundaries();
}

/// `reconcileCompartments(map, community)`: nest the drawn compartments the
/// way the model nests them, and put back inside its compartment whatever is
/// drawn outside it.
///
/// Three things can be wrong with a compartment, and each has its own remedy:
///
/// - it has to go around one that is already standing (the intermembrane
///   space around the matrix): it takes that one's box, and the inner one is
///   drawn in a little, so nothing next to them has to move;
/// - it is drawn in the wrong place altogether (the eyespot beside the cell):
///   it moves into its parent as it is, contents and all, to a free spot;
/// - its contents are not in it (earlier builds drew a cell's `s` reactions
///   out in the shared medium): they are drawn afresh inside it.
///
/// What is already where it belongs is not touched: a map that needs nothing
/// comes out exactly as it went in.
pub fn reconcile_compartments(map: &mut EditableMap, community: &CommunityModel) -> ShellReport {
    let mut report = ShellReport::default();
    let mut touched: IndexSet<String> = mend_node_ownership(map, community); // cells to look over

    for sp in community.active_species() {
        let Some(ci) = map.g.cells.iter().position(|c| c.id == sp.id) else { continue };
        let cell_id = sp.id.clone();
        let tree = sp.model.compartment_tree();
        let names: &Names = &sp.model.compartment_names;
        let mut pending: Vec<Pending> = Vec::new();

        // Outside in, so a compartment's parent is settled before it is.
        let mut order: Vec<&coxeter_core::compartments::TreeNode> = tree.by_id.values().collect();
        order.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| js::locale_compare(&a.id, &b.id)));
        for node in order {
            let cell = &map.g.cells[ci];
            let Some(ki) = comp_index(cell, &node.id) else { continue };
            let comp = cell.compartments[ki].clone();
            let want = node.parent.clone();
            if comp.parent_id == want {
                continue;
            }
            let parent_drawn = want.as_deref().is_some_and(|w| comp_index(cell, w).is_some());

            // The model has put a compartment between this one and the one it
            // was drawn in, and that compartment is not drawn at all. Draw it
            // here: nothing else could ever get it around what it has to
            // contain.
            if let (Some(w), false) = (&want, parent_drawn) {
                let between = tree.by_id.get(w);
                let outer_parent = between.and_then(|b| comp_index(cell, b.parent.as_deref().unwrap_or(""))).map(|k| cell.compartments[k].id.clone());
                match (between, outer_parent) {
                    (Some(b), Some(op)) if comp.parent_id == b.parent => {
                        let visual_id = cell_compartment_visual_id(w, names);
                        let created = MapCompartment {
                            id: w.clone(),
                            rect: comp.rect,
                            depth: 0.0,
                            parent_id: Some(op.clone()),
                            label: cell_compartment_name(w, names, "en"),
                            membrane: membrane_style(&visual_id, &Names::new()).as_str().into(),
                            visual_id,
                            fill: None,
                            stroke: None,
                            shape: None,
                            extra: Map::new(),
                        };
                        map.g.cells[ci].compartments.push(created);
                        wrap(map, ci, &mut pending, w, &comp.id, &op);
                        touched.insert(cell_id.clone());
                        report.wrapped.push(Wrapped { cell: cell_id.clone(), id: w.clone(), around: vec![comp.id.clone()], created: true });
                    }
                    _ => report.left.push(LeftCompartment {
                        cell: cell_id.clone(),
                        id: comp.id.clone(),
                        reason: format!("its parent {w} is not drawn"),
                    }),
                }
                continue;
            }

            let Some(want) = want else {
                // Was nested, is now a compartment of its own beside the
                // cell. It keeps its place; only what it is said to be inside
                // changes.
                map.g.cells[ci].compartments[ki].parent_id = None;
                touched.insert(cell_id.clone());
                report.moved.push(MovedCompartment { cell: cell_id.clone(), id: comp.id.clone(), into: None });
                continue;
            };

            // Drawn elsewhere, and it has to go around a compartment that is
            // already standing in its parent.
            let inside: Vec<String> = cell
                .compartments
                .iter()
                .filter(|c| tree.by_id.get(&c.id).and_then(|n| n.parent.as_deref()) == Some(comp.id.as_str()) && c.parent_id.as_deref() == Some(want.as_str()))
                .map(|c| c.id.clone())
                .collect();
            let own_children = cell.compartments.iter().filter(|c| c.parent_id.as_deref() == Some(comp.id.as_str())).count();
            if inside.len() == 1 && own_children == 0 {
                wrap(map, ci, &mut pending, &comp.id, &inside[0], &want);
                touched.insert(cell_id.clone());
                report.wrapped.push(Wrapped { cell: cell_id.clone(), id: comp.id.clone(), around: vec![inside[0].clone()], created: false });
                continue;
            }

            // Otherwise it moves into its parent as it is, with everything in it.
            let Some(spot) = find_spot(map, ci, ki, &want) else {
                report.left.push(LeftCompartment {
                    cell: cell_id.clone(),
                    id: comp.id.clone(),
                    reason: format!("there is no room for it in {want}"),
                });
                continue;
            };
            let was = (comp.parent_id.clone(), comp.rect);
            {
                let cell = &mut map.g.cells[ci];
                cell.compartments[ki].parent_id = Some(want.clone());
                if spot.resized {
                    let r = cell.compartments[ki].rect;
                    cell.compartments[ki].rect = Rect::new(r.x, r.y, spot.w, spot.h);
                }
                set_depths(cell);
            }
            map.refresh_boundaries();
            let r = map.g.cells[ci].compartments[ki].rect;
            if edit_geometry_rect(map, Some(&cell_id), &comp.id, Rect::new(spot.x, spot.y, r.w, r.h)).is_none() {
                let cell = &mut map.g.cells[ci];
                cell.compartments[ki].parent_id = was.0;
                cell.compartments[ki].rect = was.1;
                set_depths(cell);
                map.refresh_boundaries();
                report.left.push(LeftCompartment {
                    cell: cell_id.clone(),
                    id: comp.id.clone(),
                    reason: format!("it could not be moved into {want}"),
                });
                continue;
            }
            touched.insert(cell_id.clone());
            report.moved.push(MovedCompartment { cell: cell_id.clone(), id: comp.id.clone(), into: Some(want) });
        }

        if !touched.contains(&cell_id) {
            continue;
        }

        {
            let cell = &mut map.g.cells[ci];
            set_depths(cell);
            let roots: Vec<(String, Rect)> = cell.compartments.iter().filter(|c| c.parent_id.is_none()).map(|c| (c.id.clone(), c.rect)).collect();
            if !roots.is_empty() {
                cell.rect = union(&roots.iter().map(|r| r.1).collect::<Vec<_>>());
            }
            if !roots.iter().any(|r| Some(&r.0) == cell.outer_compartment.as_ref()) {
                let pick = roots.iter().find(|r| r.0 == "e").or_else(|| roots.first()).map(|r| r.0.clone());
                if let Some(p) = pick {
                    cell.outer_compartment = Some(p);
                }
            }
        }
        map.refresh_boundaries();
        let boxes: IndexMap<String, Rect> = map.g.cells[ci].compartments.iter().map(|c| (c.id.clone(), c.rect)).collect();

        // A transporter has to be on a membrane it crosses, which for the
        // same reaction is a different one once the nesting changes. And a
        // reaction inside a compartment has to be inside it.
        let mut cache = SubtreeCache::new();
        let mut misplaced: Vec<String> = Vec::new();
        let mut strays: IndexMap<String, Vec<String>> = IndexMap::new(); // compartment id -> reactions drawn outside it
        for (id, rxn) in &map.g.reactions {
            if rxn.cell_id.as_deref() != Some(cell_id.as_str()) {
                continue;
            }
            if rxn.on_membrane {
                if let Some(bid) = rxn.boundary_id.as_deref().filter(|b| !b.is_empty()) {
                    if !reaction_boundary_keys(community, id, Some(&mut cache)).iter().any(|k| k == bid) {
                        misplaced.push(id.clone());
                    }
                    continue;
                }
            }
            let home = rxn.compartments.iter().find_map(|cid| boxes.get_key_value(cid));
            if let Some((home_id, rect)) = home {
                if !holds(rect, rxn.x, rxn.y) {
                    strays.entry(home_id.clone()).or_default().push(id.clone());
                }
            }
        }
        take_off(map, &mut pending, misplaced);
        // Reactions found outside their compartment all want the same nearest
        // corner of it; they are shared out over the box instead.
        let mut places: IndexMap<String, Point> = IndexMap::new();
        for (comp_id, ids) in strays {
            let r = boxes[&comp_id];
            let n = ids.len() as f64;
            let cols = js::max(1.0, ((n * r.w) / js::max(r.h, 1.0)).sqrt().ceil());
            let rows = js::max(1.0, (n / cols).ceil());
            for (k, id) in ids.iter().enumerate() {
                let k = k as f64;
                places.insert(
                    id.clone(),
                    Point::new(r.x + (((k % cols) + 0.5) / cols) * r.w, r.y + (((k / cols).floor() + 0.5) / rows) * r.h),
                );
            }
            take_off(map, &mut pending, ids);
        }

        for p in &pending {
            if map.g.reactions.contains_key(&p.id) {
                continue;
            }
            let anchor = places.get(&p.id).copied().unwrap_or(p.at);
            if map.add_reaction(community, &p.id, &PlaceOptions { anchor: Some(anchor), ..Default::default() }).is_some() {
                report.reseated.push(p.id.clone());
            }
        }

        // Transporters still on the right membrane, whose membrane has moved.
        for id in map.reaction_ids() {
            let rxn = &map.g.reactions[&id];
            if rxn.cell_id.as_deref() != Some(cell_id.as_str()) || !rxn.on_membrane {
                continue;
            }
            let Some(bid) = rxn.boundary_id.as_deref().filter(|b| !b.is_empty()) else { continue };
            let Some(boundary) = map.boundary(bid).filter(|b| b.rect.is_some()) else { continue };
            let p = closest_boundary_point(map, boundary, Point::new(rxn.x, rxn.y));
            let (x, y) = (rxn.x, rxn.y);
            if js::hypot(p.x - x, p.y - y) > 0.5 {
                map.move_reaction(&id, x, y);
            }
        }

        // Last, any metabolite still outside the compartment it belongs to.
        for id in map.node_ids() {
            let node = &map.g.nodes[&id];
            if node.cell_id.as_deref() != Some(cell_id.as_str()) || node.kind == NodeKind::Medium || !is_metabolite_kind(&node.kind) {
                continue;
            }
            let Some(home) = node.compartment.as_ref().and_then(|c| boxes.get(c)) else { continue };
            if holds(home, node.x, node.y) {
                continue;
            }
            let p = clamp_node_position(map, Placeable::from(node), node.x, node.y);
            let dx = p.x - node.x;
            let dy = p.y - node.y;
            if dx.abs() + dy.abs() < 0.01 {
                continue;
            }
            let node = map.g.nodes.get_mut(&id).expect("listed");
            node.x = p.x;
            node.y = p.y;
            update_connected_segments(map, &id, Some((dx, dy)));
            report.pulled += 1;
        }
    }

    if !touched.is_empty() {
        map.refresh_boundaries();
        straighten_linear_reactions(map, None);
    }
    report
}

/// `describeReconciliation(contents, shell)`: say in one sentence what
/// bringing a map up to date changed, or `""` if nothing.
pub fn describe_reconciliation(contents: Option<&PruneReport>, shell: Option<&ShellReport>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let list = |ids: &[String]| -> String {
        let names: Vec<String> = ids
            .iter()
            .take(4)
            .map(|id| match id.find('@') {
                Some(i) => id[i + 1..].to_string(),
                None => id.clone(),
            })
            .collect();
        let more = if ids.len() > 4 { format!(" and {} more", ids.len() - 4) } else { String::new() };
        format!("{}{more}", names.join(", "))
    };
    if let Some(c) = contents {
        if !c.dropped.is_empty() {
            let n = c.dropped.len();
            parts.push(format!(
                "{n} reaction{} the model no longer has {} left out ({})",
                if n == 1 { "" } else { "s" },
                if n == 1 { "was" } else { "were" },
                list(&c.dropped)
            ));
        }
        if !c.redrawn.is_empty() {
            parts.push(format!("{} drawn afresh because the model has changed their metabolites ({})", c.redrawn.len(), list(&c.redrawn)));
        }
        if !c.changed.is_empty() {
            parts.push(format!("{} still drawn with metabolites the model has since changed ({})", c.changed.len(), list(&c.changed)));
        }
        if c.nodes > 0 {
            parts.push(format!("{} metabolite{} the model no longer has removed", c.nodes, if c.nodes == 1 { "" } else { "s" }));
        }
    }
    if let Some(s) = shell {
        let mut placed: Vec<String> = s.wrapped.iter().map(|w| format!("{} around {}", w.id, w.around.join(", "))).collect();
        placed.extend(s.moved.iter().map(|m| match &m.into {
            Some(into) if !into.is_empty() => format!("{} into {into}", m.id),
            _ => format!("{} out beside the cell", m.id),
        }));
        if !placed.is_empty() {
            parts.push(format!("compartments put where the model nests them ({})", placed.join("; ")));
        }
        if !s.reseated.is_empty() {
            let n = s.reseated.len();
            parts.push(format!("{n} reaction{} there drawn afresh ({})", if n == 1 { "" } else { "s" }, list(&s.reseated)));
        }
        if !s.left.is_empty() {
            parts.push(s.left.iter().map(|l| format!("{} left where it was: {}", l.id, l.reason)).collect::<Vec<_>>().join("; "));
        }
    }
    if parts.is_empty() { String::new() } else { format!(" Brought up to date with the model: {}.", parts.join("; ")) }
}

// Helpers

/// A metabolite node says where it lives through `kind` and `cellId`, and maps
/// saved by earlier builds have two things wrong there. A metabolite in a
/// cell's own `s` compartment was taken for one in the shared medium, which
/// shares the letter, and held out in the medium. And a real medium node was
/// filed under the cell of the reaction that placed it, so that moving that
/// cell's compartments would now carry it off with them.
fn mend_node_ownership(map: &mut EditableMap, community: &CommunityModel) -> IndexSet<String> {
    let mut cells = IndexSet::new();
    let mut demoted: Vec<String> = Vec::new();
    for node in map.g.nodes.values_mut() {
        if !is_metabolite_kind(&node.kind) {
            continue;
        }
        let Some(owner) = node.merged_id.as_deref().and_then(|m| community.metabolite_owner.get(m)) else { continue };
        let Some(species) = &owner.species_id else {
            if node.kind == NodeKind::Medium {
                node.cell_id = None;
            }
            continue;
        };
        if node.kind != NodeKind::Medium {
            continue;
        }
        node.kind = NodeKind::Metabolite;
        node.cell_id = Some(species.clone());
        node.compartment = Some(owner.compartment.clone());
        demoted.push(node.id.clone());
        cells.insert(species.clone());
    }
    map.g.medium.nodes.retain(|id| !demoted.contains(id));
    cells
}

/// Reactions drawn with a metabolite in one compartment of one cell.
fn reactions_touching(map: &EditableMap, cell_id: &str, compartment_id: &str) -> Vec<String> {
    let mut out: IndexSet<String> = IndexSet::new();
    for seg in &map.g.segments {
        let node = seg.node_id.as_deref().filter(|n| !n.is_empty()).and_then(|n| map.g.nodes.get(n));
        if node.is_some_and(|n| n.cell_id.as_deref() == Some(cell_id) && n.compartment.as_deref() == Some(compartment_id)) {
            out.insert(seg.reaction_id.clone());
        }
    }
    out.into_iter().collect()
}

fn holds(rect: &Rect, x: f64, y: f64) -> bool {
    x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
}

/// How wide the band between an enclosing box and the enclosed one is on one side.
fn ring_width(outer: &Rect, inner: &Rect, side: &str) -> f64 {
    match side {
        "left" => inner.x - outer.x,
        "right" => (outer.x + outer.w) - (inner.x + inner.w),
        "top" => inner.y - outer.y,
        _ => (outer.y + outer.h) - (inner.y + inner.h),
    }
}

fn grow(r: &Rect, side: &str, by: f64) -> Rect {
    match side {
        "left" => Rect::new(r.x - by, r.y, r.w + by, r.h),
        "right" => Rect::new(r.x, r.y, r.w + by, r.h),
        "top" => Rect::new(r.x, r.y - by, r.w, r.h + by),
        _ => Rect::new(r.x, r.y, r.w, r.h + by),
    }
}

fn set_depths(cell: &mut MapCell) {
    let parents: IndexMap<String, Option<String>> = cell.compartments.iter().map(|c| (c.id.clone(), c.parent_id.clone())).collect();
    for comp in &mut cell.compartments {
        let mut depth = 0.0;
        let mut at = comp.id.clone();
        let mut seen: HashSet<String> = HashSet::from([comp.id.clone()]);
        loop {
            let Some(Some(p)) = parents.get(&at) else { break };
            if p.is_empty() || !parents.contains_key(p) || seen.contains(p) {
                break;
            }
            at = p.clone();
            seen.insert(at.clone());
            depth += 1.0;
        }
        comp.depth = depth;
    }
}

struct Spot {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    resized: bool,
}

/// Where in `parent` a compartment drawn somewhere else can be set down:
/// clear of the other compartments in there, and on as little of what is
/// already drawn as possible. An empty compartment is cut down to fit; one
/// with something in it keeps its size, since its contents keep their
/// arrangement.
fn find_spot(map: &EditableMap, ci: usize, ki: usize, parent_id: &str) -> Option<Spot> {
    let cell = &map.g.cells[ci];
    let comp = &cell.compartments[ki];
    let parent = &cell.compartments[comp_index(cell, parent_id)?];
    let area = inset(&parent.rect, CHILD_PAD);
    let mut subtree: HashSet<String> = HashSet::from([comp.id.clone()]);
    let mut grew = true;
    while grew {
        grew = false;
        for c in &cell.compartments {
            if let Some(p) = c.parent_id.as_deref().filter(|p| !p.is_empty()) {
                if subtree.contains(p) && !subtree.contains(&c.id) {
                    subtree.insert(c.id.clone());
                    grew = true;
                }
            }
        }
    }
    // Only what is actually drawn in the box travels with it. Contents that
    // are filed under the compartment but drawn somewhere else are put right
    // later.
    let occupied = subtree.len() > 1
        || map.g.nodes.values().any(|n| {
            n.cell_id.as_deref() == Some(cell.id.as_str()) && n.compartment.as_ref().is_some_and(|c| subtree.contains(c)) && holds(&comp.rect, n.x, n.y)
        })
        || map.g.reactions.values().any(|r| {
            r.cell_id.as_deref() == Some(cell.id.as_str())
                && holds(&comp.rect, r.x, r.y)
                && (r.compartments.iter().any(|id| subtree.contains(id)) || r.membrane.as_ref().is_some_and(|m| subtree.contains(m)))
        });

    let siblings: Vec<Rect> = cell
        .compartments
        .iter()
        .enumerate()
        .filter(|(k, c)| c.parent_id.as_deref() == Some(parent.id.as_str()) && *k != ki)
        .map(|(_, c)| c.rect)
        .collect();
    let mut marks: Vec<Point> = Vec::new();
    for node in map.g.nodes.values() {
        if node.hidden != Some(true) {
            marks.push(node.pos());
        }
    }
    for rxn in map.g.reactions.values() {
        marks.push(Point::new(rxn.x, rxn.y));
    }
    for text in map.g.texts.values() {
        marks.push(Point::new(text.x, text.y));
    }
    let from = Point::new(comp.rect.x + comp.rect.w / 2.0, comp.rect.y + comp.rect.h / 2.0);

    let search = |w: f64, h: f64, resized: bool| -> Option<Spot> {
        if w > area.w || h > area.h {
            return None;
        }
        let step = js::max_n([24.0, js::min(w, h) / 3.0, js::max(area.w - w, area.h - h) / 60.0]);
        let mut best: Option<(Spot, f64, f64)> = None;
        let mut y = area.y;
        while y <= area.y + area.h - h + 0.5 {
            let mut x = area.x;
            while x <= area.x + area.w - w + 0.5 {
                let r = Rect::new(x, y, w, h);
                if !siblings.iter().any(|s| overlaps(&r, s, CHILD_PAD)) {
                    let grown = inset(&r, -24.0);
                    let covered = marks.iter().filter(|m| m.x >= grown.x && m.x <= grown.x + grown.w && m.y >= grown.y && m.y <= grown.y + grown.h).count() as f64;
                    let away = js::hypot(x + w / 2.0 - from.x, y + h / 2.0 - from.y);
                    let better = match &best {
                        None => true,
                        Some((_, bc, ba)) => covered < *bc || (covered == *bc && away < *ba),
                    };
                    if better {
                        best = Some((Spot { x, y, w, h, resized }, covered, away));
                    }
                }
                x += step;
            }
            y += step;
        }
        best.map(|b| b.0)
    };

    if occupied {
        return search(comp.rect.w, comp.rect.h, false);
    }
    // Nothing in it to preserve: try it at a fair size, then smaller, until
    // there is a place between its new neighbours that it fits.
    for share in [0.22, 0.16, 0.12, 0.09, 0.06, 0.04, 0.0] {
        // The last try is the smallest box a compartment can usefully be.
        let (least_w, least_h) = if share != 0.0 { (160.0, 110.0) } else { (96.0, 70.0) };
        let w = js::max(least_w, js::min(comp.rect.w, area.w * share));
        let h = js::max(least_h, js::min(comp.rect.h, area.h * share));
        if let Some(spot) = search(w, h, w != comp.rect.w || h != comp.rect.h) {
            return Some(spot);
        }
    }
    None
}

fn union(rects: &[Rect]) -> Rect {
    let x1 = js::min_n(rects.iter().map(|r| r.x));
    let y1 = js::min_n(rects.iter().map(|r| r.y));
    let x2 = js::max_n(rects.iter().map(|r| r.x + r.w));
    let y2 = js::max_n(rects.iter().map(|r| r.y + r.h));
    Rect::new(x1, y1, x2 - x1, y2 - y1)
}

/// This module's own `inset`: no floor at zero.
fn inset(r: &Rect, d: f64) -> Rect {
    Rect::new(r.x + d, r.y + d, r.w - 2.0 * d, r.h - 2.0 * d)
}

fn contains(outer: &Rect, inner: &Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

fn overlaps(a: &Rect, b: &Rect, gap: f64) -> bool {
    !(a.x + a.w + gap <= b.x || b.x + b.w + gap <= a.x || a.y + a.h + gap <= b.y || b.y + b.h + gap <= a.y)
}
