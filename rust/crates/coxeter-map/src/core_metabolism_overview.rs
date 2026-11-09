//! Minimal pedagogical glycolysis overview (port of
//! `src/map/core-metabolism-overview.js`).
//!
//! The Core metabolism view intentionally shows only glycolysis, arranged as
//! a vertical pathway from top to bottom. Every cell uses the same normalised
//! coordinates so strain-to-strain comparison is immediate and uncluttered.

use coxeter_core::community::{CommunityModel, namespaced};
use coxeter_core::js;
use coxeter_core::mapdata::{AxisEnd, MapCompartment, Point, Rect, SegmentKind};
use coxeter_core::model::Reaction;

use crate::editable_map::{EditableMap, SegEnd, TextOptions};
use crate::layout_settings::{RawLayoutSettings, layout_engine_options};
use crate::place_reaction::PlaceOptions;
use crate::util::lerp;

/// One group of the overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewGroup {
    pub id: &'static str,
    pub label: &'static str,
    pub reactions: &'static [&'static str],
}

const GLYCOLYSIS_REACTIONS: [&str; 9] = ["PGI", "PFK", "FBA", "TPI", "GAPD", "PGK", "PGM", "ENO", "PYK"];

pub const CORE_OVERVIEW_GROUPS: [OverviewGroup; 1] =
    [OverviewGroup { id: "glycolysis", label: "Glycolysis", reactions: &GLYCOLYSIS_REACTIONS }];

/// The canonical top-to-bottom direction of each step.
const REACTION_FLOW: [(&str, [&str; 2]); 9] = [
    ("PGI", ["g6p_c", "f6p_c"]),
    ("PFK", ["f6p_c", "fdp_c"]),
    ("FBA", ["fdp_c", "g3p_c"]),
    ("TPI", ["dhap_c", "g3p_c"]),
    ("GAPD", ["g3p_c", "13dpg_c"]),
    ("PGK", ["13dpg_c", "3pg_c"]),
    ("PGM", ["3pg_c", "2pg_c"]),
    ("ENO", ["2pg_c", "pep_c"]),
    ("PYK", ["pep_c", "pyr_c"]),
];

/// Fixed vertical topology: `(local id, x, y, landmark)`.
///
/// FBA branches FBP into DHAP and G3P. TPI returns DHAP to G3P, after which
/// the pathway continues vertically to pyruvate.
const METABOLITE_TEMPLATE: [(&str, f64, f64, bool); 10] = [
    ("g6p_c", 0.50, 0.10, true),
    ("f6p_c", 0.50, 0.19, true),
    ("fdp_c", 0.50, 0.28, true),
    ("dhap_c", 0.36, 0.38, false),
    ("g3p_c", 0.50, 0.42, true),
    ("13dpg_c", 0.50, 0.52, false),
    ("3pg_c", 0.50, 0.61, true),
    ("2pg_c", 0.50, 0.70, false),
    ("pep_c", 0.50, 0.79, true),
    ("pyr_c", 0.50, 0.89, true),
];

const LABELS: [(&str, f64, f64); 10] = [
    ("G6P", 0.535, 0.10),
    ("F6P", 0.535, 0.19),
    ("FBP", 0.535, 0.28),
    ("DHAP", 0.315, 0.38),
    ("G3P", 0.535, 0.42),
    ("1,3-BPG", 0.535, 0.52),
    ("3PG", 0.535, 0.61),
    ("2PG", 0.535, 0.70),
    ("PEP", 0.535, 0.79),
    ("PYR", 0.535, 0.89),
];

/// `coreOverviewSupport` result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverviewSupport {
    pub supported: bool,
    pub coverage: f64,
    pub groups: usize,
}

/// `coreOverviewSupport(community)`: can the overview be drawn for this
/// community? Three quarters of the glycolysis steps have to be there.
pub fn core_overview_support(community: &CommunityModel) -> OverviewSupport {
    let active: Vec<_> = community.active_species().collect();
    if active.is_empty() {
        return OverviewSupport { supported: false, coverage: 0.0, groups: 0 };
    }
    let mut hits = 0.0;
    let mut total = 0.0;
    for sp in &active {
        for id in GLYCOLYSIS_REACTIONS {
            total += 1.0;
            if sp.model.reactions.contains_key(id) {
                hits += 1.0;
            }
        }
    }
    let coverage = if total != 0.0 { hits / total } else { 0.0 };
    OverviewSupport { supported: coverage >= 0.75, coverage, groups: active.len() }
}

/// One drawn pathway group of one cell.
#[derive(Debug, Clone, PartialEq)]
pub struct PathwayGroup {
    pub id: String,
    pub species_id: String,
    pub group_id: String,
    pub name: String,
    pub reaction_ids: Vec<String>,
}

/// `buildCoreMetabolismOverview` result.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreOverview {
    pub map: EditableMap,
    pub pathway_groups: Vec<PathwayGroup>,
    pub placed: usize,
    /// `(species id, [(group id, count)])`.
    pub per_cell: Vec<(String, Vec<(String, usize)>)>,
    pub support: OverviewSupport,
}

/// `buildCoreMetabolismOverview(community, layoutSettings)`.
pub fn build_core_metabolism_overview(community: &CommunityModel, layout_settings: &RawLayoutSettings) -> CoreOverview {
    let mut engine = layout_engine_options(layout_settings);
    let or1 = |v: Option<f64>, fallback: f64| crate::util::num_or(v.unwrap_or(f64::NAN), fallback);
    engine.cell_area_multiplier = Some(js::max(6.0, or1(engine.cell_area_multiplier, 1.0)));
    engine.node_spacing = Some(js::max(1.15, or1(engine.node_spacing, 1.0)));
    engine.cell_gap = Some(js::max(220.0, or1(engine.cell_gap, 170.0)));

    let mut map = EditableMap::empty_from_community(community, &engine);
    let mut pathway_groups = Vec::new();
    let mut per_cell = Vec::new();
    let mut placed = 0;

    for sp in community.active_species() {
        let Some(cell) = map.cell(&sp.id) else { continue };
        let cytosol = cell.compartments.iter().find(|c| c.id == "c").or_else(|| largest_compartment(&cell.compartments));
        let Some(rect) = cytosol.map(|c| c.rect) else { continue };

        let mut reaction_ids = Vec::new();
        for local_id in GLYCOLYSIS_REACTIONS {
            if !sp.model.reactions.contains_key(local_id) {
                continue;
            }
            let reaction_id = namespaced(&sp.id, local_id);
            let opts = PlaceOptions { anchor: Some(normalized_point(&rect, 0.50, 0.50)), ..Default::default() };
            if map.add_reaction(community, &reaction_id, &opts).is_none() {
                continue;
            }
            reaction_ids.push(reaction_id);
            placed += 1;
        }

        apply_metabolite_template(&mut map, &sp.id, &rect);
        place_reactions_between_landmarks(&mut map, community, &sp.id);

        let title = normalized_point(&rect, 0.50, 0.045);
        map.add_text(
            "GLYCOLYSIS",
            title.x,
            title.y,
            &TextOptions { id: Some(format!("overview:{}:glycolysis", sp.id)), font_size: Some(28.0), font_family: None },
        );
        for (text, x, y) in LABELS {
            let q = normalized_point(&rect, x, y);
            map.add_text(text, q.x, q.y, &TextOptions { id: Some(format!("overview:{}:met:{text}", sp.id)), font_size: Some(16.0), font_family: None });
        }

        per_cell.push((sp.id.clone(), vec![("glycolysis".to_string(), reaction_ids.len())]));
        pathway_groups.push(PathwayGroup {
            id: format!("{}-glycolysis", sp.id),
            species_id: sp.id.clone(),
            group_id: "glycolysis".into(),
            name: "Glycolysis".into(),
            reaction_ids,
        });
    }

    CoreOverview { map, pathway_groups, placed, per_cell, support: core_overview_support(community) }
}

fn apply_metabolite_template(map: &mut EditableMap, species_id: &str, rect: &Rect) {
    for (local_id, x, y, landmark) in METABOLITE_TEMPLATE {
        let merged = namespaced(species_id, local_id);
        let nodes: Vec<String> = map.nodes_for_metabolite(&merged).iter().map(|n| n.id.clone()).collect();
        if nodes.is_empty() {
            continue;
        }
        let q = normalized_point(rect, x, y);
        for id in nodes {
            map.move_node(&id, q.x, q.y);
            if landmark {
                let r = map.node(&id).map(|n| n.r).unwrap_or(13.0);
                map.set_node_radius(&id, js::max(r, 16.0));
            }
        }
    }
}

fn place_reactions_between_landmarks(map: &mut EditableMap, community: &CommunityModel, species_id: &str) {
    for local_id in GLYCOLYSIS_REACTIONS {
        let reaction_id = namespaced(species_id, local_id);
        let Some(merged) = community.merged.reactions.get(&reaction_id) else { continue };
        if !map.has_reaction(&reaction_id) {
            continue;
        }
        let participants = semantic_participants(species_id, merged);
        let nodes: Vec<Point> = participants
            .iter()
            .flat_map(|(local, _)| map.nodes_for_metabolite(&namespaced(species_id, local)).into_iter().map(|n| n.pos()).collect::<Vec<_>>())
            .collect();
        if nodes.is_empty() {
            continue;
        }
        let q = centroid(&nodes);
        map.move_reaction(&reaction_id, q.x, q.y);
        orient_reaction(map, &reaction_id, local_id, species_id, &participants);
    }
}

fn semantic_participants(species_id: &str, reaction: &Reaction) -> Vec<(String, f64)> {
    let prefix = format!("{species_id}@");
    let mut out = Vec::new();
    for (merged_id, &coef) in &reaction.metabolites {
        let local = merged_id.strip_prefix(&prefix).unwrap_or(merged_id);
        if !METABOLITE_TEMPLATE.iter().any(|t| t.0 == local) {
            continue;
        }
        out.push((local.to_string(), coef));
    }
    out
}

fn orient_reaction(map: &mut EditableMap, reaction_id: &str, local_reaction_id: &str, species_id: &str, semantic: &[(String, f64)]) {
    let Some(rxn) = map.reaction(reaction_id) else { return };
    let Some(axis) = rxn.axis else { return };
    if semantic.len() < 2 {
        return;
    }
    let cell_id = rxn.cell_id.clone().unwrap_or_default();
    let (rx, ry) = (rxn.x, rxn.y);

    // Reversible GEM reactions are not guaranteed to be stored in the
    // intuitive glycolytic direction (PGK and PGM are commonly written
    // backwards). The pedagogical overview therefore uses a canonical
    // top-to-bottom pathway direction rather than raw stoichiometric sign.
    let mut ends: Option<(Point, Point)> = None;
    if let Some((_, [from, to])) = REACTION_FLOW.iter().find(|(id, _)| *id == local_reaction_id) {
        let start = map.nodes_for_metabolite(&namespaced(species_id, from)).first().map(|n| n.pos());
        let end = map.nodes_for_metabolite(&namespaced(species_id, to)).first().map(|n| n.pos());
        if let (Some(a), Some(b)) = (start, end) {
            ends = Some((a, b));
        }
    }
    let (a, b) = match ends {
        Some(e) => e,
        None => {
            let side = |want_sub: bool| -> Vec<Point> {
                semantic
                    .iter()
                    .filter(|(_, c)| if want_sub { *c < 0.0 } else { *c > 0.0 })
                    .flat_map(|(local, _)| map.nodes_for_metabolite(&namespaced(&cell_id, local)).into_iter().map(|n| n.pos()).collect::<Vec<_>>())
                    .collect()
            };
            let subs = side(true);
            let prods = side(false);
            if subs.is_empty() || prods.is_empty() {
                return;
            }
            (centroid(&subs), centroid(&prods))
        }
    };
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let norm = js::hypot(dx, dy);
    if norm < 1e-6 {
        return;
    }
    let dir = Point::new(dx / norm, dy / norm);
    let half = js::max(10.0, js::hypot(axis.a2.x - axis.a1.x, axis.a2.y - axis.a1.y) / 2.0);

    map.g.reactions[reaction_id].axis.as_mut().expect("checked").dir = dir;
    map.replace_axis_point(reaction_id, AxisEnd::A1, Point::new(rx - dir.x * half, ry - dir.y * half));
    map.replace_axis_point(reaction_id, AxisEnd::A2, Point::new(rx + dir.x * half, ry + dir.y * half));
    let axis = map.g.reactions[reaction_id].axis.expect("checked");

    for i in 0..map.g.segments.len() {
        if map.g.segments[i].reaction_id != reaction_id {
            continue;
        }
        if map.g.segments[i].kind == SegmentKind::Axis {
            map.alias_seg(i, SegEnd::From, AxisEnd::A1);
            map.alias_seg(i, SegEnd::To, AxisEnd::A2);
            continue;
        }
        let seg = &map.g.segments[i];
        let Some(node_id) = seg.node_id.clone().filter(|n| !n.is_empty()) else { continue };
        if seg.side.is_none() {
            continue;
        }
        let Some(at) = map.g.nodes.get(&node_id).map(|n| n.pos()) else { continue };
        let substrate = seg.is_substrate();
        let end = if substrate { axis.a1 } else { axis.a2 };
        if substrate {
            map.set_seg(i, SegEnd::From, at);
            map.alias_seg(i, SegEnd::To, AxisEnd::A1);
        } else {
            map.alias_seg(i, SegEnd::From, AxisEnd::A2);
            map.set_seg(i, SegEnd::To, at);
        }
        let seg = &mut map.g.segments[i];
        seg.b1 = Some(lerp(at, end, 0.34));
        seg.b2 = Some(lerp(at, end, 0.72));
        seg.extra.shift_remove("control");
    }
}

fn normalized_point(rect: &Rect, x: f64, y: f64) -> Point {
    Point::new(rect.x + rect.w * x, rect.y + rect.h * y)
}

fn centroid(points: &[Point]) -> Point {
    let n = points.len() as f64;
    Point::new(points.iter().fold(0.0, |s, q| s + q.x) / n, points.iter().fold(0.0, |s, q| s + q.y) / n)
}

/// The compartment with the largest box (the first of equals, as a stable
/// sort leaves it).
fn largest_compartment(compartments: &[MapCompartment]) -> Option<&MapCompartment> {
    let mut best: Option<&MapCompartment> = None;
    for c in compartments {
        match best {
            Some(b) if c.rect.w * c.rect.h <= b.rect.w * b.rect.h => {}
            _ => best = Some(c),
        }
    }
    best
}
