//! Incremental placement of a single reaction onto an existing map (port of
//! `src/layout/place-reaction.js`).
//!
//! This module never calls the automatic layout, never solves FBA and never
//! moves geometry that was already on the map.

use std::collections::HashSet;

use coxeter_core::boundaries::reaction_boundary_keys;
use coxeter_core::community::{CommunityModel, LINK_COMPARTMENT, SHARED_COMPARTMENT, split_id};
use coxeter_core::geometry::{SIZES, center};
use coxeter_core::js;
use coxeter_core::mapdata::{Axis, Bounds, MapReaction, Node, NodeKind, Point, Rect, Segment, SegmentKind, TransportLane};
use coxeter_core::model::{Metabolite, Reaction, strip_compartment};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::editable_map::{Boundary, EditableMap};
use crate::reaction_edit::{closest_boundary_point, satellite_place, shape_branch};
use crate::util::{blank_node, blank_reaction, blank_segment, point_from_value, point_value};

/// Which way a reaction runs when nothing says otherwise: down the page.
const BASE_DIRECTION: Point = Point::new(0.0, 1.0);

/// How far from the marker the metabolite on the backbone's line stands.
const PRIMARY_DISTANCE: f64 = 72.0;

/// Escher-style secondary/currency metabolites. These are duplicated locally
/// beside each internal reaction instead of being shared as long-range hubs.
pub const ESCHER_SECONDARY_BASES: [&str; 12] =
    ["nad", "nadh", "nadp", "nadph", "h", "h2o", "co2", "coa", "atp", "adp", "amp", "pi"];

/// Options of `placeReaction` / `EditableMap::add_reaction`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaceOptions {
    /// Start from this metabolite node already on the map.
    pub anchor_node_id: Option<String>,
    /// Put a transporter on this boundary, if it crosses it.
    pub boundary_id: Option<String>,
    /// Where the user clicked.
    pub anchor: Option<Point>,
    /// Which way the new reaction should appear from the anchor node.
    pub placement_direction: Option<Point>,
    /// The metabolite the backbone runs straight to.
    pub terminal_metabolite_id: Option<String>,
}

/// What `placeReaction` added.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub reaction_id: String,
    /// Node ids added (the primary instances; satellites are in `satellites`).
    pub nodes: Vec<String>,
    /// Cofactor nodes created beside the reaction.
    pub satellites: Vec<String>,
    /// How many segments were appended to the map.
    pub segments: usize,
    pub anchor_node: Option<String>,
}

/// `besideBackbone(index)`: where the `index`-th further primary metabolite
/// of a side stands: off to alternate hands of the backbone, starting on the
/// hand the cofactors do not start on, each pair a little wider and a little
/// nearer the marker.
fn beside_backbone(index: usize) -> (f64, f64) {
    let row = (index / 2) as f64 + 1.0;
    let hand = if index % 2 == 0 { -1.0 } else { 1.0 };
    (js::max(40.0, PRIMARY_DISTANCE - 10.0 * row), hand * 62.0 * row)
}

/// `isEscherSecondaryMetabolite`.
pub fn is_escher_secondary_metabolite(community: &CommunityModel, merged_id: &str, metabolite: Option<&Metabolite>) -> bool {
    let Some(met) = metabolite.or_else(|| community.merged.metabolites.get(merged_id)) else { return false };
    let base = local_metabolite_base(community, merged_id, Some(met));
    ESCHER_SECONDARY_BASES.contains(&base.to_lowercase().as_str())
}

/// `localMetaboliteBase`: what a metabolite is called inside its own species,
/// that is the id without the species in front and the compartment behind.
/// This is the label of a node, and what groups the two ends of a
/// transported metabolite into one lane.
pub fn local_metabolite_base(community: &CommunityModel, merged_id: &str, metabolite: Option<&Metabolite>) -> String {
    let owner = community.metabolite_owner.get(merged_id);
    // A shared-medium metabolite is recorded under its base already; running
    // it through the compartment stripper would cut a real suffix off `pe_hs`.
    if let Some(o) = owner {
        if o.species_id.is_none() && o.compartment == SHARED_COMPARTMENT {
            return o.original_id.clone();
        }
    }
    let compartment = owner.map(|o| o.compartment.clone()).or_else(|| metabolite.map(|m| m.compartment.clone())).unwrap_or_default();
    let local = owner.map(|o| o.original_id.clone()).unwrap_or_else(|| split_id(merged_id).1.to_string());
    strip_compartment(&local, &compartment)
}

/// The shared medium is the compartment `s` that belongs to no cell. The id
/// alone does not say so: BiGG gives the eyespot of an alga the letter `s`
/// too, and a metabolite there lives in the cell's own `s` box.
fn is_shared_place(cell_id: Option<&str>, compartment: Option<&str>) -> bool {
    cell_id.is_none() && compartment == Some(SHARED_COMPARTMENT)
}

struct MetPlace {
    compartment: Option<String>,
    shared: bool,
    cell_id: Option<String>,
}

/// Where one merged metabolite sits: its compartment and, unless shared, its cell.
fn metabolite_place(community: &CommunityModel, merged_id: &str, met: Option<&Metabolite>, fallback: Option<&str>) -> MetPlace {
    let owner = community.metabolite_owner.get(merged_id);
    let compartment = owner.map(|o| o.compartment.clone()).or_else(|| met.map(|m| m.compartment.clone()));
    let shared = match owner {
        Some(o) => is_shared_place(o.species_id.as_deref(), compartment.as_deref()),
        None => compartment.as_deref() == Some(SHARED_COMPARTMENT),
    };
    let cell_id = if shared { None } else { owner.and_then(|o| o.species_id.clone()).or_else(|| fallback.map(str::to_string)) };
    MetPlace { compartment, shared, cell_id }
}

fn normalize_direction(d: Point) -> Point {
    let fix = |v: f64| if v == 0.0 { 0.0 } else { v };
    Point::new(fix(d.x), fix(d.y))
}

fn unit_direction(dir: Option<Point>) -> Option<Point> {
    let d = dir?;
    if !d.x.is_finite() || !d.y.is_finite() {
        return None;
    }
    let length = js::hypot(d.x, d.y);
    if length < 1e-9 {
        return None;
    }
    Some(normalize_direction(Point::new(d.x / length, d.y / length)))
}

struct Place {
    compartment: Option<String>,
    cell_id: Option<String>,
}

fn reaction_metabolites(community: &CommunityModel, reaction: &Reaction, fallback: Option<&str>) -> Vec<Place> {
    let mut out = Vec::new();
    for mid in reaction.metabolites.keys() {
        let Some(met) = community.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        let p = metabolite_place(community, mid, Some(met), fallback);
        out.push(Place { compartment: p.compartment, cell_id: p.cell_id });
    }
    out
}

fn choose_internal_place(places: &[Place]) -> Option<&Place> {
    places
        .iter()
        .find(|p| !is_shared_place(p.cell_id.as_deref(), p.compartment.as_deref()))
        .or_else(|| places.first())
}

fn find_compartment_rect(map: &EditableMap, cell_id: Option<&str>, compartment: Option<&str>) -> Option<Rect> {
    if is_shared_place(cell_id, compartment) {
        return Some(map.g.environment.rect);
    }
    let cell = map.g.cells.iter().find(|c| Some(c.id.as_str()) == cell_id)?;
    cell.compartments.iter().find(|c| Some(c.id.as_str()) == compartment).map(|c| c.rect)
}

fn clamp_inside_rect(p: Point, r: Option<Rect>, padding: f64) -> Point {
    let Some(r) = r else { return p };
    Point::new(
        js::min(r.x + r.w - padding, js::max(r.x + padding, p.x)),
        js::min(r.y + r.h - padding, js::max(r.y + padding, p.y)),
    )
}

struct Placement {
    x: f64,
    y: f64,
    on_membrane: bool,
    membrane: Option<String>,
    boundary_id: Option<String>,
    normal: Option<Point>,
}

fn choose_reaction_position(
    map: &EditableMap,
    cell_id: Option<&str>,
    places: &[Place],
    boundary_id: Option<&str>,
    anchor: Option<Point>,
) -> Option<Placement> {
    if let Some(bid) = boundary_id {
        let boundary = map.boundary(bid).filter(|b| b.rect.is_some())?;
        let p = closest_boundary_point(map, boundary, anchor.unwrap_or_else(|| center(&boundary.rect.expect("filtered"))));
        return Some(Placement {
            x: p.x,
            y: p.y,
            on_membrane: true,
            membrane: Some(boundary.inside.clone()),
            boundary_id: Some(boundary.id.clone()),
            normal: Some(Point::new(p.nx, p.ny)),
        });
    }
    let place = choose_internal_place(places)?;
    let rect_cell = if is_shared_place(place.cell_id.as_deref(), place.compartment.as_deref()) {
        None
    } else {
        place.cell_id.as_deref().or(cell_id)
    };
    let r = find_compartment_rect(map, rect_cell, place.compartment.as_deref())?;
    let raw = anchor.unwrap_or_else(|| center(&r));
    let p = clamp_inside_rect(raw, Some(r), SIZES.marker_radius + 14.0);
    Some(Placement { x: p.x, y: p.y, on_membrane: false, membrane: None, boundary_id: None, normal: None })
}

fn choose_boundary(map: &EditableMap, crossed: &[String], anchor: Option<Point>) -> Option<String> {
    let mut candidates: Vec<&Boundary> = crossed.iter().filter_map(|id| map.boundary(id)).filter(|b| b.rect.is_some()).collect();
    if candidates.is_empty() {
        return None;
    }
    let depth_order = |a: &Boundary, b: &Boundary| b.depth - a.depth;
    let as_ordering = |d: f64| if d < 0.0 { std::cmp::Ordering::Less } else if d > 0.0 { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal };
    if let Some(anchor) = anchor {
        candidates.sort_by(|a, b| {
            let pa = closest_boundary_point(map, a, anchor);
            let pb = closest_boundary_point(map, b, anchor);
            let da = js::pow(pa.x - anchor.x, 2.0) + js::pow(pa.y - anchor.y, 2.0);
            let db = js::pow(pb.x - anchor.x, 2.0) + js::pow(pb.y - anchor.y, 2.0);
            let d = da - db;
            if d != 0.0 && !d.is_nan() {
                return as_ordering(d);
            }
            as_ordering(depth_order(a, b))
        });
        return Some(candidates[0].id.clone());
    }
    candidates.sort_by(|a, b| as_ordering(depth_order(a, b)));
    Some(candidates[0].id.clone())
}

fn selected_reaction_anchor<'m>(map: &'m EditableMap, reaction: &Reaction, anchor_node_id: Option<&str>) -> Option<&'m Node> {
    let id = anchor_node_id.filter(|s| !s.is_empty())?;
    let node = map.g.nodes.get(id)?;
    let merged = node.merged_id.as_deref().filter(|m| !m.is_empty())?;
    if !matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium | NodeKind::Currency) {
        return None;
    }
    reaction.metabolites.contains_key(merged).then_some(node)
}

fn reaction_visual_node(map: &EditableMap, reaction_id: &str, merged_id: &str) -> Option<String> {
    map.g
        .nodes
        .values()
        .find(|n| {
            matches!(n.kind, NodeKind::Metabolite | NodeKind::Medium)
                && n.reaction_local_for.as_deref() == Some(reaction_id)
                && n.merged_id.as_deref() == Some(merged_id)
        })
        .map(|n| n.id.clone())
}

fn next_reaction_node_id(nodes: &IndexMap<String, Node>, reaction_id: &str, merged_id: &str) -> String {
    let root = format!("met:{reaction_id}:{merged_id}");
    if !nodes.contains_key(&root) {
        return root;
    }
    let mut index = 2;
    let mut id = format!("{root}#{index}");
    while nodes.contains_key(&id) {
        index += 1;
        id = format!("{root}#{index}");
    }
    id
}

/// `placeReaction(map, community, reactionId, opts)`: add one biological
/// reaction to the map without rebuilding or solving. Every metabolite is
/// created as a reaction-local visual instance; existing copies are never
/// re-used automatically (the user may merge copies by drag), except the one
/// node the user started from.
pub fn place_reaction(map: &mut EditableMap, community: &CommunityModel, reaction_id: &str, opts: &PlaceOptions) -> Option<Placed> {
    if map.g.reactions.contains_key(reaction_id) {
        return None;
    }
    let model_reaction = community.merged.reactions.get(reaction_id)?;

    // Add Reaction mode may start from one already-drawn metabolite. That one
    // explicit node is the only visual instance we are allowed to reuse; every
    // other metabolite keeps the normal "fresh instance" insertion rule.
    let anchor_node = selected_reaction_anchor(map, model_reaction, opts.anchor_node_id.as_deref()).cloned();

    let owner = community.reaction_owner.get(reaction_id);
    let cell_id: Option<String> =
        owner.and_then(|o| o.species_id.clone()).or_else(|| split_id(reaction_id).0.map(str::to_string));
    let places = reaction_metabolites(community, model_reaction, cell_id.as_deref());
    if places.is_empty() {
        return None;
    }

    let crossed = reaction_boundary_keys(community, reaction_id, None);
    let requested = opts.boundary_id.as_ref().filter(|b| !b.is_empty() && crossed.contains(b)).cloned();
    let boundary_id = requested.or_else(|| choose_boundary(map, &crossed, opts.anchor));

    let placement = choose_reaction_position(map, cell_id.as_deref(), &places, boundary_id.as_deref(), opts.anchor)?;

    let placement_direction = unit_direction(opts.placement_direction);
    let anchor_coefficient = match &anchor_node {
        Some(n) => model_reaction.metabolites.get(n.merged_id.as_deref().unwrap_or("")).copied().unwrap_or(f64::NAN),
        None => 0.0,
    };
    // placementDirection says where the new reaction should appear relative
    // to the selected metabolite. The biochemical axis must flip when that
    // selected metabolite is a product so its branch still begins at the
    // chosen node.
    let directed = match (placement_direction, &anchor_node) {
        (Some(d), Some(_)) if anchor_coefficient > 0.0 => Some(Point::new(-d.x, -d.y)),
        _ => placement_direction,
    };
    // With nothing to say which way it should run, a reaction that is not
    // being attached to a metabolite already on the map is drawn in its base
    // shape: straight down the page, substrate above the marker and product
    // below.
    let axis_direction_hint =
        directed.or(if anchor_node.is_some() || placement.on_membrane { None } else { Some(BASE_DIRECTION) });

    let mut r = blank_reaction(reaction_id);
    r.local_id = Some(owner.map(|o| o.original_id.clone()).unwrap_or_else(|| split_id(reaction_id).1.to_string()));
    r.name = model_reaction.name.clone();
    r.subsystem = Some(model_reaction.subsystem.clone());
    r.cell_id = cell_id.clone();
    r.kind = owner.map(|o| o.role.as_str().to_string()).unwrap_or_else(|| model_reaction.kind.as_str().to_string());
    // The model always lists them (JS falls back to the places only for a
    // reaction object that has no `compartments`).
    r.compartments = model_reaction.compartments.clone();
    r.membrane = placement.membrane.clone();
    r.boundary_id = placement.boundary_id.clone();
    r.on_membrane = placement.on_membrane;
    r.normal = placement.normal;
    r.placement_direction = Some(placement_direction.map(point_value).unwrap_or(Value::Null));
    r.axis_direction_hint = axis_direction_hint;
    r.x = placement.x;
    r.y = placement.y;
    r.equation = Some(model_reaction.equation());
    r.terminal_metabolite_id = Some(opts.terminal_metabolite_id.clone().map(Value::String).unwrap_or(Value::Null));
    r.bounds = Some(Bounds { lower: model_reaction.lower_bound, upper: model_reaction.upper_bound });

    map.g.reactions.insert(reaction_id.to_string(), r);
    let added = ensure_reaction_nodes(map, community, reaction_id, model_reaction, anchor_node.as_ref());
    let before_nodes: HashSet<String> = map.g.nodes.keys().cloned().collect();
    let (segments, links) = build_single_reaction_segments(map, community, reaction_id, opts.anchor_node_id.as_deref());
    let satellites: Vec<String> = map.g.nodes.keys().filter(|k| !before_nodes.contains(*k)).cloned().collect();
    let count = segments.len();
    for (seg, link) in segments.into_iter().zip(links) {
        map.push_segment(seg, link);
    }
    if let Some(a) = &anchor_node {
        if let Some(n) = map.g.nodes.get_mut(&a.id) {
            n.shared_instance = Some(true);
        }
    }
    Some(Placed { reaction_id: reaction_id.to_string(), nodes: added, satellites, segments: count, anchor_node: anchor_node.map(|n| n.id) })
}

fn ensure_reaction_nodes(
    map: &mut EditableMap,
    community: &CommunityModel,
    reaction_id: &str,
    model_reaction: &Reaction,
    anchor_node: Option<&Node>,
) -> Vec<String> {
    let mut added = Vec::new();
    let mut index = 0usize;
    let reaction = map.g.reactions[reaction_id].clone();
    let lateral_by_base = membrane_lateral_offsets(community, &reaction, model_reaction);
    let mut beside = [0usize, 0usize]; // per side: substrate, product
    let terminal = reaction.terminal_metabolite_id.as_ref().and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);

    for (mid, &coefficient) in &model_reaction.metabolites {
        let Some(met) = community.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        // The metabolite explicitly selected in Add Reaction mode is the
        // single exception to duplication: the new reaction must start from
        // that exact visual node.
        if anchor_node.and_then(|a| a.merged_id.as_deref()) == Some(mid.as_str()) {
            continue;
        }
        // Internal Escher-style secondary metabolites are reaction-local
        // satellites.
        if !reaction.on_membrane && is_escher_secondary_metabolite(community, mid, Some(met)) {
            continue;
        }

        let primary_id = next_reaction_node_id(&map.g.nodes, reaction_id, mid);
        let owner = community.metabolite_owner.get(mid);
        let place = metabolite_place(community, mid, Some(met), reaction.cell_id.as_deref());
        let base = local_metabolite_base(community, mid, Some(met));
        // The first primary metabolite on each side stands on the backbone's
        // own line, which is what makes a reaction read as substrate, marker,
        // product. The rest of that side stand off to either hand of it.
        let axial = !reaction.on_membrane
            && is_axial_primary(community, model_reaction, mid, coefficient, anchor_node, terminal.as_deref());
        let mut offset = None;
        if !reaction.on_membrane && !axial {
            let side = if coefficient < 0.0 { 0 } else { 1 };
            offset = Some(beside_backbone(beside[side]));
            beside[side] += 1;
        }
        let position = position_new_metabolite(
            map,
            &reaction,
            place.compartment.as_deref(),
            place.cell_id.as_deref(),
            coefficient,
            index,
            lateral_by_base.get(&base).copied(),
            axial,
            offset,
        );

        let mut node = blank_node(&primary_id, if place.shared { NodeKind::Medium } else { NodeKind::Metabolite });
        node.merged_id = Some(mid.clone());
        node.local_id = Some(owner.map(|o| o.original_id.clone()).unwrap_or_else(|| met.id.clone()));
        // The label. Taken from the species' own id: the merged one carries
        // the species in front, and the node would be named `eco@g6p`.
        node.base = Some(base);
        node.name = Some(met.name.clone());
        node.formula = Some(met.formula.clone());
        node.compartment = place.compartment.clone();
        node.cell_id = place.cell_id.clone();
        node.reaction_local_for = Some(reaction_id.to_string());
        node.reaction_local = Some(true);
        node.x = position.x;
        node.y = position.y;
        node.r = if place.shared { SIZES.medium_radius } else { SIZES.primary_radius };
        node.primary = Some(true);
        node.visual_role = Some("primary".into());
        map.g.nodes.insert(primary_id.clone(), node);

        if place.shared && !map.g.medium.nodes.contains(&primary_id) {
            map.g.medium.nodes.push(primary_id.clone());
        }
        added.push(primary_id);
        index += 1;
    }
    added
}

#[allow(clippy::too_many_arguments)]
fn position_new_metabolite(
    map: &EditableMap,
    reaction: &MapReaction,
    compartment: Option<&str>,
    cell_id: Option<&str>,
    coefficient: f64,
    index: usize,
    lateral_override: Option<f64>,
    axial: bool,
    offset: Option<(f64, f64)>,
) -> Point {
    let dir = reaction
        .normal
        .or(reaction.axis_direction_hint)
        .or_else(|| reaction.placement_direction.as_ref().and_then(point_from_value))
        .unwrap_or(Point::new(0.0, 1.0));
    // On a membrane, geometry follows biological location rather than the
    // sign of the stoichiometric coefficient: inside is opposite the outward
    // normal and outside is along it. Internal reactions keep the plain
    // substrate/product split.
    let side = if reaction.on_membrane {
        if is_inside_boundary(map, reaction, cell_id, compartment) { -1.0 } else { 1.0 }
    } else if coefficient < 0.0 {
        -1.0
    } else {
        1.0
    };
    let perp = Point::new(-dir.y, dir.x);
    let distance = if reaction.on_membrane { 42.0 } else { offset.map(|o| o.0).unwrap_or(PRIMARY_DISTANCE) };
    let lateral = if axial {
        0.0
    } else {
        offset.map(|o| o.1).or(lateral_override).unwrap_or(((index % 3) as f64 - 1.0) * 34.0)
    };
    let point = Point::new(
        reaction.x + dir.x * side * distance + perp.x * lateral,
        reaction.y + dir.y * side * distance + perp.y * lateral,
    );
    if is_shared_place(cell_id, compartment) {
        return clamp_inside_rect(point, Some(map.g.environment.rect), SIZES.medium_radius + 8.0);
    }
    match find_compartment_rect(map, cell_id, compartment) {
        Some(r) => clamp_inside_rect(point, Some(r), SIZES.primary_radius + 8.0),
        None => point,
    }
}

fn is_axial_primary(
    community: &CommunityModel,
    model_reaction: &Reaction,
    target_mid: &str,
    target_coefficient: f64,
    anchor_node: Option<&Node>,
    preferred: Option<&str>,
) -> bool {
    if let Some(p) = preferred {
        return target_mid == p;
    }
    let target_side = if target_coefficient < 0.0 { -1 } else { 1 };
    for (mid, &coefficient) in &model_reaction.metabolites {
        if (if coefficient < 0.0 { -1 } else { 1 }) != target_side {
            continue;
        }
        if anchor_node.and_then(|a| a.merged_id.as_deref()) == Some(mid.as_str()) {
            return false;
        }
        let Some(met) = community.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        if is_escher_secondary_metabolite(community, mid, Some(met)) {
            continue;
        }
        return mid == target_mid;
    }
    false
}

fn membrane_lateral_offsets(community: &CommunityModel, reaction: &MapReaction, model_reaction: &Reaction) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    if !reaction.on_membrane {
        return out;
    }
    let mut bases: Vec<String> = Vec::new();
    for mid in model_reaction.metabolites.keys() {
        let Some(met) = community.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        let base = local_metabolite_base(community, mid, Some(met));
        if !bases.contains(&base) {
            bases.push(base);
        }
    }
    if bases.len() < 2 {
        return out;
    }
    let spacing = 34.0;
    let centre = (bases.len() as f64 - 1.0) / 2.0;
    for (i, base) in bases.into_iter().enumerate() {
        out.insert(base, (i as f64 - centre) * spacing);
    }
    out
}

/// One metabolite of the reaction being drawn.
struct Entry<'c> {
    mid: String,
    coef: f64,
    /// The node it is drawn as, when there is one already.
    node: Option<String>,
    met: &'c Metabolite,
    transport_base: String,
}

/// `buildSingleReactionSegments`: the axis, branches and (for a multi-lane
/// transporter) the lanes of one reaction; satellites are added to the map.
/// Returns the segments with their axis links, for the caller to append.
pub fn build_single_reaction_segments(
    map: &mut EditableMap,
    community: &CommunityModel,
    reaction_id: &str,
    anchor_node_id: Option<&str>,
) -> (Vec<Segment>, Vec<crate::editable_map::SegLinks>) {
    let (Some(rxn), Some(mrxn)) = (map.g.reactions.get(reaction_id), community.merged.reactions.get(reaction_id)) else {
        return (vec![], vec![]);
    };
    let on_membrane = rxn.on_membrane;
    let anchor = selected_reaction_anchor(map, mrxn, anchor_node_id).map(|n| (n.id.clone(), n.merged_id.clone()));

    let mut subs: Vec<Entry> = Vec::new();
    let mut prods: Vec<Entry> = Vec::new();
    for (mid, &coef) in &mrxn.metabolites {
        let Some(met) = community.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        let local_secondary = !on_membrane && is_escher_secondary_metabolite(community, mid, Some(met));
        let node = match &anchor {
            Some((id, merged)) if merged.as_deref() == Some(mid.as_str()) => Some(id.clone()),
            _ if local_secondary => None,
            _ => reaction_visual_node(map, reaction_id, mid),
        };
        let entry = Entry { mid: mid.clone(), coef, node, met, transport_base: local_metabolite_base(community, mid, Some(met)) };
        if coef < 0.0 { subs.push(entry) } else { prods.push(entry) }
    }

    let dir = normalize_direction(axis_direction(map, &map.g.reactions[reaction_id], &subs, &prods));
    let half = SIZES.reaction_axis;
    let rxn = &map.g.reactions[reaction_id];
    let a1 = Point::new(rxn.x - dir.x * half, rxn.y - dir.y * half);
    let a2 = Point::new(rxn.x + dir.x * half, rxn.y + dir.y * half);
    map.replace_axis(reaction_id, Axis { a1, a2, dir });

    if on_membrane {
        let all: Vec<&Entry> = subs.iter().chain(prods.iter()).collect();
        let (mode, lanes) = transport_streams(map, community, reaction_id, &all);
        if lanes.len() >= 2 {
            return build_multi_transport(map, reaction_id, &subs, &prods, mode, lanes);
        }
    }
    {
        let r = &mut map.g.reactions[reaction_id];
        r.transport_mode = if on_membrane { Some("uniport".into()) } else { None };
        r.transport_lanes = None;
    }

    let mut segments = vec![blank_segment(reaction_id, SegmentKind::Axis, a1, a2, false)];
    let mut links: Vec<crate::editable_map::SegLinks> =
        vec![(Some(coxeter_core::mapdata::AxisEnd::A1), Some(coxeter_core::mapdata::AxisEnd::A2))];

    let rxn = map.g.reactions[reaction_id].clone();
    let substrate_anchors = backbone_anchors(&rxn, subs.len(), -1.0);
    let product_anchors = backbone_anchors(&rxn, prods.len(), 1.0);

    // Cofactors are counted on each side of the reaction separately, so the
    // first on either side takes the same place and a pair reads as one arc
    // past the marker (ATP in, ADP out) rather than as two unrelated spokes.
    let mut placed = [0usize, 0usize];
    let mut satellite = |map: &mut EditableMap, entry: &Entry, side: f64| -> String {
        let k = if side < 0.0 { 0 } else { 1 };
        let at = satellite_place(&rxn, side, placed[k]);
        placed[k] += 1;
        let id = format!("cur:{reaction_id}:{}", entry.mid);
        let mut node = blank_node(&id, NodeKind::Currency);
        node.merged_id = Some(entry.mid.clone());
        node.local_id = Some(entry.met.id.clone());
        node.base = Some(entry.transport_base.clone());
        node.name = Some(entry.met.name.clone());
        node.formula = Some(entry.met.formula.clone());
        node.compartment = Some(entry.met.compartment.clone());
        node.cell_id = rxn.cell_id.clone();
        node.reaction_id = Some(reaction_id.to_string());
        node.x = at.x;
        node.y = at.y;
        node.r = SIZES.currency_radius;
        node.primary = Some(false);
        node.visual_role = Some("secondary".into());
        node.escher_secondary = Some(true);
        map.g.nodes.insert(id.clone(), node);
        id
    };

    for (i, entry) in subs.iter().enumerate() {
        let node_id = match &entry.node {
            Some(id) => id.clone(),
            None => satellite(map, entry, -1.0),
        };
        let node = &map.g.nodes[&node_id];
        let kind = segment_kind_for(entry.node.as_ref().map(|id| &map.g.nodes[id].kind));
        let mut seg = blank_segment(reaction_id, kind, node.pos(), substrate_anchors.get(i).copied().unwrap_or(a1), false);
        seg.node_id = Some(node_id);
        seg.coefficient = Some(entry.coef);
        seg.side = Some("substrate".into());
        seg.backbone_anchor = true;
        segments.push(seg);
        links.push((None, None));
    }
    for (i, entry) in prods.iter().enumerate() {
        let node_id = match &entry.node {
            Some(id) => id.clone(),
            None => satellite(map, entry, 1.0),
        };
        let node = &map.g.nodes[&node_id];
        let kind = segment_kind_for(entry.node.as_ref().map(|id| &map.g.nodes[id].kind));
        let mut seg = blank_segment(reaction_id, kind, product_anchors.get(i).copied().unwrap_or(a2), node.pos(), true);
        seg.node_id = Some(node_id);
        seg.coefficient = Some(entry.coef);
        seg.side = Some("product".into());
        seg.backbone_anchor = true;
        segments.push(seg);
        links.push((None, None));
    }
    if let Some(boundary) = rxn.boundary {
        segments.push(blank_segment(reaction_id, SegmentKind::Boundary, a2, boundary, true));
        links.push((Some(coxeter_core::mapdata::AxisEnd::A2), None));
    }
    initialize_branch_geometry(&mut segments, &map.g.reactions[reaction_id]);
    (segments, links)
}

/// `entry.node?.kind === 'currency' ? 'currency' : (entry.node ? 'primary' : 'currency')`.
fn segment_kind_for(node_kind: Option<&NodeKind>) -> SegmentKind {
    match node_kind {
        Some(NodeKind::Currency) | None => SegmentKind::Currency,
        Some(_) => SegmentKind::Primary,
    }
}

fn backbone_anchors(rxn: &MapReaction, count: usize, side: f64) -> Vec<Point> {
    let Some(axis) = rxn.axis else { return vec![] };
    if count == 0 {
        return vec![];
    }
    let dir = axis.dir;
    let half = SIZES.reaction_axis;
    if count == 1 {
        let d = half * 0.72 * side;
        return vec![Point::new(rxn.x + dir.x * d, rxn.y + dir.y * d)];
    }
    (0..count)
        .map(|i| {
            let t = i as f64 / (count - 1) as f64;
            let d = half * (0.28 + 0.58 * t) * side;
            Point::new(rxn.x + dir.x * d, rxn.y + dir.y * d)
        })
        .collect()
}

/// Give every branch of a newly placed reaction its base curve: along the
/// backbone at both ends.
fn initialize_branch_geometry(segments: &mut [Segment], rxn: &MapReaction) {
    for seg in segments {
        if seg.node_id.as_deref().unwrap_or("").is_empty()
            || matches!(seg.kind, SegmentKind::Axis | SegmentKind::Boundary | SegmentKind::TransportLane)
        {
            continue;
        }
        shape_branch(seg, rxn);
    }
}

/// One lane of a multi-lane transporter while it is being built.
struct Stream {
    base: String,
    /// `(index into the entries, inside)`.
    entries: Vec<(usize, bool)>,
    direction: f64,
}

/// `transportStreams`: group a transporter's metabolites into lanes, one per
/// metabolite that crosses the membrane.
fn transport_streams(
    map: &EditableMap,
    community: &CommunityModel,
    reaction_id: &str,
    entries: &[&Entry],
) -> (&'static str, Vec<(Stream, f64, Point, Point)>) {
    let rxn = &map.g.reactions[reaction_id];
    let mut groups: IndexMap<String, Vec<(usize, bool)>> = IndexMap::new();
    for (k, entry) in entries.iter().enumerate() {
        let place = metabolite_place(community, &entry.mid, Some(entry.met), rxn.cell_id.as_deref());
        let inside = is_inside_boundary(map, rxn, place.cell_id.as_deref(), place.compartment.as_deref());
        groups.entry(entry.transport_base.clone()).or_default().push((k, inside));
    }
    let mut streams = Vec::new();
    for (base, list) in groups {
        let has_inside = list.iter().any(|e| e.1);
        let has_outside = list.iter().any(|e| !e.1);
        let sub_in = list.iter().any(|e| entries[e.0].coef < 0.0 && e.1);
        let sub_out = list.iter().any(|e| entries[e.0].coef < 0.0 && !e.1);
        let prod_in = list.iter().any(|e| entries[e.0].coef > 0.0 && e.1);
        let prod_out = list.iter().any(|e| entries[e.0].coef > 0.0 && !e.1);
        if !has_inside || !has_outside {
            continue;
        }
        let mut direction = 0.0;
        if sub_in && prod_out {
            direction = 1.0; // outward at + flux
        } else if sub_out && prod_in {
            direction = -1.0; // inward at + flux
        }
        streams.push(Stream { base, entries: list, direction });
    }
    if streams.len() < 2 {
        return ("uniport", vec![]);
    }
    let dirs: HashSet<i64> = streams.iter().map(|s| s.direction as i64).filter(|d| *d != 0).collect();
    let mode = if dirs.len() > 1 { "antiport" } else { "symport" };
    let normal = rxn.normal.or(rxn.axis.map(|a| a.dir)).unwrap_or(Point::new(1.0, 0.0));
    let perp = Point::new(-normal.y, normal.x);
    let spacing = 12.0;
    let centre = (streams.len() as f64 - 1.0) / 2.0;
    let half = SIZES.reaction_axis;
    let lanes = streams
        .into_iter()
        .enumerate()
        .map(|(i, stream)| {
            let offset = (i as f64 - centre) * spacing;
            let inside = Point::new(rxn.x - normal.x * half + perp.x * offset, rxn.y - normal.y * half + perp.y * offset);
            let outside = Point::new(rxn.x + normal.x * half + perp.x * offset, rxn.y + normal.y * half + perp.y * offset);
            (stream, offset, inside, outside)
        })
        .collect();
    (mode, lanes)
}

fn build_multi_transport(
    map: &mut EditableMap,
    reaction_id: &str,
    subs: &[Entry],
    prods: &[Entry],
    mode: &'static str,
    lanes: Vec<(Stream, f64, Point, Point)>,
) -> (Vec<Segment>, Vec<crate::editable_map::SegLinks>) {
    let all: Vec<&Entry> = subs.iter().chain(prods.iter()).collect();
    // The lane records, written as the JavaScript writes them: every entry
    // holds the node it is drawn as and the model's metabolite.
    let mut live_nodes = HashSet::new();
    let mut records = Vec::new();
    for (stream, offset, inside, outside) in &lanes {
        let entries: Vec<Value> = stream
            .entries
            .iter()
            .map(|&(k, in_)| {
                let e = all[k];
                let node = match &e.node {
                    Some(id) => {
                        live_nodes.insert(id.clone());
                        serde_json::to_value(&map.g.nodes[id]).expect("nodes serialise")
                    }
                    None => Value::Null,
                };
                json!({
                    "mid": e.mid, "coef": e.coef, "node": node, "met": e.met.to_json(),
                    "transportBase": e.transport_base, "inside": in_,
                })
            })
            .collect();
        let mut extra = Map::new();
        extra.insert("entries".into(), Value::Array(entries));
        records.push(TransportLane {
            base: stream.base.clone(),
            offset: *offset,
            direction: stream.direction,
            inside_point: Some(*inside),
            outside_point: Some(*outside),
            extra,
        });
    }
    {
        let r = &mut map.g.reactions[reaction_id];
        r.transport_mode = Some(mode.into());
        r.transport_lanes = Some(records);
    }
    if !live_nodes.is_empty() {
        map.live_lanes.insert(reaction_id.to_string(), live_nodes);
    }

    let mut segments = Vec::new();
    for (stream, _, inside, outside) in &lanes {
        let mut seg = blank_segment(reaction_id, SegmentKind::TransportLane, *inside, *outside, false);
        seg.transport_base = Some(stream.base.clone());
        seg.transport_direction = Some(stream.direction);
        segments.push(seg);
    }
    let lane_of = |base: &str| lanes.iter().find(|l| l.0.base == base);
    for (entries, substrate) in [(subs, true), (prods, false)] {
        for entry in entries {
            let Some((stream, _, inside, outside)) = lane_of(&entry.transport_base) else { continue };
            let Some(node_id) = &entry.node else { continue };
            let owner_inside =
                stream.entries.iter().find(|(j, _)| all[*j].mid == entry.mid).map(|(_, i)| *i).unwrap_or(false);
            let endpoint = if owner_inside { *inside } else { *outside };
            let node = &map.g.nodes[node_id];
            let kind = if node.kind == NodeKind::Currency { SegmentKind::Currency } else { SegmentKind::Primary };
            let mut seg = if substrate {
                blank_segment(reaction_id, kind, node.pos(), endpoint, false)
            } else {
                blank_segment(reaction_id, kind, endpoint, node.pos(), true)
            };
            seg.node_id = Some(node_id.clone());
            seg.coefficient = Some(entry.coef);
            seg.side = Some(if substrate { "substrate" } else { "product" }.into());
            seg.transport_base = Some(stream.base.clone());
            seg.transport_lane = true;
            segments.push(seg);
        }
    }
    initialize_branch_geometry(&mut segments, &map.g.reactions[reaction_id]);
    let links = vec![(None, None); segments.len()];
    (segments, links)
}

fn axis_direction(map: &EditableMap, rxn: &MapReaction, subs: &[Entry], prods: &[Entry]) -> Point {
    let placement = rxn.placement_direction.as_ref().filter(|v| js::truthy(v));
    if !rxn.on_membrane && (rxn.axis_direction_hint.is_some() || placement.is_some()) {
        return rxn
            .axis_direction_hint
            .or_else(|| placement.and_then(point_from_value))
            .unwrap_or(Point::new(f64::NAN, f64::NAN));
    }
    let node_pos = |e: &Entry| e.node.as_ref().map(|id| map.g.nodes[id].pos());
    if rxn.on_membrane {
        if let Some(n) = rxn.normal {
            // A membrane has two equally valid geometric orientations. Choose
            // the one that makes substrate branches reach a1 and product
            // branches reach a2 with the least total travel. This handles
            // uptake, secretion, symport and antiport without hard-coding
            // reaction names or assuming directionality.
            let forward = n;
            let reverse = Point::new(-n.x, -n.y);
            let cost = |dir: Point| {
                let half = SIZES.reaction_axis;
                let a1 = Point::new(rxn.x - dir.x * half, rxn.y - dir.y * half);
                let a2 = Point::new(rxn.x + dir.x * half, rxn.y + dir.y * half);
                let sq = |p: Option<Point>, q: Point| match p {
                    None => 0.0,
                    Some(p) => {
                        let (dx, dy) = (p.x - q.x, p.y - q.y);
                        dx * dx + dy * dy
                    }
                };
                let mut c = 0.0;
                for e in subs {
                    c += sq(node_pos(e), a1);
                }
                for e in prods {
                    c += sq(node_pos(e), a2);
                }
                c
            };
            let fc = cost(forward);
            let rc = cost(reverse);
            return if rc + 1e-9 < fc { reverse } else { forward };
        }
    }
    let centroid = |list: &[Entry]| -> Option<Point> {
        let valid: Vec<Point> = list.iter().filter_map(node_pos).collect();
        if valid.is_empty() {
            return None;
        }
        let n = valid.len() as f64;
        Some(Point::new(valid.iter().fold(0.0, |s, p| s + p.x) / n, valid.iter().fold(0.0, |s, p| s + p.y) / n))
    };
    let from = centroid(subs).unwrap_or(Point::new(rxn.x, rxn.y - 1.0));
    let to = centroid(prods).unwrap_or(Point::new(rxn.x, rxn.y + 1.0));
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = js::hypot(dx, dy);
    if length < 1e-6 {
        return Point::new(0.0, 1.0);
    }
    Point::new(dx / length, dy / length)
}

/// Is a compartment on the enclosed side of the membrane this reaction uses?
fn is_inside_boundary(map: &EditableMap, reaction: &MapReaction, cell_id: Option<&str>, compartment: Option<&str>) -> bool {
    let Some(bid) = reaction.boundary_id.as_deref().filter(|b| !b.is_empty()) else { return false };
    if is_shared_place(cell_id, compartment) {
        return false;
    }
    let Some(boundary) = map.boundary(bid) else { return false };
    if boundary.cell_id.as_deref() != cell_id {
        return false;
    }
    let Some(cell) = map.g.cells.iter().find(|c| Some(c.id.as_str()) == cell_id) else { return false };
    let by_id: IndexMap<&str, &coxeter_core::mapdata::MapCompartment> =
        cell.compartments.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut current = compartment.and_then(|c| by_id.get(c).copied());
    let mut guard = 0;
    while let Some(c) = current {
        if c.id == boundary.inside {
            return true;
        }
        guard += 1;
        if guard > by_id.len() + 1 {
            return false; // a cycle in the drawn nesting
        }
        current = c.parent_id.as_deref().and_then(|p| by_id.get(p).copied());
    }
    false
}
