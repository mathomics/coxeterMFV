//! Map layout for a multi-cell, multi-compartment community (port of
//! `src/layout/layout.js`).
//!
//! The layout is built strictly outside-in, so every boundary in the drawing
//! corresponds to a real boundary in the model:
//!
//! ```text
//!   environment (shared medium)
//!     +-- cell                      one per species
//!           +-- outermost compartment (usually `e`, the cell surface layer)
//!                 +-- ... nested organelles, from the model's compartment tree
//! ```
//!
//! A reaction is placed *inside* a compartment when all of its metabolites
//! live there, and *on the boundary* between two compartments when it crosses
//! one. That is what turns a membrane from decoration into the thing
//! transporters actually sit on.

use std::collections::HashMap;

use coxeter_core::community::{
    CommunityModel, LINK_COMPARTMENT, SHARED_COMPARTMENT, SpeciesEntry, community_exchange, namespaced,
    shared_metabolite,
};
use coxeter_core::compartments::{
    Names, cell_compartment_name, cell_compartment_visual_id, compartment_layout_profile, compartment_name,
    membrane_style,
};
use coxeter_core::geometry::{
    CLEARANCE, Corners, SIZES, bottom, center, closest_perimeter_point, closest_perimeter_point_corners,
    compartment_corners, default_corners, grid_cells, inset, inset_all,
    perimeter_point, right,
};
use coxeter_core::js;
use coxeter_core::mapdata::{
    Axis, Bounds, Environment, MapCell, MapCompartment, MapReaction, Medium, Node, NodeKind, Point, Rect, Segment,
    SegmentKind,
};
use coxeter_core::model::{Metabolite, Model, Reaction, ReactionKind};
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::force::{Edge, Item, RelaxOptions, make_random, relax};
use crate::util::{blank_node, blank_reaction, blank_segment, clamp, num_or};

const MIN_SHELL: f64 = 44.0;
const NODE_AREA: f64 = 1500.0;
/// Leaves a corridor of shared medium between cells.
const CELL_GAP: f64 = 170.0;
const ENV_PAD: f64 = 26.0;

/// What a layout places: everything, or only the shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Contents {
    #[default]
    All,
    /// Packs the cells, the nested compartments and the environment but
    /// places nothing inside them: no metabolite nodes, no reaction markers,
    /// no segments. The shells come out identical to a full layout, because
    /// their sizes are derived from the model rather than from what happens
    /// to be drawn.
    None,
}

/// Options of `layoutCommunity` (and of `EditableMap.fromCommunity`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutOptions {
    pub seed: Option<f64>,
    pub contents: Contents,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub aspect: Option<f64>,
    pub cell_gap: Option<f64>,
    pub min_shell: Option<f64>,
    pub node_spacing: Option<f64>,
    pub cell_area_multiplier: Option<f64>,
    /// `fromLayout`/`fromCommunity`: keep only these reaction ids.
    pub reactions: Option<Vec<String>>,
}

/// The clamped tuning the layout works with (`normaliseLayoutTuning`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    pub cell_gap: f64,
    pub min_shell: f64,
    pub node_spacing: f64,
    pub cell_area_multiplier: f64,
}

fn opt_or(v: Option<f64>, fallback: f64) -> f64 {
    num_or(v.unwrap_or(f64::NAN), fallback)
}

/// `normaliseLayoutTuning(opts)`: the tuning options with their defaults
/// filled in, each clamped to the range the layout supports.
pub fn normalise_layout_tuning(opts: &LayoutOptions) -> Tuning {
    Tuning {
        cell_gap: clamp(opt_or(opts.cell_gap, CELL_GAP), 80.0, 420.0),
        min_shell: clamp(opt_or(opts.min_shell, MIN_SHELL), 28.0, 110.0),
        node_spacing: clamp(opt_or(opts.node_spacing, 1.0), 0.65, 1.9),
        cell_area_multiplier: clamp(opt_or(opts.cell_area_multiplier, 1.0), 1.0, 8.0),
    }
}

/// What `layoutCommunity` returns: the shape `EditableMap` adopts.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub width: f64,
    pub height: f64,
    pub environment: Environment,
    pub cells: Vec<MapCell>,
    pub medium: Medium,
    pub nodes: IndexMap<String, Node>,
    pub reactions: IndexMap<String, MapReaction>,
    pub segments: Vec<Segment>,
}

/// `layoutCommunity(community, opts)`.
pub fn layout_community(community: &CommunityModel, opts: &LayoutOptions) -> Layout {
    let seed = opts.seed.unwrap_or(20240501.0);
    let contents = opts.contents;
    let tuning = normalise_layout_tuning(opts);
    let (auto_w, auto_h) = auto_canvas(community, opts.aspect.unwrap_or(1.34), &tuning);
    let width = opts.width.unwrap_or(auto_w);
    let height = opts.height.unwrap_or(auto_h);

    let env_rect = Rect::new(ENV_PAD, ENV_PAD, width - 2.0 * ENV_PAD, height - 2.0 * ENV_PAD);
    let species: Vec<&SpeciesEntry> = community.active_species().collect();

    let mut nodes: IndexMap<String, Node> = IndexMap::new();
    let mut reactions: IndexMap<String, MapReaction> = IndexMap::new();
    let mut cells = Vec::new();

    // Cells get area in proportion to how much they have to show.
    let cell_weights: Vec<f64> =
        species.iter().map(|sp| cell_target_area(&sp.model, tuning.node_spacing, tuning.cell_area_multiplier)).collect();
    let margin = js::min(170.0, js::max(82.0, width * 0.055));
    let cell_rects = weighted_grid(
        &inset(&env_rect, margin * 0.62, Some(margin), Some(margin * 0.62), Some(margin)),
        &cell_weights,
        tuning.cell_gap,
    );

    // The medium is laid out first because it only needs the cell rectangles,
    // and each cell then places its transfers facing the medium node they
    // trade with. Doing it the other way round leaves a cell's surface
    // metabolites stranded on the far side, with their transfer line drawn
    // straight through the cytosol.
    let medium = if contents == Contents::None {
        Medium::default()
    } else {
        layout_medium(community, &species, &env_rect, &cell_rects, &mut nodes, &mut reactions, seed + 7919.0, &tuning)
    };

    for (index, sp) in species.iter().enumerate() {
        let cell = layout_cell(
            community,
            sp,
            cell_rects[index],
            &mut nodes,
            &mut reactions,
            seed + index as f64 * 977.0,
            contents,
            &tuning,
        );
        cells.push(cell);
    }

    let segments =
        if contents == Contents::None { Vec::new() } else { build_segments(community, &mut nodes, &mut reactions) };

    let mut shared = Names::new();
    shared.insert(SHARED_COMPARTMENT.into(), "Shared medium".into());
    Layout {
        width,
        height,
        environment: Environment {
            rect: env_rect,
            id: SHARED_COMPARTMENT.into(),
            label: compartment_name(SHARED_COMPARTMENT, &shared, "en"),
            hidden: None,
            extra: Map::new(),
        },
        cells,
        medium,
        nodes,
        reactions,
        segments,
    }
}

// Cells

/// What a relaxation item stands for.
#[derive(Clone, Copy)]
enum ItemNode<'a> {
    Reaction(&'a Reaction),
    Metabolite(&'a Metabolite),
}

/// The fields of a JS layout item that `force.relax` does not read.
#[derive(Clone)]
struct Meta<'a> {
    key: String,
    node: ItemNode<'a>,
    nx: f64,
    ny: f64,
    membrane: Option<String>,
    on_membrane: bool,
    transfer: bool,
    seeded: bool,
}

#[derive(Clone, Copy)]
struct Hint {
    v: f64,
    u: f64,
}

#[derive(Clone, Copy)]
struct HintRange {
    min_rank: f64,
    max_rank: f64,
    min_lane: f64,
    max_lane: f64,
}

#[allow(clippy::too_many_arguments)]
fn layout_cell(
    community: &CommunityModel,
    sp: &SpeciesEntry,
    cell_rect: Rect,
    nodes: &mut IndexMap<String, Node>,
    reactions: &mut IndexMap<String, MapReaction>,
    seed: f64,
    contents: Contents,
    tuning: &Tuning,
) -> MapCell {
    let model: &Model = &sp.model;
    let tree = model.compartment_tree();
    let names = &model.compartment_names;
    let layout_profile = compartment_layout_profile(&model.compartment_ids(), names);
    let ns = tuning.node_spacing;

    // How much room each compartment needs
    let mut own: IndexMap<String, f64> = IndexMap::new();
    let mut internal_reactions: IndexMap<String, Vec<&Reaction>> = IndexMap::new();
    let mut crossing: Vec<(&Reaction, Vec<String>)> = Vec::new();
    let mut exchanges: Vec<&Reaction> = Vec::new();
    let mut primary_mets: IndexMap<String, Vec<&Metabolite>> = IndexMap::new();

    for id in tree.by_id.keys() {
        own.insert(id.clone(), 0.0);
        internal_reactions.insert(id.clone(), Vec::new());
        primary_mets.insert(id.clone(), Vec::new());
    }
    for met in model.metabolites.values() {
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        let Some(list) = primary_mets.get_mut(&met.compartment) else { continue };
        if met.is_primary() {
            list.push(met);
        }
    }
    for rxn in model.reactions.values() {
        let comps: Vec<String> = rxn.compartments.iter().filter(|c| tree.by_id.contains_key(*c)).cloned().collect();
        if rxn.kind == ReactionKind::Exchange {
            exchanges.push(rxn);
            continue;
        }
        if comps.len() == 1 {
            internal_reactions.get_mut(&comps[0]).expect("tree id").push(rxn);
        } else if comps.len() > 1 {
            crossing.push((rxn, comps));
        }
    }
    for id in tree.by_id.keys() {
        let v = primary_mets[id].len() as f64 + internal_reactions[id].len() as f64 * 1.15 + 1.0;
        own.insert(id.clone(), v);
    }
    let mut total: HashMap<String, f64> = HashMap::new();
    fn compute_total(
        id: &str,
        tree: &coxeter_core::compartments::CompartmentTree,
        own: &IndexMap<String, f64>,
        total: &mut HashMap<String, f64>,
    ) -> f64 {
        let mut t = own[id];
        for ch in &tree.by_id[id].children {
            t += compute_total(ch, tree, own, total);
        }
        total.insert(id.to_string(), t);
        t
    }
    for r in &tree.roots {
        compute_total(r, &tree, &own, &mut total);
    }

    // Nested packing
    let mut compartments: Vec<MapCompartment> = Vec::new();
    let mut rect_of: HashMap<String, Rect> = HashMap::new();
    let root_weights: Vec<f64> = tree.roots.iter().map(|r| total[r]).collect();
    // What a compartment needs before it is cramped: room for its own nodes,
    // three times over, since a map drawn by hand is never packed tight.
    let room_for = |id: &str| total[id] * NODE_AREA * ns * ns * 3.0;
    let root_rects = if tree.roots.len() == 1 {
        vec![cell_rect]
    } else {
        let mins: Vec<f64> = tree.roots.iter().map(|r| room_for(r)).collect();
        pack_compartments(&cell_rect, &root_weights, 16.0, &mins)
    };

    struct Packer<'t> {
        tree: &'t coxeter_core::compartments::CompartmentTree,
        names: &'t Names,
        own: &'t IndexMap<String, f64>,
        total: &'t HashMap<String, f64>,
        tuning: Tuning,
        yeast9: bool,
    }
    impl Packer<'_> {
        fn room_for(&self, id: &str) -> f64 {
            self.total[id] * NODE_AREA * self.tuning.node_spacing * self.tuning.node_spacing * 3.0
        }
        fn pack_nested(&self, id: &str, r: Rect, rect_of: &mut HashMap<String, Rect>, out: &mut Vec<MapCompartment>) {
            rect_of.insert(id.to_string(), r);
            let node = &self.tree.by_id[id];
            let visual_id = cell_compartment_visual_id(id, self.names);
            out.push(MapCompartment {
                id: id.to_string(),
                rect: r,
                depth: node.depth as f64,
                parent_id: node.parent.clone(),
                label: cell_compartment_name(id, self.names, "en"),
                membrane: membrane_style(&visual_id, &Names::new()).as_str().to_string(),
                visual_id: visual_id.clone(),
                fill: None,
                stroke: None,
                shape: None,
                extra: Map::new(),
            });
            if node.children.is_empty() {
                return;
            }
            let ns = self.tuning.node_spacing;
            let child_total = node.children.iter().fold(0.0, |s, ch| s + self.total[ch]);
            let own_weight = self.own[id];

            // Give the nested compartments a share of the area proportional to
            // how much they have to show, so a cytosol with 70 nodes is not
            // squeezed into a thin ring around a mitochondrion with 20. The
            // shell is what is left over, with a floor so a membrane always
            // has room for the metabolites sitting on it.
            let inner_frac = clamp(child_total / (own_weight + child_total), 0.12, 0.9);
            let scale = inner_frac.sqrt();
            // The floor scales with the compartment so deep nesting does not
            // run out of room: four stacked membranes at a fixed 44 px would
            // eat a small cell.
            let floor = clamp(js::min(r.w, r.h) * 0.09, 15.0, self.tuning.min_shell);
            let shell_x = clamp((r.w * (1.0 - scale)) / 2.0, floor, r.w * 0.34);
            let shell_y = clamp((r.h * (1.0 - scale)) / 2.0, floor, r.h * 0.34);
            let mut inner = inset(&r, shell_y, Some(shell_x), Some(shell_y), Some(shell_x));

            // Last resort: if the ring is still too thin for this
            // compartment's own nodes, take an extra band off the long axis so
            // the parent keeps one contiguous area to lay a pathway out in.
            let needed = own_weight * NODE_AREA * ns * ns;
            let ring_area = r.w * r.h - inner.w * inner.h;
            if ring_area < needed {
                let deficit = needed - ring_area;
                if inner.w >= inner.h {
                    let band = js::min(deficit / js::max(inner.h, 1.0), inner.w * 0.5);
                    inner = Rect::new(inner.x + band, inner.y, inner.w - band, inner.h);
                } else {
                    let band = js::min(deficit / js::max(inner.w, 1.0), inner.h * 0.5);
                    inner = Rect::new(inner.x, inner.y + band, inner.w, inner.h - band);
                }
            }
            // Yeast9 has multiple organelles that coexist as islands in the
            // cytosol. Treating them as a tiled grid makes the cytosol look
            // like a mosaic and visually erases the fact that these are
            // separate organelles. Spread the organelles through the cytosolic
            // space instead; their own children (ER lumen, mitochondrial
            // interior, Golgi and vacuole) remain normally nested.
            if self.yeast9 && id == "c" {
                let pad = clamp(js::min(r.w, r.h) * 0.045, 24.0, 84.0);
                inner = inset_all(&r, pad);
            }

            let parts = if node.children.len() == 1 {
                vec![inner]
            } else if self.yeast9 && id == "c" {
                yeast9_organelle_rects(&inner, &node.children)
            } else {
                let weights: Vec<f64> = node.children.iter().map(|ch| self.total[ch]).collect();
                let mins: Vec<f64> = node.children.iter().map(|ch| self.room_for(ch)).collect();
                pack_compartments(&inner, &weights, 18.0, &mins)
            };
            for (i, ch) in node.children.iter().enumerate() {
                self.pack_nested(ch, parts[i], rect_of, out);
            }
        }
    }
    let packer = Packer { tree: &tree, names, own: &own, total: &total, tuning: *tuning, yeast9: layout_profile == "yeast9" };
    for (i, root) in tree.roots.iter().enumerate() {
        packer.pack_nested(root, root_rects[i], &mut rect_of, &mut compartments);
    }

    let mut child_rects: HashMap<String, Vec<Rect>> = HashMap::new();
    for c in &compartments {
        child_rects.insert(
            c.id.clone(),
            compartments.iter().filter(|o| o.parent_id.as_deref() == Some(c.id.as_str())).map(|o| o.rect).collect(),
        );
    }

    // An empty map stops here: the cell, its membranes and the compartments
    // nested inside them are the whole drawing. Everything below this point
    // places contents, and contents are exactly what an empty map has none of.
    if contents == Contents::None {
        return describe_cell(sp, cell_rect, compartments);
    }

    // Primary metabolites and single-compartment reactions
    let mut items: Vec<Item> = Vec::new();
    let mut metas: Vec<Meta> = Vec::new();
    let mut item_index: HashMap<String, usize> = HashMap::new();
    let mut rand = make_random(seed);

    let hints_by_compartment = collect_hints(model, &tree);
    let mut ctx = ItemCtx { rect_of: &rect_of, child_rects: &child_rects, ns, rand: &mut rand };
    for (comp_id, list) in &internal_reactions {
        for rxn in list {
            let hint = normalise_hint(rxn.layout.as_ref(), hints_by_compartment.get(comp_id).copied());
            ctx.push_item(
                format!("rxn:{}", rxn.id),
                ItemNode::Reaction(rxn),
                comp_id,
                SIZES.marker_radius,
                hint,
                (&mut items, &mut metas, &mut item_index),
            );
        }
    }
    for (comp_id, list) in &primary_mets {
        for met in list {
            let hint = metabolite_hint(met, model, hints_by_compartment.get(comp_id).copied());
            ctx.push_item(
                format!("met:{}", met.id),
                ItemNode::Metabolite(met),
                comp_id,
                SIZES.primary_radius,
                hint,
                (&mut items, &mut metas, &mut item_index),
            );
        }
    }

    let mut edges: Vec<Edge> = Vec::new();
    for list in internal_reactions.values() {
        for rxn in list {
            let ri = item_index[&format!("rxn:{}", rxn.id)];
            for mid in rxn.metabolites.keys() {
                let Some(&mi) = item_index.get(&format!("met:{mid}")) else { continue };
                edges.push(Edge { a: ri, b: mi, length: 62.0 * ns, strength: 1.0 });
            }
        }
    }

    let repulsion = 1.25 * ns * ns;
    relax(&mut items, &edges, RelaxOptions { iterations: 360, seed, repulsion, rank_strength: 0.10 });

    // Stage A: transfers to the shared medium, on the cell's outer boundary.
    // These go first because the medium node on the other side is already
    // fixed, so they are the one end of the chain that is not free to move.
    // Each surface metabolite is then pulled to the side of the cell that
    // faces its partner.
    let outer_id = tree.roots.first().cloned().unwrap_or_else(|| "e".into());
    let outer_rect = rect_of.get(&outer_id).copied().unwrap_or(cell_rect);
    let outer_radius = match compartments.iter().find(|c| c.id == outer_id) {
        Some(c) => compartment_corners(c),
        None => default_corners(&outer_id, &outer_rect),
    };
    let mut used_outer: Vec<f64> = Vec::new();
    let mut transfer_items: Vec<(Item, Meta)> = Vec::new();
    let mut aimed: Vec<(&Reaction, f64)> = exchanges
        .iter()
        .map(|rxn| {
            let local_id = rxn.metabolites.keys().next();
            let met = local_id.and_then(|id| model.metabolites.get(id));
            let target = met.and_then(|m| nodes.get(&format!("met:{}", shared_metabolite(&m.base))));
            let anchor = target.map(|n| Point::new(n.x, n.y)).unwrap_or_else(|| center(&outer_rect));
            (*rxn, perimeter_param(&outer_rect, closest_perimeter_point(&outer_rect, anchor, 0.0)))
        })
        .collect();
    aimed.sort_by(|a, b| {
        let d = a.1 - b.1;
        if d != 0.0 && !d.is_nan() {
            return d.partial_cmp(&0.0).unwrap();
        }
        js::locale_compare(&a.0.id, &b.0.id)
    });
    for (rxn, t) in aimed {
        let slot = spread_along_perimeter(t, &used_outer, min_spacing(&outer_rect, 46.0 * ns));
        used_outer.push(slot);
        let perimeter = perimeter_point(&outer_rect, slot);
        let p = closest_perimeter_point_corners(&outer_rect, perimeter.point(), outer_radius);
        transfer_items.push((
            Item { x: p.x, y: p.y, r: SIZES.marker_radius, ..Default::default() },
            Meta {
                key: format!("rxn:{}", rxn.id),
                node: ItemNode::Reaction(rxn),
                nx: p.nx,
                ny: p.ny,
                membrane: Some(outer_id.clone()),
                on_membrane: true,
                transfer: true,
                seeded: false,
            },
        ));
    }

    let mut edges2 = edges.clone();
    edges2.extend(pin_to_membrane(model, ns, transfer_items, &mut items, &mut metas, &mut item_index));
    relax(&mut items, &edges2, RelaxOptions { iterations: 230, seed: seed + 31.0, repulsion, rank_strength: 0.0 });

    // Stage B: transporters on the membranes they cross.
    // Placed after the metabolites have settled against the outer boundary,
    // so an inner transporter lands on the side its partners actually ended
    // up on.
    let mut membrane_groups: IndexMap<String, Vec<&Reaction>> = IndexMap::new();
    for (rxn, comps) in &crossing {
        let target = innermost_of(comps, &tree);
        membrane_groups.entry(target).or_default().push(rxn);
    }

    let mut transporter_items: Vec<(Item, Meta)> = Vec::new();
    for (comp_id, list) in membrane_groups.iter_mut() {
        let r = rect_of[comp_id];
        let radius = match compartments.iter().find(|c| &c.id == comp_id) {
            Some(c) => compartment_corners(c),
            None => Corners::circular(f64::NAN),
        };
        list.sort_by(|a, b| {
            let d = layout_lane(a) - layout_lane(b);
            if d != 0.0 && !d.is_nan() {
                return d.partial_cmp(&0.0).unwrap();
            }
            js::locale_compare(&a.id, &b.id)
        });
        let mut used: Vec<f64> = Vec::new();
        let count = list.len();
        for (i, rxn) in list.iter().enumerate() {
            // Aim at the metabolites this reaction actually touches, then
            // slide along the perimeter until it stops colliding with its
            // neighbours.
            let partners: Vec<&Item> = rxn
                .metabolites
                .keys()
                .filter_map(|mid| item_index.get(&format!("met:{mid}")).map(|&k| &items[k]))
                .collect();
            let mut t = if !partners.is_empty() {
                let cx = partners.iter().fold(0.0, |s, p| s + p.x) / partners.len() as f64;
                let cy = partners.iter().fold(0.0, |s, p| s + p.y) / partners.len() as f64;
                perimeter_param(&r, closest_perimeter_point(&r, Point::new(cx, cy), 0.0))
            } else {
                i as f64 / js::max(count as f64, 1.0)
            };
            t = spread_along_perimeter(t, &used, min_spacing(&r, 54.0 * ns));
            used.push(t);
            let perimeter = perimeter_point(&r, t);
            let p = closest_perimeter_point_corners(&r, perimeter.point(), radius);
            transporter_items.push((
                Item { x: p.x, y: p.y, r: SIZES.marker_radius, ..Default::default() },
                Meta {
                    key: format!("rxn:{}", rxn.id),
                    node: ItemNode::Reaction(rxn),
                    nx: p.nx,
                    ny: p.ny,
                    membrane: Some(comp_id.clone()),
                    on_membrane: true,
                    transfer: false,
                    seeded: false,
                },
            ));
        }
    }

    let mut edges3 = edges2.clone();
    edges3.extend(pin_to_membrane(model, ns, transporter_items, &mut items, &mut metas, &mut item_index));
    relax(&mut items, &edges3, RelaxOptions { iterations: 180, seed: seed + 61.0, repulsion, rank_strength: 0.0 });

    // Commit to the shared node and reaction tables
    for (item, meta) in items.iter().zip(&metas) {
        match &meta.node {
            ItemNode::Metabolite(met) => {
                let merged = namespaced(&sp.id, &met.id);
                let id = format!("met:{merged}");
                let mut node = blank_node(&id, NodeKind::Metabolite);
                node.merged_id = Some(merged);
                node.local_id = Some(met.id.clone());
                node.base = Some(met.base.clone());
                node.name = Some(met.name.clone());
                node.formula = Some(met.formula.clone());
                node.compartment = Some(met.compartment.clone());
                node.visual_compartment = Some(cell_compartment_visual_id(&met.compartment, names));
                node.cell_id = Some(sp.id.clone());
                node.x = item.x;
                node.y = item.y;
                node.r = SIZES.primary_radius;
                node.primary = Some(true);
                nodes.insert(id, node);
            }
            ItemNode::Reaction(rxn) => {
                let merged_id = namespaced(&sp.id, &rxn.id);
                // A transfer's stoichiometry, bounds and compartments only
                // exist in the merged model: in the species' own model it is
                // still a boundary exchange.
                let merged_rxn = community.merged.reactions.get(&merged_id);
                let source = match merged_rxn {
                    Some(m) if meta.transfer => m,
                    _ => *rxn,
                };
                let mut out = blank_reaction(&merged_id);
                out.local_id = Some(rxn.id.clone());
                out.name = rxn.name.clone();
                out.subsystem =
                    Some(if meta.transfer { "Intercambio con el medio".to_string() } else { rxn.subsystem.clone() });
                out.cell_id = Some(sp.id.clone());
                out.kind = if meta.transfer { "transfer".into() } else { rxn.kind.as_str().into() };
                out.compartments = source.compartments.clone();
                out.membrane = meta.membrane.clone();
                out.membrane_visual_id = meta.membrane.as_ref().map(|m| cell_compartment_visual_id(m, names));
                out.on_membrane = meta.on_membrane;
                out.normal = meta.on_membrane.then(|| Point::new(meta.nx, meta.ny));
                out.x = item.x;
                out.y = item.y;
                out.equation = Some(source.equation());
                out.bounds = Some(Bounds { lower: source.lower_bound, upper: source.upper_bound });
                reactions.insert(merged_id, out);
            }
        }
    }

    describe_cell(sp, cell_rect, compartments)
}

struct ItemCtx<'r> {
    rect_of: &'r HashMap<String, Rect>,
    child_rects: &'r HashMap<String, Vec<Rect>>,
    ns: f64,
    rand: &'r mut crate::force::Random,
}

type ItemTables<'v, 'a> = (&'v mut Vec<Item>, &'v mut Vec<Meta<'a>>, &'v mut HashMap<String, usize>);

impl ItemCtx<'_> {
    fn push_item<'a>(
        &mut self,
        key: String,
        node: ItemNode<'a>,
        compartment_id: &str,
        radius: f64,
        hint: Option<Hint>,
        (items, metas, item_index): ItemTables<'_, 'a>,
    ) {
        let r = self.rect_of[compartment_id];
        let usable = inset_all(&r, radius + 10.0 * self.ns);
        let (x, y, anchor) = hint_to_point(hint, &usable, self.rand);
        item_index.insert(key.clone(), items.len());
        items.push(Item {
            x,
            y,
            r: radius,
            fixed: false,
            bounds: Some(r),
            holes: self.child_rects.get(compartment_id).cloned().unwrap_or_default(),
            anchor_x: anchor.map(|a| a.0),
            anchor_y: anchor.map(|a| a.1),
        });
        metas.push(Meta { key, node, nx: 0.0, ny: 0.0, membrane: None, on_membrane: false, transfer: false, seeded: false });
    }
}

/// Pin a reaction to a membrane and let the relaxation pull its metabolites
/// towards it. Returns the edges the next pass should honour.
///
/// Low-degree partners are also *moved* next to the marker rather than only
/// being pulled: a metabolite sitting in a thin shell cannot reach the far
/// side of its compartment on a spring alone, because the straight line runs
/// through the organelle inside it and the containment force cancels the
/// pull. Seeding the position instead of fighting for it is what keeps a
/// transfer from being drawn right across the cytosol.
fn pin_to_membrane<'a>(
    model: &Model,
    ns: f64,
    placed: Vec<(Item, Meta<'a>)>,
    items: &mut Vec<Item>,
    metas: &mut Vec<Meta<'a>>,
    item_index: &mut HashMap<String, usize>,
) -> Vec<Edge> {
    let mut extra = Vec::new();
    for (t, tm) in placed {
        item_index.insert(tm.key.clone(), items.len());
        let index = items.len();
        let (tx, ty, tnx, tny) = (t.x, t.y, tm.nx, tm.ny);
        let membrane = tm.membrane.clone();
        let ItemNode::Reaction(rxn) = tm.node else { unreachable!("only reactions are pinned") };
        items.push(Item { fixed: true, bounds: None, holes: Vec::new(), ..t });
        metas.push(tm);

        for mid in rxn.metabolites.keys() {
            let Some(&mi) = item_index.get(&format!("met:{mid}")) else { continue };
            extra.push(Edge { a: index, b: mi, length: 56.0 * ns, strength: 1.4 });

            let Some(met) = model.metabolites.get(mid) else { continue };
            if met.reactions.len() > 3 {
                continue; // hubs keep their place
            }
            if metas[mi].seeded {
                continue;
            }
            // Inside the membrane if the metabolite belongs to it, outside if
            // it belongs to the compartment this one is nested in.
            let inside = if Some(&met.compartment) == membrane.as_ref() { -1.0 } else { 1.0 };
            let offset = (items[mi].r + 22.0 * ns) * inside;
            items[mi].x = tx + tnx * offset;
            items[mi].y = ty + tny * offset;
            metas[mi].seeded = true;
        }
    }
    extra
}

/// `rxn.layout?.lane ?? 0`, as a number.
fn layout_lane(rxn: &Reaction) -> f64 {
    layout_field(rxn.layout.as_ref(), "lane")
}

/// `layout.<key> ?? 0` read as JavaScript arithmetic reads it.
fn layout_field(layout: Option<&Value>, key: &str) -> f64 {
    match layout.and_then(|l| l.as_object()).and_then(|o| o.get(key)) {
        None | Some(Value::Null) => 0.0,
        Some(v) => js::to_number(v),
    }
}

/// The cell record the map is built from; identical for full and empty maps.
fn describe_cell(sp: &SpeciesEntry, cell_rect: Rect, compartments: Vec<MapCompartment>) -> MapCell {
    let outer = compartments.iter().find(|c| c.parent_id.is_none()).map(|c| c.id.clone()).unwrap_or_else(|| "e".into());
    MapCell {
        id: sp.id.clone(),
        name: sp.model.cell.name.clone(),
        organism: Some(sp.model.cell.organism.clone()),
        note: Some(sp.model.cell.note.clone()),
        color: sp.color.clone(),
        abundance: Some(sp.abundance),
        rect: cell_rect,
        compartments,
        outer_compartment: Some(outer),
        stroke: None,
        manually_drawn: None,
        extra: Map::new(),
    }
}

// Medium

#[allow(clippy::too_many_arguments)]
fn layout_medium(
    community: &CommunityModel,
    species: &[&SpeciesEntry],
    env_rect: &Rect,
    cell_rects: &[Rect],
    nodes: &mut IndexMap<String, Node>,
    reactions: &mut IndexMap<String, MapReaction>,
    seed: f64,
    tuning: &Tuning,
) -> Medium {
    let mut bases: Vec<String> = community.transfers_by_base.keys().cloned().collect();
    bases.sort_by(|a, b| js::code_unit_compare(a, b));
    if bases.is_empty() {
        return Medium::default();
    }
    let ns = tuning.node_spacing;
    let mut items: Vec<Item> = Vec::new();
    let mut degrees: Vec<usize> = Vec::new();
    let mut rand = make_random(seed);
    let cell_centres: Vec<Point> = cell_rects.iter().map(center).collect();

    for base in &bases {
        let transfers = &community.transfers_by_base[base];
        let partners: Vec<Point> = transfers
            .iter()
            .filter_map(|t| species.iter().position(|s| s.id == t.species_id))
            .map(|k| cell_centres[k])
            .collect();
        let (cx, cy) = if partners.is_empty() {
            (center(env_rect).x, center(env_rect).y)
        } else {
            let n = partners.len() as f64;
            (partners.iter().fold(0.0, |s, p| s + p.x) / n, partners.iter().fold(0.0, |s, p| s + p.y) / n)
        };
        let x = cx + (rand.next() - 0.5) * 120.0 * ns;
        let y = cy + (rand.next() - 0.5) * 120.0 * ns;
        items.push(Item {
            x,
            y,
            r: SIZES.medium_radius,
            fixed: false,
            bounds: Some(inset_all(env_rect, CLEARANCE.environment * ns)),
            holes: cell_rects.iter().map(|r| inset_all(r, -CLEARANCE.cell * ns)).collect(),
            anchor_x: None,
            anchor_y: None,
        });
        degrees.push(transfers.len());
    }

    relax(&mut items, &[], RelaxOptions { iterations: 380, seed, repulsion: 1.7 * ns * ns, rank_strength: 0.0 });

    for ((item, base), degree) in items.iter().zip(&bases).zip(&degrees) {
        let merged = shared_metabolite(base);
        let id = format!("met:{merged}");
        let mut node = blank_node(&id, NodeKind::Medium);
        node.merged_id = Some(merged);
        node.local_id = Some(base.clone());
        node.base = Some(base.clone());
        node.name = Some(base.clone());
        node.compartment = Some(SHARED_COMPARTMENT.into());
        node.cell_id = None;
        node.x = item.x;
        node.y = item.y;
        node.r = SIZES.medium_radius;
        node.primary = Some(true);
        node.degree = Some(*degree as f64);
        nodes.insert(id, node);
    }

    // Community boundary exchanges: a short stub pointing out of the environment.
    let mut exchanges = Vec::new();
    for base in &bases {
        let ex_id = community_exchange(base);
        let Some(rxn) = community.merged.reactions.get(&ex_id) else { continue };
        let node = &nodes[&format!("met:{}", shared_metabolite(base))];
        let p = closest_perimeter_point(env_rect, node.pos(), 0.0);
        let dx = p.x - node.x;
        let dy = p.y - node.y;
        let len = num_or(js::hypot(dx, dy), 1.0);
        let mut out = blank_reaction(&ex_id);
        out.local_id = Some(ex_id.clone());
        out.name = rxn.name.clone();
        out.subsystem = Some("Community exchange".into());
        out.cell_id = None;
        out.kind = "exchange".into();
        out.compartments = vec![SHARED_COMPARTMENT.into()];
        out.membrane = None;
        out.on_membrane = false;
        out.normal = Some(Point::new(dx / len, dy / len));
        out.x = node.x + (dx / len) * js::min(len * 0.5, 46.0);
        out.y = node.y + (dy / len) * js::min(len * 0.5, 46.0);
        out.equation = Some(rxn.equation());
        out.bounds = Some(Bounds { lower: rxn.lower_bound, upper: rxn.upper_bound });
        // A short stub, not a line all the way to the canvas edge: the
        // exchange only needs to read as "in or out of the system".
        out.boundary = Some(Point::new(
            node.x + (dx / len) * js::min(len, 92.0),
            node.y + (dy / len) * js::min(len, 92.0),
        ));
        reactions.insert(ex_id.clone(), out);
        exchanges.push(ex_id);
    }

    Medium {
        nodes: bases.iter().map(|b| format!("met:{}", shared_metabolite(b))).collect(),
        exchanges,
        extra: Map::new(),
    }
}

// Segments

struct Entry<'a> {
    mid: &'a str,
    coef: f64,
    node: Option<Point>,
    node_id: Option<String>,
    met: &'a Metabolite,
}

/// Turn each reaction into Escher-style geometry: substrates converge on one
/// end of a short reaction axis, products leave from the other, and every
/// non-primary metabolite gets its own satellite node next to the marker.
fn build_segments(
    community: &CommunityModel,
    nodes: &mut IndexMap<String, Node>,
    reactions: &mut IndexMap<String, MapReaction>,
) -> Vec<Segment> {
    let mut segments = Vec::new();
    let model = &community.merged;

    for (rid, rxn) in reactions.iter_mut() {
        let Some(mrxn) = model.reactions.get(rid) else { continue };

        let mut subs: Vec<Entry> = Vec::new();
        let mut prods: Vec<Entry> = Vec::new();
        for (mid, &coef) in &mrxn.metabolites {
            let Some(met) = model.metabolites.get(mid) else { continue };
            if met.compartment == LINK_COMPARTMENT {
                continue;
            }
            let key = format!("met:{mid}");
            let node = nodes.get(&key);
            let entry = Entry { mid, coef, node: node.map(|n| n.pos()), node_id: node.map(|n| n.id.clone()), met };
            if coef < 0.0 { subs.push(entry) } else { prods.push(entry) }
        }

        // Direction of the reaction axis: from substrates towards products.
        let dir = axis_direction(rxn, &subs, &prods);
        let half = SIZES.reaction_axis;
        let a1 = Point::new(rxn.x - dir.x * half, rxn.y - dir.y * half);
        let a2 = Point::new(rxn.x + dir.x * half, rxn.y + dir.y * half);
        rxn.axis = Some(Axis { a1, a2, dir });

        segments.push(blank_segment(rid, SegmentKind::Axis, a1, a2, false));

        let mut cur_index = 0usize;
        let mut place_satellite = |entry: &Entry, side: f64, nodes: &mut IndexMap<String, Node>| -> (String, Point) {
            let perp = Point::new(-dir.y, dir.x);
            let slot = (cur_index / 2) as f64 + 1.0;
            let sign = if cur_index % 2 == 0 { 1.0 } else { -1.0 };
            cur_index += 1;
            let anchor = if side < 0.0 { a1 } else { a2 };
            let dist = 26.0 + slot * 17.0;
            let px = anchor.x + perp.x * sign * dist + dir.x * side * 12.0;
            let py = anchor.y + perp.y * sign * dist + dir.y * side * 12.0;
            let id = format!("cur:{rid}:{}", entry.mid);
            let mut node = blank_node(&id, NodeKind::Currency);
            node.merged_id = Some(entry.mid.to_string());
            node.local_id = Some(entry.met.id.clone());
            node.base = Some(entry.met.base.clone());
            node.name = Some(entry.met.name.clone());
            node.formula = Some(entry.met.formula.clone());
            node.compartment = Some(entry.met.compartment.clone());
            node.cell_id = rxn.cell_id.clone();
            node.reaction_id = Some(rid.clone());
            node.x = px;
            node.y = py;
            node.r = SIZES.currency_radius;
            node.primary = Some(false);
            nodes.insert(id.clone(), node);
            (id, Point::new(px, py))
        };

        for entry in &subs {
            let (id, p) = match (&entry.node_id, entry.node) {
                (Some(id), Some(p)) => (id.clone(), p),
                _ => place_satellite(entry, -1.0, nodes),
            };
            let kind = if entry.node.is_some() { SegmentKind::Primary } else { SegmentKind::Currency };
            let mut seg = blank_segment(rid, kind, p, a1, false);
            seg.node_id = Some(id);
            seg.coefficient = Some(entry.coef);
            seg.side = Some("substrate".into());
            segments.push(seg);
        }
        for entry in &prods {
            let (id, p) = match (&entry.node_id, entry.node) {
                (Some(id), Some(p)) => (id.clone(), p),
                _ => place_satellite(entry, 1.0, nodes),
            };
            let kind = if entry.node.is_some() { SegmentKind::Primary } else { SegmentKind::Currency };
            let mut seg = blank_segment(rid, kind, a2, p, true);
            seg.node_id = Some(id);
            seg.coefficient = Some(entry.coef);
            seg.side = Some("product".into());
            segments.push(seg);
        }

        if let Some(boundary) = rxn.boundary {
            segments.push(blank_segment(rid, SegmentKind::Boundary, a2, boundary, true));
        }
    }

    segments
}

fn axis_direction(rxn: &MapReaction, subs: &[Entry], prods: &[Entry]) -> Point {
    if rxn.on_membrane {
        if let Some(n) = rxn.normal {
            // Transporters point across the membrane they sit on.
            return n;
        }
    }
    let from = centroid_of(subs).unwrap_or(Point::new(rxn.x, rxn.y - 1.0));
    let to = centroid_of(prods).unwrap_or(Point::new(rxn.x, rxn.y + 1.0));
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = js::hypot(dx, dy);
    if len < 1e-6 {
        return Point::new(0.0, 1.0);
    }
    Point::new(dx / len, dy / len)
}

fn centroid_of(list: &[Entry]) -> Option<Point> {
    let with: Vec<Point> = list.iter().filter_map(|e| e.node).collect();
    if with.is_empty() {
        return None;
    }
    let n = with.len() as f64;
    Some(Point::new(with.iter().fold(0.0, |s, p| s + p.x) / n, with.iter().fold(0.0, |s, p| s + p.y) / n))
}

// Helpers

/// Room a single cell needs, from how much of its network has to be drawn.
fn cell_target_area(model: &Model, node_spacing: f64, cell_area_multiplier: f64) -> f64 {
    let spacing_area = node_spacing * node_spacing;
    let base_area = 230000.0 + 5600.0 * spacing_area * model.reactions.len() as f64;
    // The outer canvas has fixed margins and inter-cell corridors.
    // Multiplying the raw target by 6 would therefore yield substantially more
    // than 6× of realised cell rectangle area (about 7.7× in a four-cell
    // community). Compensate for that geometry so the user-facing multiplier
    // describes the area they actually see, not an internal pre-margin number.
    let realised_scale = 1.0 + (cell_area_multiplier - 1.0) * 0.74;
    base_area * realised_scale
}

/// Pick a canvas big enough that every cell gets its target area and the
/// shared medium still has room between the cells. The SVG viewBox scales
/// this down to whatever the screen is, so a large canvas costs nothing but
/// gives the layout space to breathe.
fn auto_canvas(community: &CommunityModel, aspect: f64, tuning: &Tuning) -> (f64, f64) {
    let active: Vec<&SpeciesEntry> = community.active_species().collect();
    let cell_area = active
        .iter()
        .fold(0.0, |sum, sp| sum + cell_target_area(&sp.model, tuning.node_spacing, tuning.cell_area_multiplier));
    // Spacious layouts intentionally devote more of the viewport to cells
    // rather than merely scaling the whole canvas up. This makes nodes and
    // labels genuinely easier to read after Fit-to-screen instead of
    // cancelling the extra spacing through a lower zoom factor.
    let density = (tuning.node_spacing - 0.65) / (1.9 - 0.65);
    let cell_coverage = clamp(0.48 + density * 0.14, 0.48, 0.62);
    let gap_allowance = js::max(0.0, active.len() as f64 - 1.0) * tuning.cell_gap * tuning.cell_gap * 0.42;
    let canvas_area = js::max(1.6e6, cell_area / cell_coverage + gap_allowance);
    let width = js::round((canvas_area * aspect).sqrt());
    (width, js::round(width / aspect))
}

/// Fractions of the cytosolic rectangle. They deliberately leave broad
/// cytosolic corridors between organelles so pathways can run around them.
const YEAST9_ORGANELLE_SLOTS: [(&str, [f64; 4]); 7] = [
    ("erm", [0.05, 0.05, 0.50, 0.20]), // ER membrane + ER lumen
    ("gm", [0.67, 0.07, 0.25, 0.18]),  // Golgi membrane + Golgi
    ("n", [0.34, 0.31, 0.28, 0.29]),   // nucleus
    ("p", [0.08, 0.36, 0.17, 0.17]),   // peroxisome (Yeast9 p)
    ("lp", [0.76, 0.33, 0.14, 0.12]),  // lipid particle
    ("mm", [0.05, 0.67, 0.39, 0.20]),  // mitochondrial membrane + interior
    ("vm", [0.63, 0.50, 0.29, 0.34]),  // vacuolar membrane + vacuole
];

fn yeast9_organelle_rects(r: &Rect, children: &[String]) -> Vec<Rect> {
    let fallback = weighted_grid(r, &vec![1.0; children.len()], 18.0);
    children
        .iter()
        .enumerate()
        .map(|(i, child)| match YEAST9_ORGANELLE_SLOTS.iter().find(|(id, _)| id == child) {
            None => fallback[i],
            Some((_, s)) => Rect::new(
                r.x + s[0] * r.w,
                r.y + s[1] * r.h,
                js::max(24.0, s[2] * r.w),
                js::max(24.0, s[3] * r.h),
            ),
        })
        .collect()
}

/// Lay out cells of similar kind: equal grid cells, nudged towards their
/// weights and squared up.
pub fn weighted_grid(r: &Rect, weights: &[f64], gap: f64) -> Vec<Rect> {
    let n = weights.len();
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![*r];
    }
    let cells = grid_cells(r, n, gap, 1.08);
    // gridCells gives equal cells; nudge sizes towards the weights within each row.
    let total = weights.iter().fold(0.0, |a, b| a + b) / n as f64;
    const TARGET_ASPECT: f64 = 1.18;
    cells
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let scale = js::min(1.25, js::max(0.55, weights[i] / total)).sqrt();
            let wanted = c.w * scale * c.h * scale;
            // Keep the area the weight earned, but square the cell up: a 2.2:1
            // cell leaves a third of the cytosol empty no matter how the nodes
            // relax.
            let mut w = (wanted * TARGET_ASPECT).sqrt();
            let mut h = wanted / w;
            if w > c.w {
                w = c.w;
                h = wanted / w;
            }
            if h > c.h {
                h = c.h;
                w = wanted / h;
            }
            w = js::min(w, c.w);
            h = js::min(h, c.h);
            Rect::new(c.x + (c.w - w) / 2.0, c.y + (c.h - h) / 2.0, w, h)
        })
        .collect()
}

const MIN_SIBLING_SHARE: f64 = 0.012;
const MIN_SIBLING_AREA: f64 = 60000.0;
const DOMINANT_SHARE: f64 = 0.6;
const BLOCK_ASPECT: f64 = 1.6;
const MAX_SIBLING_ASPECT: f64 = 2.4;

/// Share a rectangle between sibling compartments in proportion to what each
/// has to show.
///
/// `weighted_grid` starts from equal cells and only nudges them, which suits
/// the cells of a community: they are of one kind and of similar size. Sibling
/// compartments are not. A genome-scale alga has a chloroplast with 1300
/// things to draw next to a Golgi with one, and from equal cells the Golgi
/// would come out three quarters the size of the chloroplast.
///
/// Areas follow the weights, with a floor so that a compartment holding a
/// single metabolite is still a box a reaction can be put into. The tiling is
/// a squarified treemap (Bruls, Huizing and van Wijk, 2000), which keeps
/// shapes close to square, with one case taken out of its hands: when one
/// sibling outweighs all the others together, the small ones are gathered
/// into a compact block along one side instead.
///
/// `min_areas`: the least area each sibling can be drawn in.
pub fn pack_compartments(r: &Rect, weights: &[f64], gap: f64, min_areas: &[f64]) -> Vec<Rect> {
    let n = weights.len();
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![*r];
    }
    let area = r.w * r.h;
    let sum = num_or(weights.iter().fold(0.0, |a, b| a + js::max(*b, 0.0)), n as f64);
    // No sibling may be floored into taking the place over: the floors are
    // capped so that together they can never ask for more than the rectangle
    // has.
    let cap = area / (n as f64 * 1.5);
    let wanted: Vec<f64> = weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            js::max(
                (area * js::max(*w, 0.0)) / sum,
                js::min(cap, js::max_n([area * MIN_SIBLING_SHARE, MIN_SIBLING_AREA, min_areas.get(i).copied().unwrap_or(0.0)])),
            )
        })
        .collect();
    let scale = area / wanted.iter().fold(0.0, |a, b| a + b);
    let items: Vec<(usize, f64)> = wanted.iter().enumerate().map(|(i, a)| (i, a * scale)).collect();

    let mut out = vec![Rect::default(); n];
    tile_siblings(*r, items, &mut out);

    out.iter()
        .map(|cell| {
            let b = inset_all(cell, gap / 2.0);
            let (mut w, mut h) = (b.w, b.h);
            if w > h * MAX_SIBLING_ASPECT {
                w = h * MAX_SIBLING_ASPECT;
            } else if h > w * MAX_SIBLING_ASPECT {
                h = w * MAX_SIBLING_ASPECT;
            }
            Rect::new(b.x + (b.w - w) / 2.0, b.y + (b.h - h) / 2.0, w, h)
        })
        .collect()
}

fn tile_siblings(r: Rect, items: Vec<(usize, f64)>, out: &mut [Rect]) {
    if items.len() == 1 {
        out[items[0].0] = r;
        return;
    }
    let mut order = items;
    order.sort_by(|a, b| {
        let d = b.1 - a.1;
        if d != 0.0 && !d.is_nan() {
            return d.partial_cmp(&0.0).unwrap();
        }
        a.0.cmp(&b.0)
    });
    let total = order.iter().fold(0.0, |a, item| a + item.1);
    let largest = order[0];

    if largest.1 > total * DOMINANT_SHARE {
        // The block of small siblings goes along the short side, so the large
        // one keeps the proportions of the rectangle it was given.
        let rest_area = total - largest.1;
        let wide = r.w >= r.h;
        let along = if wide { r.h } else { r.w };
        let mut length = (rest_area * BLOCK_ASPECT).sqrt();
        let mut thickness = rest_area / length;
        if length > along {
            length = along;
            thickness = rest_area / along;
        }
        let offset = (along - length) / 2.0;
        let block = if wide {
            Rect::new(r.x + r.w - thickness, r.y + offset, thickness, length)
        } else {
            Rect::new(r.x + offset, r.y + r.h - thickness, length, thickness)
        };
        out[largest.0] =
            if wide { Rect::new(r.x, r.y, r.w - thickness, r.h) } else { Rect::new(r.x, r.y, r.w, r.h - thickness) };
        tile_siblings(block, order[1..].to_vec(), out);
        return;
    }

    let mut free = r;

    // Worst aspect ratio in a row laid along a side of the given length.
    let worst = |row: &[(usize, f64)], side: f64| -> f64 {
        let row_area = row.iter().fold(0.0, |a, item| a + item.1);
        let big = js::max_n(row.iter().map(|i| i.1));
        let small = js::min_n(row.iter().map(|i| i.1));
        js::max((side * side * big) / (row_area * row_area), (row_area * row_area) / (side * side * small))
    };

    let place = |row: &[(usize, f64)], free: &mut Rect, out: &mut [Rect]| {
        let row_area = row.iter().fold(0.0, |a, item| a + item.1);
        if free.w >= free.h {
            let w = row_area / js::max(free.h, 1e-9);
            let mut y = free.y;
            for item in row {
                let h = item.1 / js::max(w, 1e-9);
                out[item.0] = Rect::new(free.x, y, w, h);
                y += h;
            }
            *free = Rect::new(free.x + w, free.y, js::max(0.0, free.w - w), free.h);
        } else {
            let h = row_area / js::max(free.w, 1e-9);
            let mut x = free.x;
            for item in row {
                let w = item.1 / js::max(h, 1e-9);
                out[item.0] = Rect::new(x, free.y, w, h);
                x += w;
            }
            *free = Rect::new(free.x, free.y + h, free.w, js::max(0.0, free.h - h));
        }
    };

    let mut row: Vec<(usize, f64)> = Vec::new();
    for item in order {
        let side = js::min(free.w, free.h);
        if !row.is_empty() {
            let mut with = row.clone();
            with.push(item);
            if worst(&with, side) > worst(&row, side) {
                place(&row, &mut free, out);
                row.clear();
            }
        }
        row.push(item);
    }
    if !row.is_empty() {
        place(&row, &mut free, out);
    }
}

fn collect_hints(model: &Model, tree: &coxeter_core::compartments::CompartmentTree) -> IndexMap<String, HintRange> {
    let mut per: IndexMap<String, (Vec<f64>, Vec<f64>)> =
        tree.by_id.keys().map(|id| (id.clone(), (Vec::new(), Vec::new()))).collect();
    for rxn in model.reactions.values() {
        let Some(layout) = rxn.layout.as_ref().filter(|l| js::truthy(l)) else { continue };
        for c in &rxn.compartments {
            let Some(bucket) = per.get_mut(c) else { continue };
            bucket.0.push(layout_field(Some(layout), "rank"));
            bucket.1.push(layout_field(Some(layout), "lane"));
        }
    }
    per.into_iter()
        .map(|(id, (ranks, lanes))| {
            let range = HintRange {
                min_rank: if ranks.is_empty() { 0.0 } else { js::min_n(ranks.iter().copied()) },
                max_rank: if ranks.is_empty() { 1.0 } else { js::max_n(ranks.iter().copied()) },
                min_lane: if lanes.is_empty() { 0.0 } else { js::min_n(lanes.iter().copied()) },
                max_lane: if lanes.is_empty() { 1.0 } else { js::max_n(lanes.iter().copied()) },
            };
            (id, range)
        })
        .collect()
}

fn normalise_hint(layout: Option<&Value>, range: Option<HintRange>) -> Option<Hint> {
    let layout = layout.filter(|l| js::truthy(l))?;
    let range = range?;
    let rs = num_or(range.max_rank - range.min_rank, 1.0);
    let ls = num_or(range.max_lane - range.min_lane, 1.0);
    Some(Hint {
        v: (layout_field(Some(layout), "rank") - range.min_rank) / rs,
        u: (layout_field(Some(layout), "lane") - range.min_lane) / ls,
    })
}

fn metabolite_hint(met: &Metabolite, model: &Model, range: Option<HintRange>) -> Option<Hint> {
    let mut valid = Vec::new();
    for rid in &met.reactions {
        let Some(rxn) = model.reactions.get(rid) else { continue };
        if rxn.layout.as_ref().is_some_and(js::truthy) {
            if let Some(h) = normalise_hint(rxn.layout.as_ref(), range) {
                valid.push(h);
            }
        }
    }
    if valid.is_empty() {
        return None;
    }
    let n = valid.len() as f64;
    Some(Hint { v: valid.iter().fold(0.0, |s, h| s + h.v) / n, u: valid.iter().fold(0.0, |s, h| s + h.u) / n })
}

/// `(x, y, Some((anchorX, anchorY)))`.
fn hint_to_point(hint: Option<Hint>, usable: &Rect, rand: &mut crate::force::Random) -> (f64, f64, Option<(f64, f64)>) {
    match hint {
        None => {
            let x = usable.x + rand.next() * usable.w;
            let y = usable.y + rand.next() * usable.h;
            (x, y, None)
        }
        Some(h) => {
            let x = usable.x + h.u * usable.w;
            let y = usable.y + h.v * usable.h;
            (x, y, Some((x, y)))
        }
    }
}

/// The compartment whose membrane a transport reaction crosses.
fn innermost_of(comps: &[String], tree: &coxeter_core::compartments::CompartmentTree) -> String {
    let mut best = comps[0].clone();
    let mut best_depth = -1.0;
    for c in comps {
        let d = tree.get(c).map(|n| n.depth as f64).unwrap_or(0.0);
        if d > best_depth {
            best_depth = d;
            best = c.clone();
        }
    }
    best
}

fn perimeter_param(r: &Rect, p: coxeter_core::geometry::PerimeterPoint) -> f64 {
    let perim = 2.0 * (r.w + r.h);
    let half_top = r.w / 2.0;
    if p.ny < 0.0 && p.y <= r.y + 0.5 {
        let d = p.x - (r.x + r.w / 2.0);
        return (if d >= 0.0 { d } else { perim + d }) / perim;
    }
    if p.nx > 0.0 {
        return (half_top + (p.y - r.y)) / perim;
    }
    if p.ny > 0.0 {
        return (half_top + r.h + (right(r) - p.x)) / perim;
    }
    (half_top + r.h + r.w + (bottom(r) - p.y)) / perim
}

fn min_spacing(r: &Rect, px: f64) -> f64 {
    let perim = 2.0 * (r.w + r.h);
    px / js::max(perim, 1.0)
}

/// Slide `t` along the perimeter until it clears every already-used slot.
fn spread_along_perimeter(t: f64, used: &[f64], spacing: f64) -> f64 {
    let mut candidate = ((t % 1.0) + 1.0) % 1.0;
    for attempt in 0..200 {
        let clash = used.iter().any(|u| circular_distance(*u, candidate) < spacing);
        if !clash {
            return candidate;
        }
        let step = spacing * (if attempt % 2 == 0 { 1.0 } else { -1.0 }) * ((attempt / 2) as f64 + 1.0);
        candidate = ((t + step) % 1.0 + 1.0) % 1.0;
    }
    candidate
}

fn circular_distance(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % 1.0;
    js::min(d, 1.0 - d)
}
