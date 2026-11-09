//! Multi-selection helpers for the editable map (port of
//! `src/map/multi-selection.js`, plus `alignSelectionInLine` of
//! `editable-map.js`). Selection is graphical UI state. It never changes the
//! model or FBA.
//!
//! A rectangle that encloses a whole compartment selects the compartment
//! too, and `drag_selection` moves a selection the way the map's drag does,
//! carrying its compartments along; `translate_selection` moves the members
//! one by one.

use std::collections::{HashMap, HashSet};

use coxeter_core::js;
use coxeter_core::mapdata::{MapText, NodeKind, Point, Rect};
use coxeter_core::metabolite_label::{is_secondary_node, turn_metabolite_label};
use coxeter_core::reaction_label::{reaction_label_offset, set_reaction_label_offset};
use coxeter_core::text_style::{TextBox, text_box_of, text_centre};
use indexmap::{IndexMap, IndexSet};

use crate::drag::{Moving, Placeable, is_draggable_node, node_fits_at, update_connected_segments};
use crate::editable_map::{EditableMap, LINE_ALIGN_GAP};
use crate::reaction_edit::{
    ensure_reaction_backbone, keep_straight_through, reaction_centre_fits, reaction_line_ends, straighten_linear_reactions,
    tidy_reaction, turn_reaction_about,
};

/// What is selected: node, reaction and text ids, in the order they were
/// added, and whole compartments as `(cell id, compartment id)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    pub nodes: IndexSet<String>,
    pub reactions: IndexSet<String>,
    pub texts: IndexSet<String>,
    /// Compartments a rectangle enclosed entirely. They are moved with the
    /// selection (with everything inside them) and nothing else acts on them.
    pub compartments: IndexSet<(String, String)>,
}

/// `emptySelection()`.
pub fn empty_selection() -> Selection {
    Selection::default()
}

/// `selectionSize(selection)`.
pub fn selection_size(s: &Selection) -> usize {
    s.nodes.len() + s.reactions.len() + s.texts.len() + s.compartments.len()
}

/// `textBox(annotation)`: where a piece of free text sits. The renderer
/// anchors annotations at the start of the alphabetic baseline, so `(x, y)` is
/// the bottom-left of the glyphs; the box is reconstructed from the font's
/// average character width, a little generous, which only ever makes a label
/// easier to catch.
pub fn text_box(t: &MapText) -> TextBox {
    text_box_of(t)
}

/// `textCentre(annotation)`.
pub fn text_center(t: &MapText) -> Point {
    text_centre(t)
}

/// A selection rectangle by two corners, in any order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionRect {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

impl SelectionRect {
    pub fn from_corners(x1: f64, y1: f64, x2: f64, y2: f64) -> SelectionRect {
        SelectionRect { x1, y1, x2, y2 }
    }
    /// `{x, y, w, h}`.
    pub fn from_xywh(x: f64, y: f64, w: f64, h: f64) -> SelectionRect {
        SelectionRect { x1: x, y1: y, x2: x + w, y2: y + h }
    }
}

/// `normalizeRect(rect)`: corners in order, or None when not finite.
pub fn normalize_rect(r: SelectionRect) -> Option<SelectionRect> {
    if ![r.x1, r.y1, r.x2, r.y2].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(SelectionRect { x1: js::min(r.x1, r.x2), y1: js::min(r.y1, r.y2), x2: js::max(r.x1, r.x2), y2: js::max(r.y1, r.y2) })
}

fn contains(r: &SelectionRect, x: f64, y: f64) -> bool {
    x >= r.x1 && x <= r.x2 && y >= r.y1 && y <= r.y2
}

fn encloses(r: &SelectionRect, b: &Rect) -> bool {
    contains(r, b.x, b.y) && contains(r, b.x + b.w, b.y + b.h)
}

/// `selectableInRect(map, rect, {includeSecondary})`: what a rectangle
/// catches. With `include_secondary` false (the Cofactors toggle off)
/// secondary nodes are not on screen, so they are not caught either: a
/// rectangle followed by Delete must not remove what nobody could see.
/// Annotations are part of the drawing, so a rectangle dragged over a pathway
/// takes its labels with it.
pub fn selectable_in_rect(map: &EditableMap, rect: SelectionRect, include_secondary: bool) -> Selection {
    let mut out = Selection::default();
    let Some(b) = normalize_rect(rect) else { return out };
    for node in map.nodes().values() {
        if node.hidden == Some(true) || !matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium) {
            continue;
        }
        if !include_secondary && is_secondary_node(node) {
            continue;
        }
        if contains(&b, node.x, node.y) {
            out.nodes.insert(node.id.clone());
        }
    }
    for (id, rxn) in map.reactions() {
        if contains(&b, rxn.x, rxn.y) {
            out.reactions.insert(id.clone());
        }
    }
    for (id, t) in map.texts() {
        let c = text_centre(t);
        if contains(&b, c.x, c.y) {
            out.texts.insert(id.clone());
        }
    }
    // A compartment is caught only when the rectangle takes in all of it:
    // what lies inside is caught already, and the membrane around it should
    // come along rather than hold its contents back. The shared medium is the
    // whole drawing's frame and is never caught.
    for cell in map.cells() {
        for comp in &cell.compartments {
            if encloses(&b, &comp.rect) {
                out.compartments.insert((cell.id.clone(), comp.id.clone()));
            }
        }
    }
    out
}

/// What a selection operation moved.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Moved {
    pub nodes: Vec<String>,
    pub reactions: Vec<String>,
    pub texts: Vec<String>,
    /// Selected compartments moved whole, as `(cell id, compartment id)`.
    pub compartments: Vec<(String, String)>,
    /// How far the selection went: what was asked of `translate_selection`;
    /// for `drag_selection`, less than that when a wall held it back.
    pub dx: f64,
    pub dy: f64,
}

impl Moved {
    /// Whether anything moved at all.
    pub fn any(&self) -> bool {
        !(self.nodes.is_empty() && self.reactions.is_empty() && self.texts.is_empty() && self.compartments.is_empty())
    }
}

/// `translateSelection(map, selection, dx, dy)`.
///
/// Metabolite endpoints move first (moving b1), then reaction skeletons
/// (moving b2): when both ends of one branch are selected this translates the
/// whole curve. A straight reaction follows its two metabolites, and carries
/// the cofactor nodes it owns; with one of its ends in the selection it has
/// therefore moved already by the time its own turn comes, and so have those
/// nodes: moving either again would count the same drag twice.
pub fn translate_selection(map: &mut EditableMap, selection: &Selection, dx: f64, dy: f64) -> Option<Moved> {
    translate_selection_with(map, selection, dx, dy, false)
}

/// `translateSelection(map, selection, dx, dy, { rigid })`.
///
/// `rigid` is for a selection dragged as one ([`drag_selection`]), whose step
/// is already one every member may take: metabolites then move by exactly
/// that step, with no clamp of their own (which would shove one that starts
/// out of place sideways while the rest go straight), and a transporter goes
/// only as far as the step runs along its membrane: pushed into the membrane
/// instead, its projection back onto the outline would walk it round the
/// corners.
pub fn translate_selection_with(map: &mut EditableMap, selection: &Selection, dx: f64, dy: f64, rigid: bool) -> Option<Moved> {
    if !dx.is_finite() || !dy.is_finite() {
        return None;
    }
    let mut out = Moved { dx, dy, ..Default::default() };
    if dx == 0.0 && dy == 0.0 {
        return Some(out);
    }
    let selected: HashSet<&str> = selection.nodes.iter().map(String::as_str).collect();
    let carried = |map: &EditableMap, rid: &str| -> Option<(Option<String>, Option<String>)> {
        let l = map.reaction(rid)?.linear.as_ref()?;
        let hit = l.from.as_deref().is_some_and(|f| selected.contains(f)) || l.to.as_deref().is_some_and(|t| selected.contains(t));
        hit.then(|| (l.from.clone(), l.to.clone()))
    };

    for id in &selection.nodes {
        let Some(node) = map.node(id) else { continue };
        if !map.can_move_node(id) {
            continue;
        }
        if let Some(rid) = node.reaction_id.as_deref().filter(|r| !r.is_empty()) {
            if let Some((from, to)) = carried(map, rid) {
                if Some(id) != from.as_ref() && Some(id) != to.as_ref() {
                    continue;
                }
            }
        }
        if rigid {
            let Some(node) = map.g.nodes.get_mut(id) else { continue };
            node.x += dx;
            node.y += dy;
            update_connected_segments(map, id, Some((dx, dy)));
            out.nodes.push(id.clone());
            continue;
        }
        let before = node.pos();
        let moved = map.move_node(id, before.x + dx, before.y + dy).map(|n| n.pos());
        if let Some(p) = moved {
            if p.x != before.x || p.y != before.y {
                out.nodes.push(id.clone());
            }
        }
    }
    // A straight reaction goes where its two metabolites go (move_node does
    // this for each node it moves).
    if rigid && !out.nodes.is_empty() {
        let moved = out.nodes.clone();
        straighten_linear_reactions(map, Some(&moved));
    }

    for id in &selection.reactions {
        let Some(rxn) = map.reaction(id) else { continue };
        if !crate::reaction_edit::can_move_reaction(map, id) {
            continue;
        }
        if carried(map, id).is_some() {
            out.reactions.push(id.clone());
            continue;
        }
        let before = Point::new(rxn.x, rxn.y);
        let (mut tx, mut ty) = (before.x + dx, before.y + dy);
        if rigid && rxn.on_membrane && rxn.boundary_id.as_deref().is_some_and(|b| !b.is_empty()) {
            let n = rxn.normal.unwrap_or(Point::new(1.0, 0.0));
            let along = -dx * n.y + dy * n.x;
            if along.abs() < 1e-9 {
                continue;
            }
            (tx, ty) = (before.x - n.y * along, before.y + n.x * along);
        }
        if crate::reaction_edit::move_reaction(map, id, tx, ty) {
            let r = &map.g.reactions[id];
            if r.x != before.x || r.y != before.y {
                out.reactions.push(id.clone());
            }
        }
    }

    for id in &selection.texts {
        let Some(t) = map.text(id) else { continue };
        let (x, y) = (t.x + dx, t.y + dy);
        if map.move_text(id, x, y).is_some() {
            out.texts.push(id.clone());
        }
    }
    Some(out)
}

/// `dragSelection(map, selection, dx, dy)`: move a selection by `(dx, dy)`
/// the way the map's drag does.
///
/// `translate_selection` alone pulls a selection round an organelle apart:
/// the organelle stays where it is, so what is selected inside it stops at
/// its membrane while the rest goes on, and what is selected around it runs
/// into it. A cofactor node selected with its reaction also goes twice as
/// fast: it moves, and then its reaction carries it. Here:
///
/// - Selected compartments move first, whole, with everything that belongs to
///   them (the same move as dragging a compartment's centre handle). They are
///   then already where they are going when the rest of the selection
///   follows, so nothing in the selection collides with them.
/// - The selection goes only as far as all of it can: its compartments stay
///   inside their parents and off the compartments beside them that are not
///   moving, and its metabolites and reaction centres inside their
///   compartments and out of the organelles there. When one would hit a wall
///   the whole selection stops, or slides along it, together. (Moving each
///   member as far as it alone could go would pile the ones at the wall up
///   against it while the rest went on, and stretch the curves between them
///   across the map.)
/// - A cofactor node owned by a selected reaction is left to that reaction.
/// - Selected metabolites drawn inside a moving compartment without belonging
///   to it ride along exactly.
/// - Everything else moves as `translate_selection` moves it.
///
/// Returns what moved, plus the compartments and the step actually taken
/// (`dx`, `dy`), which is less than asked when a wall held the selection
/// back.
pub fn drag_selection(map: &mut EditableMap, selection: &Selection, dx: f64, dy: f64) -> Option<Moved> {
    if !dx.is_finite() || !dy.is_finite() {
        return None;
    }
    if dx == 0.0 && dy == 0.0 {
        return Some(Moved::default());
    }
    let plan = GroupMove::new(map, selection);
    let (dx, dy) = plan.allowed(map, dx, dy);
    if dx == 0.0 && dy == 0.0 {
        return Some(Moved::default());
    }

    let mut moved_compartments = Vec::new();
    for &(ci, ki) in &plan.compartments {
        let cell = &map.g.cells[ci];
        let comp = &cell.compartments[ki];
        let (cell_id, comp_id) = (cell.id.clone(), comp.id.clone());
        let cell_rect = comp.parent_id.is_none().then(|| Rect::new(cell.rect.x + dx, cell.rect.y + dy, cell.rect.w, cell.rect.h));
        crate::geometry_edit::translate_subtree(map, ci, &comp_id, dx, dy, cell_rect);
        moved_compartments.push((cell_id, comp_id));
    }
    if !moved_compartments.is_empty() {
        map.refresh_boundaries();
    }

    // Selected metabolites drawn inside a moving organelle without belonging
    // to it go exactly with it. They are already where their compartment says
    // they may not be, so moving them the ordinary way pulls them out of it.
    let mut riders = Vec::new();
    for id in &plan.riders {
        let Some(node) = map.g.nodes.get_mut(id) else { continue };
        let before = node.pos();
        node.x = before.x + dx;
        node.y = before.y + dy;
        let delta = (node.x - before.x, node.y - before.y);
        update_connected_segments(map, id, Some(delta));
        riders.push(id.clone());
    }
    if !riders.is_empty() {
        straighten_linear_reactions(map, Some(&riders));
    }

    let moved = translate_selection_with(map, &plan.rest, dx, dy, true).unwrap_or_default();
    let mut nodes = riders;
    nodes.extend(moved.nodes);
    let mut reactions = moved.reactions;
    reactions.extend(plan.carried_reactions);
    Some(Moved { nodes, reactions, texts: moved.texts, compartments: moved_compartments, dx, dy })
}

/// How a selection moves when dragged: the compartments that carry part of
/// it, and the rest, which moves member by member.
struct GroupMove {
    /// Selected compartments with no selected ancestor, as (cell, compartment)
    /// indices: each carries its own subtree.
    compartments: Vec<(usize, usize)>,
    /// Per cell id, every compartment those carry (their subtrees).
    carried: HashMap<String, HashSet<String>>,
    /// Selected reactions a compartment carries.
    carried_reactions: Vec<String>,
    /// Selected metabolites drawn inside a moving compartment without
    /// belonging to it: they ride along exactly.
    riders: Vec<String>,
    /// What is left for `translate_selection`.
    rest: Selection,
    /// Metabolites of `rest` that stand where they may now: the walls hold
    /// them back. One already out of place (left there by hand before) is not
    /// held to it, or it would pin the whole selection.
    held_nodes: Vec<String>,
    /// Reactions of `rest` whose centre stands where it may now.
    held_reactions: Vec<String>,
    /// Whole cells that move, so the holes they make are looked for where
    /// they will be.
    moving_cells: HashSet<String>,
    /// Every compartment that moves, as `(cell id, compartment id)`.
    moving_keys: HashSet<(String, String)>,
}

impl GroupMove {
    fn new(map: &EditableMap, selection: &Selection) -> GroupMove {
        let g = &map.g;
        let mut compartments = Vec::new();
        let mut carried: HashMap<String, HashSet<String>> = HashMap::new();
        for (cell_id, comp_id) in &selection.compartments {
            let Some(ci) = g.cells.iter().position(|c| &c.id == cell_id) else { continue };
            let cell = &g.cells[ci];
            let Some(ki) = cell.compartments.iter().position(|c| &c.id == comp_id) else { continue };
            // Nested in another selected compartment: that one carries it.
            let mut parent = cell.compartments[ki].parent_id.clone();
            let mut nested = false;
            for _ in 0..cell.compartments.len() {
                let Some(pid) = parent else { break };
                if selection.compartments.contains(&(cell_id.clone(), pid.clone())) {
                    nested = true;
                    break;
                }
                parent = cell.compartments.iter().find(|c| c.id == pid).and_then(|c| c.parent_id.clone());
            }
            if nested {
                continue;
            }
            compartments.push((ci, ki));
            carried.entry(cell_id.clone()).or_default().extend(crate::geometry_edit::descendant_ids(cell, comp_id));
        }

        let reaction_carried = |rid: &str| {
            g.reactions.get(rid).is_some_and(|r| {
                let cell = r.cell_id.as_deref().unwrap_or("");
                carried.get(cell).is_some_and(|sub| crate::geometry_edit::reaction_in_subtree(r, cell, sub))
            })
        };
        // Straight reactions that follow a selected end are handled by
        // `translate_selection` itself, and so are the nodes that are some
        // straight reaction's end.
        let selected: HashSet<&str> = selection.nodes.iter().map(String::as_str).collect();
        let follows_end = |rid: &str| {
            g.reactions.get(rid).and_then(|r| r.linear.as_ref()).is_some_and(|l| {
                l.from.as_deref().is_some_and(|f| selected.contains(f)) || l.to.as_deref().is_some_and(|t| selected.contains(t))
            })
        };
        let line_ends: HashSet<&str> = g
            .reactions
            .values()
            .filter_map(|r| r.linear.as_ref())
            .flat_map(|l| [l.from.as_deref(), l.to.as_deref()])
            .flatten()
            .collect();

        // Inside the outline of a compartment that moves, whatever it is drawn
        // as belonging to: a cytosolic metabolite of an inner-membrane
        // transporter drawn in the intermembrane space, say.
        let moving: Vec<Rect> = compartments.iter().map(|&(ci, ki)| g.cells[ci].compartments[ki].rect).collect();
        let inside_moving = |p: Point| moving.iter().any(|r| p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h);

        let mut rest = Selection { texts: selection.texts.clone(), ..Default::default() };
        let mut riders = Vec::new();
        for id in &selection.nodes {
            let Some(node) = g.nodes.get(id) else { continue };
            let in_carried = node
                .cell_id
                .as_ref()
                .and_then(|c| carried.get(c))
                .is_some_and(|sub| node.compartment.as_ref().is_some_and(|k| sub.contains(k)));
            if in_carried {
                continue;
            }
            if let Some(rid) = node.reaction_id.as_deref().filter(|r| !r.is_empty()) {
                if reaction_carried(rid) {
                    continue;
                }
                // Its reaction is selected and carries it.
                let left_to_reaction = selection.reactions.contains(rid)
                    && g.reactions.contains_key(rid)
                    && !follows_end(rid)
                    && !line_ends.contains(id.as_str());
                if left_to_reaction {
                    continue;
                }
            }
            if inside_moving(node.pos()) && is_draggable_node(Some(node)) && map.can_move_node(id) && !line_ends.contains(id.as_str()) {
                riders.push(id.clone());
                continue;
            }
            rest.nodes.insert(id.clone());
        }
        let mut carried_reactions = Vec::new();
        for id in &selection.reactions {
            if reaction_carried(id) {
                carried_reactions.push(id.clone());
            } else {
                rest.reactions.insert(id.clone());
            }
        }

        let held_nodes = rest
            .nodes
            .iter()
            .filter(|id| {
                g.nodes.get(*id).is_some_and(|n| {
                    is_draggable_node(Some(n)) && map.can_move_node(id) && node_fits_at(map, Placeable::from(n), n.x, n.y, None)
                })
            })
            .cloned()
            .collect();
        let held_reactions = rest
            .reactions
            .iter()
            .filter(|id| g.reactions.get(*id).is_some_and(|r| reaction_centre_fits(map, id, r.x, r.y, None)))
            .cloned()
            .collect();
        let moving_cells = compartments
            .iter()
            .filter(|&&(ci, ki)| g.cells[ci].compartments[ki].parent_id.is_none())
            .map(|&(ci, _)| g.cells[ci].id.clone())
            .collect();
        let moving_keys = carried.iter().flat_map(|(cell, ids)| ids.iter().map(move |id| (cell.clone(), id.clone()))).collect();
        GroupMove { compartments, carried, carried_reactions, riders, rest, held_nodes, held_reactions, moving_cells, moving_keys }
    }

    /// Whether every held metabolite and reaction centre still stands where
    /// it may after moving by `(dx, dy)`, with what moves along moved too.
    fn members_fit(&self, map: &EditableMap, dx: f64, dy: f64) -> bool {
        let moving = Moving { dx, dy, cells: &self.moving_cells, compartments: &self.moving_keys };
        self.held_nodes.iter().all(|id| {
            map.g.nodes.get(id).is_none_or(|n| node_fits_at(map, Placeable::from(n), n.x + dx, n.y + dy, Some(&moving)))
        }) && self.held_reactions.iter().all(|id| {
            map.g.reactions.get(id).is_none_or(|r| reaction_centre_fits(map, id, r.x + dx, r.y + dy, Some(&moving)))
        })
    }

    /// Whether the whole selection may move by `(dx, dy)`.
    fn fits(&self, map: &EditableMap, dx: f64, dy: f64) -> bool {
        self.allows(map, dx, dy) && self.members_fit(map, dx, dy)
    }

    fn carries(&self, cell_id: &str, compartment_id: &str) -> bool {
        self.carried.get(cell_id).is_some_and(|s| s.contains(compartment_id))
    }

    /// Whether every carried compartment still fits after moving by
    /// `(dx, dy)`: inside its parent, with the margin `edit_geometry_rect`
    /// keeps, and off whatever shares that parent and stays put. One that does
    /// not fit now is not held to it.
    fn allows(&self, map: &EditableMap, dx: f64, dy: f64) -> bool {
        self.compartments.iter().all(|&(ci, ki)| {
            let cell = &map.g.cells[ci];
            let comp = &cell.compartments[ki];
            let now = comp.rect;
            let next = Rect::new(now.x + dx, now.y + dy, now.w, now.h);
            let (parent, pad) = match &comp.parent_id {
                Some(pid) => match cell.compartments.iter().find(|c| &c.id == pid) {
                    Some(p) => (p.rect, 10.0),
                    None => return true,
                },
                None => (map.g.environment.rect, 18.0),
            };
            if within(&now, &parent, pad) && !within(&next, &parent, pad) {
                return false;
            }
            let siblings: Vec<Rect> = match &comp.parent_id {
                Some(pid) => cell
                    .compartments
                    .iter()
                    .filter(|c| c.parent_id.as_ref() == Some(pid) && !self.carries(&cell.id, &c.id))
                    .map(|c| c.rect)
                    .collect(),
                // A whole cell: the other cells are beside it.
                None => map
                    .g
                    .cells
                    .iter()
                    .filter(|c| c.id != cell.id && !c.compartments.iter().any(|k| k.parent_id.is_none() && self.carries(&c.id, &k.id)))
                    .map(|c| c.rect)
                    .collect(),
            };
            siblings.iter().all(|s| !overlaps(&next, s) || overlaps(&now, s))
        })
    }

    /// The displacement the selection allows towards `(dx, dy)`: all of it,
    /// or else as far as all of it can go along x, then along y, which lets
    /// the selection slide along a wall rather than stick to it.
    fn allowed(&self, map: &EditableMap, dx: f64, dy: f64) -> (f64, f64) {
        if self.fits(map, dx, dy) {
            return (dx, dy);
        }
        let furthest = |ok: &dyn Fn(f64) -> bool| -> f64 {
            if ok(1.0) {
                return 1.0;
            }
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..30 {
                let mid = (lo + hi) / 2.0;
                if ok(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let fx = if dx != 0.0 { furthest(&|t| self.fits(map, dx * t, 0.0)) } else { 0.0 };
        let x = dx * fx;
        let fy = if dy != 0.0 { furthest(&|t| self.fits(map, x, dy * t)) } else { 0.0 };
        (x, dy * fy)
    }
}

const FIT_EPS: f64 = 1e-6;

/// `r` lies inside `parent` with `pad` to spare on every side.
fn within(r: &Rect, parent: &Rect, pad: f64) -> bool {
    r.x >= parent.x + pad - FIT_EPS
        && r.y >= parent.y + pad - FIT_EPS
        && r.x + r.w <= parent.x + parent.w - pad + FIT_EPS
        && r.y + r.h <= parent.y + parent.h - pad + FIT_EPS
}

/// The two rectangles share some area (touching edges do not count).
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w - FIT_EPS && b.x < a.x + a.w - FIT_EPS && a.y < b.y + b.h - FIT_EPS && b.y < a.y + a.h - FIT_EPS
}

/// One selected object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    Node,
    Reaction,
    Text,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemKind::Node => "node",
            ItemKind::Reaction => "reaction",
            ItemKind::Text => "text",
        }
    }
}

/// `selectedObjects(map, selection)`: what alignment acts on, namely visible
/// movable nodes, movable reactions and texts.
pub fn selected_objects(map: &EditableMap, selection: &Selection) -> Vec<(ItemKind, String)> {
    let mut out = Vec::new();
    for id in &selection.nodes {
        let Some(node) = map.node(id) else { continue };
        if node.hidden == Some(true) || !map.can_move_node(id) {
            continue;
        }
        out.push((ItemKind::Node, id.clone()));
    }
    for id in &selection.reactions {
        if map.reaction(id).is_none() || !crate::reaction_edit::can_move_reaction(map, id) {
            continue;
        }
        out.push((ItemKind::Reaction, id.clone()));
    }
    for id in &selection.texts {
        if map.text(id).is_some() {
            out.push((ItemKind::Text, id.clone()));
        }
    }
    out
}

/// Where an item's middle is, whatever corner it happens to be anchored by.
fn item_centre(map: &EditableMap, kind: &ItemKind, id: &str) -> Point {
    match kind {
        ItemKind::Text => text_centre(&map.g.texts[id]),
        ItemKind::Node => map.g.nodes[id].pos(),
        ItemKind::Reaction => {
            let r = &map.g.reactions[id];
            Point::new(r.x, r.y)
        }
    }
}

fn object_box(map: &EditableMap, kind: &ItemKind, id: &str) -> TextBox {
    match kind {
        ItemKind::Text => text_box_of(&map.g.texts[id]),
        ItemKind::Node => {
            let n = &map.g.nodes[id];
            let r = n.r;
            TextBox { left: n.x - r, right: n.x + r, top: n.y - r, bottom: n.y + r }
        }
        ItemKind::Reaction => {
            // Internal reaction markers are 18×6. Transporters are 26×22 or
            // taller for multi-lane transport. A conservative axis-aligned
            // footprint keeps alignment predictable regardless of rotation.
            let o = &map.g.reactions[id];
            let half_w = if o.on_membrane { 13.0 } else { 9.0 };
            let lanes = o.transport_lanes.as_ref().map(|l| l.len()).unwrap_or(0) as f64;
            let half_h = if o.on_membrane { js::max(11.0, 7.0 + lanes * 5.0) } else { 3.0 };
            TextBox { left: o.x - half_w, right: o.x + half_w, top: o.y - half_h, bottom: o.y + half_h }
        }
    }
}

/// Place an item so that its middle lands on `(x, y)`.
fn move_item(map: &mut EditableMap, kind: &ItemKind, id: &str, x: f64, y: f64) -> bool {
    match kind {
        ItemKind::Text => {
            // A text is positioned by the start of its baseline, so the
            // offset from its middle has to be put back before it is moved.
            let t = &map.g.texts[id];
            let c = text_centre(t);
            let (nx, ny) = (t.x + (x - c.x), t.y + (y - c.y));
            map.move_text(id, nx, ny).is_some()
        }
        ItemKind::Node => map.move_node(id, x, y).is_some(),
        ItemKind::Reaction => crate::reaction_edit::move_reaction(map, id, x, y),
    }
}

/// What alignment or distribution placed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arranged {
    pub moved: Vec<(ItemKind, String)>,
    pub count: usize,
    pub step: Option<f64>,
}

fn average(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |s, x| s + x) / values.len() as f64
}

/// `alignSelection(map, selection, mode)`: `horizontal` (same y centre),
/// `vertical` (same x centre), or `left`/`right`/`top`/`bottom` edges.
pub fn align_selection(map: &mut EditableMap, selection: &Selection, mode: &str) -> Arranged {
    let objects = selected_objects(map, selection);
    let count = objects.len();
    if count < 2 {
        return Arranged { moved: vec![], count, step: None };
    }
    let boxes: Vec<(ItemKind, String, TextBox, Point)> =
        objects.into_iter().map(|(k, id)| (k.clone(), id.clone(), object_box(map, &k, &id), item_centre(map, &k, &id))).collect();
    let target = match mode {
        "horizontal" => average(&boxes.iter().map(|b| b.3.y).collect::<Vec<_>>()),
        "vertical" => average(&boxes.iter().map(|b| b.3.x).collect::<Vec<_>>()),
        "left" => js::min_n(boxes.iter().map(|b| b.2.left)),
        "right" => js::max_n(boxes.iter().map(|b| b.2.right)),
        "top" => js::min_n(boxes.iter().map(|b| b.2.top)),
        "bottom" => js::max_n(boxes.iter().map(|b| b.2.bottom)),
        _ => return Arranged { moved: vec![], count, step: None },
    };
    let mut moved = Vec::new();
    for (kind, id, b, centre) in boxes {
        let mut x = centre.x;
        let mut y = centre.y;
        match mode {
            "horizontal" => y = target,
            "vertical" => x = target,
            "left" => x += target - b.left,
            "right" => x += target - b.right,
            "top" => y += target - b.top,
            "bottom" => y += target - b.bottom,
            _ => {}
        }
        if move_item(map, &kind, &id, x, y) {
            moved.push((kind, id));
        }
    }
    Arranged { moved, count, step: None }
}

/// `distributeSelection(map, selection, axis)`: evenly distribute selected
/// object centres between the first and last item along `horizontal` or
/// `vertical`. End points stay fixed. Requires at least three objects.
pub fn distribute_selection(map: &mut EditableMap, selection: &Selection, axis: &str) -> Arranged {
    let objects = selected_objects(map, selection);
    let count = objects.len();
    if count < 3 || !matches!(axis, "horizontal" | "vertical") {
        return Arranged { moved: vec![], count, step: None };
    }
    let horizontal = axis == "horizontal";
    let key = |p: &Point| if horizontal { p.x } else { p.y };
    let mut sorted: Vec<(ItemKind, String, Point)> =
        objects.into_iter().map(|(k, id)| { let c = item_centre(map, &k, &id); (k, id, c) }).collect();
    sorted.sort_by(|a, b| {
        let d = key(&a.2) - key(&b.2);
        if d != 0.0 && !d.is_nan() {
            return if d < 0.0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        js::locale_compare(&a.1, &b.1)
    });
    let first = key(&sorted[0].2);
    let last = key(&sorted[sorted.len() - 1].2);
    let step = (last - first) / (sorted.len() as f64 - 1.0);
    let mut moved = Vec::new();
    for (i, (kind, id, centre)) in sorted.iter().enumerate().take(sorted.len() - 1).skip(1) {
        let x = if horizontal { first + step * i as f64 } else { centre.x };
        let y = if !horizontal { first + step * i as f64 } else { centre.y };
        if move_item(map, kind, id, x, y) {
            moved.push((kind.clone(), id.clone()));
        }
    }
    Arranged { moved, count, step: Some(step) }
}

// Even spacing and turning

/// `withReactionMetabolites(map, selection)`: a selection with the main
/// metabolites of its reactions added, which is what even spacing and turning
/// act on. A pathway is picked by its reactions, and a rectangle drawn round
/// them easily misses the metabolites at either end: left behind, a straight
/// reaction would bend back to them and the pathway would zigzag. Align does
/// the same with the metabolites a reaction's line runs through. Cofactors are
/// not added (their reactions carry them), nor are the metabolites of
/// transporters, which stay on their membranes.
pub fn with_reaction_metabolites(map: &EditableMap, selection: &Selection) -> Selection {
    let mut out = selection.clone();
    for seg in map.segments() {
        let Some(node_id) = seg.node_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        if !selection.reactions.contains(&seg.reaction_id) || map.reaction(&seg.reaction_id).is_some_and(|r| r.on_membrane) {
            continue;
        }
        let Some(node) = map.node(node_id) else { continue };
        if matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium) && !is_secondary_node(node) {
            out.nodes.insert(node.id.clone());
        }
    }
    out
}

/// `spacingItems(map, selection)`: what a selection is spaced and lined up
/// by: its metabolites that stand on their own, those of its reactions
/// included ([`with_reaction_metabolites`]), and its reaction markers.
///
/// Secondary metabolites are left out, since their reactions place them, and
/// so is a straight reaction with one of these metabolites at an end: it goes
/// where its two metabolites go, and placing it as well would count it twice.
pub fn spacing_items(map: &EditableMap, selection: &Selection) -> Vec<(ItemKind, String)> {
    let mut out = Vec::new();
    for id in &with_reaction_metabolites(map, selection).nodes {
        let Some(node) = map.node(id) else { continue };
        if node.hidden == Some(true)
            || !matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium)
            || is_secondary_node(node)
            || !map.can_move_node(id)
        {
            continue;
        }
        out.push((ItemKind::Node, id.clone()));
    }
    let placed: HashSet<String> = out.iter().map(|(_, id)| id.clone()).collect();
    for id in &selection.reactions {
        if map.reaction(id).is_none() || !crate::reaction_edit::can_move_reaction(map, id) || follows_end(map, id, &placed) {
            continue;
        }
        out.push((ItemKind::Reaction, id.clone()));
    }
    out
}

/// `selectionMainAxis(map, selection)`: the line a selection lies along, as
/// a unit vector: the principal axis of where its spacing items stand, so a
/// pathway drawn on a slant is taken along its slant. None with fewer than two
/// items. When they all stand on one spot there is no line, and x is taken.
pub fn selection_main_axis(map: &EditableMap, selection: &Selection) -> Option<Point> {
    let items = spacing_items(map, selection);
    if items.len() < 2 {
        return None;
    }
    Some(principal_axis(&items.iter().map(|(kind, id)| item_centre(map, kind, id)).collect::<Vec<_>>()))
}

/// What even spacing placed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpacedEvenly {
    pub moved: Vec<(ItemKind, String)>,
    pub count: usize,
    /// The line the items were spaced along, and the step between them.
    pub axis: Option<Point>,
    pub step: Option<f64>,
}

/// `spaceSelectionEvenly(map, selection)`: space a selection evenly along its
/// own line, in whatever direction it runs: the two items furthest apart
/// along it stay, the others are set at equal steps between them, each
/// keeping how far off the line it stands. The items are moved as align and
/// distribute move them, so a reaction brings its cofactors, a straight
/// reaction stays on its two metabolites, and a metabolite stays inside its
/// compartment. Needs three items ([`spacing_items`]).
pub fn space_selection_evenly(map: &mut EditableMap, selection: &Selection) -> SpacedEvenly {
    let items = spacing_items(map, selection);
    let count = items.len();
    if count < 3 {
        return SpacedEvenly { count, ..Default::default() };
    }
    let centres: Vec<Point> = items.iter().map(|(kind, id)| item_centre(map, kind, id)).collect();
    let axis = principal_axis(&centres);
    let along = |p: Point| p.x * axis.x + p.y * axis.y;
    let mut placed: Vec<(ItemKind, String, Point, f64)> =
        items.into_iter().zip(centres).map(|((kind, id), centre)| (kind, id, centre, along(centre))).collect();
    placed.sort_by(|a, b| {
        let d = a.3 - b.3;
        if d != 0.0 && !d.is_nan() {
            return if d < 0.0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        js::locale_compare(&format!("{}:{}", a.0.as_str(), a.1), &format!("{}:{}", b.0.as_str(), b.1))
    });
    let first = placed[0].3;
    let step = (placed[placed.len() - 1].3 - first) / (placed.len() as f64 - 1.0);

    // Moving one item never moves another (a metabolite takes only the
    // straight reactions on it, a reaction only its own cofactors, and neither
    // is an item), so every target can be taken from where things stand now.
    let mut moved = Vec::new();
    for (i, (kind, id, centre, at)) in placed.iter().enumerate().take(placed.len() - 1).skip(1) {
        let shift = first + step * i as f64 - at;
        if move_item(map, kind, id, centre.x + axis.x * shift, centre.y + axis.y * shift) {
            moved.push((kind.clone(), id.clone()));
        }
    }
    SpacedEvenly { moved, count, axis: Some(axis), step: Some(step) }
}

/// `TURN_REFUSAL`: why a selection could not be turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRefusal {
    /// Transporters stay on their membranes.
    Transporter,
    /// Compartments stay upright.
    Compartment,
    /// Something would be turned out of where it may be.
    Room,
}

impl TurnRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnRefusal::Transporter => "transporter",
            TurnRefusal::Compartment => "compartment",
            TurnRefusal::Room => "room",
        }
    }
}

/// What turning a selection did.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionTurn {
    Turned { angle: f64, pivot: Point, nodes: Vec<String>, reactions: Vec<String>, texts: Vec<String> },
    Refused(TurnRefusal),
}

/// `rotateSelection(map, selection, angle, pivot)`: turn a selection as one
/// rigid piece by `angle` about `pivot` (by default the middle of the box
/// round its selected objects), so that a pathway can be laid on a diagonal
/// and come out the same drawing, its cofactors and curves where they were
/// relative to it.
///
/// - Selected metabolites turn about the pivot, their labels with them, and
///   so do the main metabolites of selected reactions
///   ([`with_reaction_metabolites`]). The branches of the reactions that turn
///   with them keep their shape; the branches of the others are dragged
///   along, as when the metabolite is dragged.
/// - Selected reactions turn their whole drawing ([`turn_reaction_about`]),
///   their cofactors and names included.
/// - A straight reaction with a selected metabolite at an end goes where its
///   two metabolites go. With both of them selected that is the same turn.
/// - Texts turn about the pivot by their anchor, and stay level.
///
/// Transporters stay on their membranes and compartments stay upright, so a
/// selection holding either is refused, as is one that would turn a
/// metabolite or a reaction centre out of where it may be (one already out of
/// place is not held to it). A refusal is decided before anything moves.
///
/// `angle` is in radians; positive is clockwise on screen.
pub fn rotate_selection(map: &mut EditableMap, picked: &Selection, angle: f64, pivot: Option<Point>) -> Option<SelectionTurn> {
    if !angle.is_finite() {
        return None;
    }
    if !picked.compartments.is_empty() {
        return Some(SelectionTurn::Refused(TurnRefusal::Compartment));
    }
    if picked.reactions.iter().any(|id| map.reaction(id).is_some_and(|r| r.on_membrane)) {
        return Some(SelectionTurn::Refused(TurnRefusal::Transporter));
    }
    let selection = &with_reaction_metabolites(map, picked);
    let centre = match pivot {
        Some(p) => p,
        None => box_centre(&selected_objects(map, selection).iter().map(|(kind, id)| item_centre(map, kind, id)).collect::<Vec<_>>())?,
    };
    if !centre.x.is_finite() || !centre.y.is_finite() {
        return None;
    }
    if angle.abs() < 1e-9 {
        return Some(SelectionTurn::Turned { angle: 0.0, pivot: centre, nodes: vec![], reactions: vec![], texts: vec![] });
    }

    let plan = plan_turn(map, selection);
    let cos = js::cos(angle);
    let sin = js::sin(angle);
    let turn = |p: Point| {
        let dx = p.x - centre.x;
        let dy = p.y - centre.y;
        Point::new(centre.x + dx * cos - dy * sin, centre.y + dx * sin + dy * cos)
    };

    // Everything that lands where the turn puts it, checked before any of it moves.
    for id in &plan.landing {
        let node = &map.g.nodes[id];
        let to = turn(node.pos());
        if node_fits_at(map, Placeable::from(node), node.x, node.y, None) && !node_fits_at(map, Placeable::from(node), to.x, to.y, None) {
            return Some(SelectionTurn::Refused(TurnRefusal::Room));
        }
    }
    for id in &plan.reactions {
        let rxn = &map.g.reactions[id];
        let to = turn(Point::new(rxn.x, rxn.y));
        if reaction_centre_fits(map, id, rxn.x, rxn.y, None) && !reaction_centre_fits(map, id, to.x, to.y, None) {
            return Some(SelectionTurn::Refused(TurnRefusal::Room));
        }
    }

    // Every curve that turns whole needs its handles first: made after the
    // turn, from ends turned and ends not yet turned, they would bend it.
    for id in &plan.rigid {
        ensure_reaction_backbone(map, id);
    }
    for id in &plan.nodes {
        // The branches of a reaction that turns too keep their shape: their
        // metabolite-side handle turns. Any other branch is dragged along.
        let handles: Vec<(usize, Point)> = map
            .g
            .segments
            .iter()
            .enumerate()
            .filter(|(_, seg)| seg.node_id.as_deref() == Some(id.as_str()) && plan.rigid.contains(&seg.reaction_id))
            .filter_map(|(i, seg)| seg.b1.map(|b1| (i, turn(b1))))
            .collect();
        let node = map.g.nodes.get_mut(id).expect("planned");
        let to = turn(node.pos());
        let delta = (to.x - node.x, to.y - node.y);
        node.x = to.x;
        node.y = to.y;
        update_connected_segments(map, id, Some(delta));
        for (i, b1) in handles {
            map.g.segments[i].b1 = Some(b1);
        }
        turn_metabolite_label(map.g.nodes.get_mut(id).expect("planned"), angle);
    }
    for id in &plan.reactions {
        turn_reaction_about(map, id, centre, angle);
    }
    straighten_linear_reactions(map, Some(&plan.landing));
    // A straight reaction with both ends turned has turned as a whole: its
    // name and its cofactors' labels turn with it.
    for id in &plan.straight {
        let rxn = &mut map.g.reactions[id];
        if let Some(offset) = reaction_label_offset(rxn) {
            set_reaction_label_offset(rxn, Some(Point::new(offset.x * cos - offset.y * sin, offset.x * sin + offset.y * cos)));
        }
        let (from, to) = rxn.linear.as_ref().map(|l| (l.from.clone(), l.to.clone())).unwrap_or_default();
        for node in map.g.nodes.values_mut() {
            if node.reaction_id.as_deref() == Some(id.as_str()) && Some(&node.id) != from.as_ref() && Some(&node.id) != to.as_ref() {
                turn_metabolite_label(node, angle);
            }
        }
    }

    let mut texts = Vec::new();
    for id in &selection.texts {
        let Some(t) = map.text(id) else { continue };
        let to = turn(Point::new(t.x, t.y));
        if map.move_text(id, to.x, to.y).is_some() {
            texts.push(id.clone());
        }
    }
    Some(SelectionTurn::Turned {
        angle,
        pivot: centre,
        nodes: plan.nodes,
        reactions: plan.reactions.into_iter().chain(plan.straight).collect(),
        texts,
    })
}

/// `pointingAngle(map, selection, direction)`: how far to turn a selection so
/// that its main line points `direction` (`N`, `NE`, … `NW`; y grows down the
/// page), the way its reactions run: the sum of their backbones along the
/// line says which end is the start. With no reactions, or as many running
/// each way, the line is turned whichever way round is the smaller turn. None
/// with fewer than two items to take a line from.
pub fn pointing_angle(map: &EditableMap, selection: &Selection, direction: &str) -> Option<f64> {
    let way = crate::reaction_edit::compass(direction)?;
    let axis = selection_main_axis(map, selection)?;
    let directions: Vec<Point> = selection.reactions.iter().filter_map(|id| map.reaction(id)?.axis.map(|a| a.dir)).collect();
    let flow = directions.iter().fold(0.0, |sum, dir| sum + dir.x * axis.x + dir.y * axis.y);
    let angle_from = |u: Point| js::atan2(u.x * way.y - u.y * way.x, u.x * way.x + u.y * way.y);
    let back = Point::new(-axis.x, -axis.y);
    if !directions.is_empty() && flow.abs() >= FLOW_CLEAR * directions.len() as f64 {
        return Some(angle_from(if flow > 0.0 { axis } else { back }));
    }
    let forward = angle_from(axis);
    let backward = angle_from(back);
    Some(if backward.abs() < forward.abs() { backward } else { forward })
}

/// `pointSelection(map, selection, direction)`: turn a selection so that its
/// main line points `direction` ([`pointing_angle`]).
pub fn point_selection(map: &mut EditableMap, selection: &Selection, direction: &str) -> Option<SelectionTurn> {
    let angle = pointing_angle(map, selection, direction)?;
    rotate_selection(map, selection, angle, None)
}

/// How much of the reactions' backbones, on average, must run one way along
/// the line before that way is taken as the way the pathway runs.
const FLOW_CLEAR: f64 = 0.25;

/// `planTurn`: who moves what when a selection turns.
///
/// A cofactor is carried by its reaction when that reaction turns or follows
/// its ends, so moving it as well would turn it twice.
struct TurnPlan {
    /// Selected metabolites the turn moves itself.
    nodes: Vec<String>,
    /// Selected reactions that turn whole (not transporters, not straight
    /// ones, which follow their ends).
    reactions: Vec<String>,
    /// Straight reactions with both ends among `nodes`: they turn whole too,
    /// by being straightened.
    straight: Vec<String>,
    /// Ids of the two kinds above, whose branches keep their shape.
    rigid: HashSet<String>,
    /// Every metabolite the turn sets down exactly where it turns it to,
    /// cofactors included, for the room check.
    landing: Vec<String>,
}

fn plan_turn(map: &EditableMap, selection: &Selection) -> TurnPlan {
    let selected: Vec<String> = selection.nodes.iter().filter(|id| map.node(id).is_some() && map.can_move_node(id)).cloned().collect();
    let ids: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let reactions: Vec<String> = selection
        .reactions
        .iter()
        .filter(|id| map.reaction(id).is_some_and(|r| !r.on_membrane) && !is_straight(map, id))
        .cloned()
        .collect();
    let turning: HashSet<&str> = reactions.iter().map(String::as_str).collect();
    let carried = |id: &str| -> bool {
        let Some(owner) = map.g.nodes[id].reaction_id.as_deref().filter(|r| !r.is_empty()).and_then(|r| map.reaction(r)) else {
            return false;
        };
        if turning.contains(owner.id.as_str()) {
            return true;
        }
        let Some((from, to)) = straight_ends(map, &owner.id) else { return false };
        if id == from || id == to {
            return false;
        }
        // A straight reaction keeps its cofactors with it, wherever it goes.
        selection.reactions.contains(&owner.id) || ids.contains(from.as_str()) || ids.contains(to.as_str())
    };
    let nodes: Vec<String> = selected.iter().filter(|id| !carried(id)).cloned().collect();
    let moving: HashSet<&str> = nodes.iter().map(String::as_str).collect();
    let straight: Vec<String> = map
        .g
        .reactions
        .keys()
        .filter(|id| straight_ends(map, id).is_some_and(|(from, to)| moving.contains(from.as_str()) && moving.contains(to.as_str())))
        .cloned()
        .collect();
    let rigid: HashSet<String> = reactions.iter().chain(&straight).cloned().collect();
    let mut landing = nodes.clone();
    for node in map.g.nodes.values() {
        if node.reaction_id.as_deref().is_some_and(|r| !r.is_empty() && rigid.contains(r)) && !moving.contains(node.id.as_str()) {
            landing.push(node.id.clone());
        }
    }
    TurnPlan { nodes, reactions, straight, rigid, landing }
}

/// `isStraight`: a reaction kept straight between two metabolites that are
/// both on the map. Its two ends, when it is one.
fn straight_ends(map: &EditableMap, reaction_id: &str) -> Option<(String, String)> {
    let rxn = map.reaction(reaction_id)?;
    let linear = rxn.linear.as_ref()?;
    if rxn.axis.is_none() || rxn.on_membrane {
        return None;
    }
    let from = linear.from.as_deref().filter(|id| map.has_node(id))?;
    let to = linear.to.as_deref().filter(|id| map.has_node(id))?;
    Some((from.to_string(), to.to_string()))
}

fn is_straight(map: &EditableMap, reaction_id: &str) -> bool {
    straight_ends(map, reaction_id).is_some()
}

/// `followsEnd`: a straight reaction with one of `node_ids` at an end: it goes
/// where they go.
fn follows_end(map: &EditableMap, reaction_id: &str, node_ids: &HashSet<String>) -> bool {
    straight_ends(map, reaction_id).is_some_and(|(from, to)| node_ids.contains(&from) || node_ids.contains(&to))
}

/// `principalAxis`: the principal axis of some points, x when they give none.
fn principal_axis(points: &[Point]) -> Point {
    let mean = Point::new(average(&points.iter().map(|p| p.x).collect::<Vec<_>>()), average(&points.iter().map(|p| p.y).collect::<Vec<_>>()));
    let mut sxx = 0.0;
    let mut syy = 0.0;
    let mut sxy = 0.0;
    for p in points {
        let dx = p.x - mean.x;
        let dy = p.y - mean.y;
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    if sxx + syy < 1e-9 {
        return Point::new(1.0, 0.0);
    }
    let theta = js::atan2(2.0 * sxy, sxx - syy) / 2.0;
    // Snap the components that should be nought, so a line already level or
    // upright is read as exactly that.
    let snap = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
    Point::new(snap(js::cos(theta)), snap(js::sin(theta)))
}

/// `boxCentre`: the middle of the box round some points, or None for none.
fn box_centre(points: &[Point]) -> Option<Point> {
    if points.is_empty() {
        return None;
    }
    let xs = || points.iter().map(|p| p.x);
    let ys = || points.iter().map(|p| p.y);
    Some(Point::new((js::min_n(xs()) + js::max_n(xs())) / 2.0, (js::min_n(ys()) + js::max_n(ys())) / 2.0))
}

/// What Align vertical / horizontal for a selection with reactions did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LineAlignment {
    pub nodes: Vec<String>,
    pub reactions: Vec<String>,
    pub texts: Vec<String>,
    /// Metabolites a membrane stopped short of the line.
    pub held: Vec<String>,
}

/// `alignSelectionInLine(selection, mode)`: Escher's Align vertical / Align
/// horizontal for a selection with reactions in it. The reactions come with
/// their metabolites, and everything ends on one line.
///
/// Each selected reaction brings the two metabolites its line runs through.
/// Those, the other selected primary metabolites and the selected reaction
/// markers are set on one vertical (same x) or horizontal (same y) line, at
/// the mean of where the metabolites and markers were, each keeping its place
/// along it. Every selected reaction is then kept straight between its two
/// metabolites, and its cofactors put back in order around the new line.
/// Transporters stay on their membranes; a metabolite a membrane stops is
/// reported in `held`. Selected texts are centred on the line.
pub fn align_selection_in_line(map: &mut EditableMap, selection: &Selection, mode: &str) -> Option<LineAlignment> {
    if mode != "horizontal" && mode != "vertical" {
        return None;
    }
    let vertical = mode == "vertical";
    let reaction_ids: Vec<String> = selection
        .reactions
        .iter()
        .filter(|id| map.reaction(id).is_some_and(|r| !r.on_membrane))
        .cloned()
        .collect();
    let primary = |map: &EditableMap, id: &str| map.node(id).is_some_and(|n| !is_secondary_node(n));
    let mut node_ids: IndexSet<String> = selection.nodes.iter().filter(|id| primary(map, id)).cloned().collect();
    let mut ends: IndexMap<String, (String, String)> = IndexMap::new();
    for id in &reaction_ids {
        let (from, to) = reaction_line_ends(map, id);
        for end in [&from, &to].into_iter().flatten() {
            if primary(map, end) {
                node_ids.insert(end.clone());
            }
        }
        if let (Some(f), Some(t)) = (&from, &to) {
            if f != t && primary(map, f) && primary(map, t) {
                ends.insert(id.clone(), (f.clone(), t.clone()));
            }
        }
    }
    let text_ids: Vec<String> = selection.texts.iter().filter(|id| map.text(id).is_some()).cloned().collect();
    // The line runs through the drawing; a selected title somewhere else
    // joins it rather than pulling it away.
    let anchors: Vec<Point> = node_ids
        .iter()
        .map(|id| map.g.nodes[id].pos())
        .chain(reaction_ids.iter().map(|id| { let r = &map.g.reactions[id]; Point::new(r.x, r.y) }))
        .collect();
    if anchors.is_empty() || anchors.len() + text_ids.len() < 2 {
        return None;
    }
    let target = anchors.iter().fold(0.0, |s, p| s + if vertical { p.x } else { p.y }) / anchors.len() as f64;

    // The metabolites keep their places along the line, in order; two that
    // would now sit on top of each other are spread to leave room for the
    // marker between them, the later ones moving on. Spacing follows where
    // each one really landed, since a membrane can stop it short.
    let along = |p: Point| if vertical { p.y } else { p.x };
    let across = |p: Point| if vertical { p.x } else { p.y };
    let on_line = |at: f64| if vertical { Point::new(target, at) } else { Point::new(at, target) };
    let mut held = Vec::new();
    let mut previous = f64::NEG_INFINITY;
    for id in line_order(map, &node_ids, &ends, &along) {
        let at = js::max(along(map.g.nodes[&id].pos()), previous + 2.0 * LINE_ALIGN_GAP);
        let p = on_line(at);
        map.move_node(&id, p.x, p.y);
        let landed = map.g.nodes[&id].pos();
        if (across(landed) - target).abs() > 0.5 || (along(landed) - at).abs() > 0.5 {
            held.push(id.clone());
        }
        previous = js::max(previous, along(landed));
    }

    // A reaction with both its metabolites on the line is kept straight
    // between them, its marker half way, as Escher draws it; one with a
    // single metabolite there brings its marker onto the line where it stands
    // along it.
    for id in &reaction_ids {
        match ends.get(id) {
            Some((f, t)) => {
                let a = map.g.nodes[f].pos();
                let b = map.g.nodes[t].pos();
                keep_straight_through(map, id, f, t, Some(Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)));
            }
            None => {
                let r = &map.g.reactions[id];
                let p = on_line(along(Point::new(r.x, r.y)));
                crate::reaction_edit::move_reaction(map, id, p.x, p.y);
                tidy_reaction(map, id);
            }
        }
    }
    let all: Vec<String> = node_ids.iter().cloned().collect();
    straighten_linear_reactions(map, Some(&all));

    // Neighbours not selected keep their place, but not curves stretched to
    // where their metabolite used to be.
    let selected: HashSet<&String> = reaction_ids.iter().collect();
    let mut neighbours: IndexSet<String> = IndexSet::new();
    for seg in &map.g.segments {
        if seg.node_id.as_ref().is_some_and(|n| node_ids.contains(n)) && !selected.contains(&seg.reaction_id) {
            neighbours.insert(seg.reaction_id.clone());
        }
    }
    for id in neighbours {
        if let Some(r) = map.g.reactions.get(&id) {
            if !r.on_membrane && r.linear.is_none() {
                tidy_reaction(map, &id);
            }
        }
    }

    for id in &text_ids {
        let t = &map.g.texts[id];
        let c = text_centre(t);
        let at = if vertical { Point::new(target, c.y) } else { Point::new(c.x, target) };
        let (x, y) = (t.x + (at.x - c.x), t.y + (at.y - c.y));
        map.move_text(id, x, y);
    }
    Some(LineAlignment { nodes: node_ids.into_iter().collect(), reactions: reaction_ids, texts: text_ids, held })
}

/// `selectionHasLineReactions`: whether a selection has a reaction that
/// Align vertical / horizontal can lay on a line.
pub fn selection_has_line_reactions(map: &EditableMap, selection: &Selection) -> bool {
    selection.reactions.iter().any(|id| map.reaction(id).is_some_and(|r| !r.on_membrane))
}

/// `lineOrder`: the order metabolites take on a line made by Align vertical /
/// horizontal.
///
/// When the selected reactions join their metabolites into one unbranched
/// chain (g6p → f6p → fdp), the chain's own order is used, from the end that
/// stands first along the line: projecting a pathway drawn across the line
/// would fold it back on itself wherever two neighbours had swapped sides.
/// Anything else keeps the order of where each stands now, as Escher does.
fn line_order(
    map: &EditableMap,
    node_ids: &IndexSet<String>,
    ends: &IndexMap<String, (String, String)>,
    along: &dyn Fn(Point) -> f64,
) -> Vec<String> {
    let mut by_place: Vec<String> = node_ids.iter().cloned().collect();
    by_place.sort_by(|a, b| {
        let d = along(map.g.nodes[a].pos()) - along(map.g.nodes[b].pos());
        if d < 0.0 {
            std::cmp::Ordering::Less
        } else if d > 0.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    if ends.is_empty() {
        return by_place;
    }
    let mut next: IndexMap<String, Vec<String>> = node_ids.iter().map(|id| (id.clone(), Vec::new())).collect();
    for (from, to) in ends.values() {
        if let Some(l) = next.get_mut(from) {
            l.push(to.clone());
        }
        if let Some(l) = next.get_mut(to) {
            l.push(from.clone());
        }
    }
    let chain = next.values().all(|links| {
        let unique: HashSet<&String> = links.iter().collect();
        links.len() <= 2 && unique.len() == links.len()
    }) && ends.len() == node_ids.len().wrapping_sub(1);
    let tips: Vec<String> = next.iter().filter(|(_, l)| l.len() == 1).map(|(id, _)| id.clone()).collect();
    if !chain || tips.len() != 2 {
        return by_place;
    }
    let start = if along(map.g.nodes[&tips[0]].pos()) <= along(map.g.nodes[&tips[1]].pos()) { &tips[0] } else { &tips[1] };
    let mut order = vec![start.clone()];
    while order.len() < node_ids.len() {
        let last = order.last().expect("non-empty").clone();
        let step = next[&last].iter().find(|id| !order.contains(id)).cloned();
        match step {
            Some(s) => order.push(s),
            None => return by_place,
        }
    }
    order
}
