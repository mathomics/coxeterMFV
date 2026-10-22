//! The editable map (port of `src/map/editable-map.js`).
//!
//! This module is where three things are kept apart:
//!
//! ```text
//!   Model        everything the biology makes available (`Model` and
//!                `CommunityModel` in coxeter-core).
//!   EditableMap  what is currently drawn. A subset of the model, plus the
//!                geometry that subset is drawn with. Owns cells,
//!                compartments, boundaries, nodes, reactions, segments and the
//!                canvas.
//!   Simulation   the LP and its answer. It reads the model, never the map.
//! ```
//!
//! The point of the split is that a map is allowed to be incomplete. It may
//! hold every reaction in the community, a handful of them, or none at all,
//! and none of those is a degenerate case: an empty map is the honest starting
//! point for building one by hand, and it still draws the cells, their
//! membranes and the medium, because those come from the model's structure
//! rather than from its reactions.
//!
//! Nothing here solves, rebuilds or re-runs the automatic layout. The map is
//! geometry and membership; the numbers live elsewhere.
//!
//! # Points shared with a reaction's axis
//!
//! In JavaScript some segment endpoints are the very objects of the reaction's
//! axis (`seg.to === rxn.axis.a1`), so moving the axis in place moves them, and
//! replacing an endpoint breaks the link. The Rust map keeps a copy of every
//! point and records, per segment end, which axis point it is currently the
//! same object as ([`EditableMap::segment_links`]). Every edit keeps that
//! record exactly as the JavaScript object graph would be, so an edit that in
//! JavaScript moves a point through an alias moves the same points here.

use std::collections::{HashMap, HashSet};

use coxeter_core::boundaries::{BoundaryKind, boundary_key, environment_key};
use coxeter_core::colors::normalize_hex_color;
use coxeter_core::community::{CommunityModel, SHARED_COMPARTMENT};
use coxeter_core::compartments::{
    Names, cell_compartment_name, cell_compartment_visual_id, compartment_name, membrane_style,
};
use coxeter_core::geometry::{SIZES, closest_perimeter_point, closest_perimeter_point_corners, compartment_corners};
use coxeter_core::js;
use coxeter_core::mapdata::{
    AxisEnd, Canvas, Environment, MapCell, MapCompartment, MapData, MapGraph, MapReaction, MapText, Medium, Node,
    NodeKind, Point, Rect, Segment, axis_links,
};
use coxeter_core::metabolite_label::{is_secondary_node, metabolite_label_angle};
use coxeter_core::text_style::{TextBox, clamp_text_size, normalize_text_font};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::layout::{Contents, Layout, LayoutOptions, layout_community};
use crate::util::blank_node;

/// Which axis point each end of a segment is currently the same object as:
/// `(from, to)`.
pub type SegLinks = (Option<AxisEnd>, Option<AxisEnd>);

/// The map field that says how secondary metabolites are drawn.
pub const COFACTOR_STYLE: &str = "cofactorStyle";

/// `normalizeCofactorStyle`: the secondary-metabolite styles a map can carry;
/// anything else is the default.
fn normalize_cofactor_style(style: Option<&Value>) -> Option<&'static str> {
    match style.and_then(Value::as_str) {
        Some("quiet") => Some("quiet"),
        _ => None,
    }
}

/// One end of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegEnd {
    From,
    To,
}

/// Errors a user can cause by handing the map something it cannot read.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MapError {
    #[error("The map could not be read: {0}")]
    Invalid(String),
}

/// A membrane in the drawing, derived from the cells and the environment.
///
/// One per compartment (the surface of a compartment is what separates it
/// from whatever it sits in), plus one for the shared medium, whose outside
/// is the world beyond the system.
#[derive(Debug, Clone, PartialEq)]
pub struct Boundary {
    pub id: String,
    pub kind: BoundaryKind,
    pub cell_id: Option<String>,
    pub inside: String,
    /// None: beyond the system.
    pub outside: Option<String>,
    pub compartment_id: String,
    pub parent_id: Option<String>,
    pub depth: f64,
    pub membrane: String,
    pub rect: Option<Rect>,
    pub label: String,
}

impl Boundary {
    /// The JSON the JavaScript object would print.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "kind": self.kind.as_str(), "cellId": self.cell_id, "inside": self.inside,
            "outside": self.outside, "compartmentId": self.compartment_id, "parentId": self.parent_id,
            "depth": self.depth, "membrane": self.membrane, "rect": self.rect, "label": self.label,
        })
    }
}

/// `stats()`: a count of everything, for status lines and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapStats {
    pub cells: usize,
    pub compartments: usize,
    pub boundaries: usize,
    pub reactions: usize,
    pub nodes: usize,
    pub metabolites: usize,
    pub currency: usize,
    pub medium: usize,
    pub texts: usize,
    pub segments: usize,
}

/// A colour edit: leave it, reset it to the theme, or set it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ColorChange {
    #[default]
    Keep,
    Reset,
    Set(String),
}

/// What `addCell` is given. Strings are trimmed; anything missing takes the
/// JavaScript default.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellSpec {
    pub species_id: String,
    pub outer_compartment_id: Option<String>,
    pub rect: Option<Rect>,
    pub outer_label: Option<String>,
    pub outer_membrane: Option<String>,
    pub cell_name: Option<String>,
    pub organism: Option<String>,
    pub note: Option<String>,
    pub color: Option<String>,
    pub abundance: Option<f64>,
}

/// What `addCompartment` is given.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompartmentSpec {
    pub species_id: String,
    pub compartment_id: String,
    pub parent_id: Option<String>,
    pub rect: Option<Rect>,
    pub label: Option<String>,
    pub membrane: Option<String>,
    pub cell_name: Option<String>,
    pub organism: Option<String>,
    pub note: Option<String>,
    pub color: Option<String>,
    pub abundance: Option<f64>,
}

/// `addCell` / `addCompartment` results.
#[derive(Debug, Clone, PartialEq)]
pub struct AddedShell {
    pub cell: MapCell,
    pub compartment: MapCompartment,
    pub boundary: Option<Boundary>,
}

/// `removeReaction` result.
#[derive(Debug, Clone, PartialEq)]
pub struct RemovedReaction {
    pub reaction: MapReaction,
    pub nodes: Vec<String>,
}

/// What `removeCompartment` took off the drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct RemovedCompartment {
    pub cell_id: String,
    /// The compartment and those nested in it.
    pub compartment_ids: Vec<String>,
    /// Whether it was the cell's outermost compartment, and the cell went.
    pub cell_removed: bool,
    /// The cell's name, or the compartment's.
    pub label: String,
    pub reactions: Vec<String>,
    pub nodes: Vec<String>,
    pub texts: Vec<String>,
}

/// `mergeMetaboliteNodes` result.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    pub source: Node,
    pub target: Node,
    pub rewired: usize,
}

/// Options of `duplicateMetaboliteNode`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DuplicateOptions {
    pub id: Option<String>,
    pub offset: Option<f64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

/// Options of `addText`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextOptions {
    pub id: Option<String>,
    pub font_size: Option<f64>,
    pub font_family: Option<String>,
}

/// Where `geometryTarget` points: a cell's compartment, or the environment.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryTarget {
    Environment,
    Compartment { cell: usize, compartment: usize },
}

/// Node kinds that represent a metabolite rather than map furniture.
pub(crate) fn is_metabolite_kind(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Metabolite | NodeKind::Medium | NodeKind::Currency)
}

/// `['metabolite', 'medium'].includes(kind)`.
pub(crate) fn is_primary_kind(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Metabolite | NodeKind::Medium)
}

/// The least distance between two neighbours on a line made by Align
/// vertical / horizontal: a reaction marker and a metabolite either side of
/// it, with room for their labels.
pub(crate) const LINE_ALIGN_GAP: f64 = 40.0;

/// The live, editable map. See the module documentation.
#[derive(Debug, Clone, PartialEq)]
pub struct EditableMap {
    pub(crate) g: MapGraph,
    /// Parallel to `g.segments`: which axis point each end is the same object as.
    pub(crate) links: Vec<SegLinks>,
    pub(crate) boundaries: Vec<Boundary>,
    pub(crate) boundary_index: HashMap<String, usize>,
    /// Reactions whose transport lanes were built in this session, with the
    /// node ids their `entries[].node` hold: in JavaScript those are the live
    /// node objects, so they are written as the nodes currently are.
    pub(crate) live_lanes: HashMap<String, HashSet<String>>,
}

impl EditableMap {
    // Construction

    /// The constructor: adopt a graph as it is and apply the normalisations
    /// every map goes through on its way in. `links` is parallel to the
    /// segments; `None` derives it from the axis rule.
    pub(crate) fn from_graph(mut g: MapGraph, links: Option<Vec<SegLinks>>) -> EditableMap {
        name_cell_compartments(&mut g.cells);
        migrate_reaction_local_ownership(&mut g.nodes);
        normalize_visual_roles(&mut g.nodes);
        normalize_metabolite_label_angles(&mut g.nodes);
        strip_species_from_labels(&mut g.nodes);
        let links = links.unwrap_or_else(|| initial_links(&g));
        let mut map = EditableMap { g, links, boundaries: Vec::new(), boundary_index: HashMap::new(), live_lanes: HashMap::new() };
        map.refresh_boundaries();
        map
    }

    /// `EditableMap.fromLayout(layout, opts)`: adopt a layout produced by
    /// [`layout_community`]. `reactions` keeps only those reaction ids.
    pub fn from_layout(layout: Layout, reactions: Option<&[String]>) -> EditableMap {
        let canvas = Canvas { width: layout.width, height: layout.height };
        let g = MapGraph {
            version: Value::from(1),
            canvas,
            environment: layout.environment,
            cells: layout.cells,
            medium: layout.medium,
            nodes: layout.nodes,
            reactions: layout.reactions,
            segments: layout.segments,
            texts: IndexMap::new(),
            label_settings: None,
            node_style: None,
            extra: Map::new(),
        };
        let map = EditableMap::from_graph(g, None);
        match reactions {
            None => map,
            Some(ids) => map.subset_by_reactions(ids),
        }
    }

    /// `EditableMap.fromCommunity(community, opts)`: lay a community out and
    /// adopt the result.
    pub fn from_community(community: &CommunityModel, opts: &LayoutOptions) -> EditableMap {
        EditableMap::from_layout(layout_community(community, opts), opts.reactions.as_deref())
    }

    /// `EditableMap.emptyFromCommunity`: the shell of a community and nothing
    /// else: every cell, its outer membrane, the compartments nested inside
    /// it and the shared medium around them. The compartment packing is the
    /// same code the full layout uses, so the shell a user starts from is the
    /// shell their finished map will have.
    pub fn empty_from_community(community: &CommunityModel, opts: &LayoutOptions) -> EditableMap {
        let opts = LayoutOptions { contents: Contents::None, ..opts.clone() };
        EditableMap::from_layout(layout_community(community, &opts), None)
    }

    /// `EditableMap.blank`: a truly blank drawing surface, with no environment
    /// decoration, cells, compartments, metabolites, reactions or segments.
    pub fn blank(width: Option<f64>, height: Option<f64>) -> EditableMap {
        let width = js::max(320.0, crate::util::num_or(width.unwrap_or(f64::NAN), 1200.0));
        let height = js::max(240.0, crate::util::num_or(height.unwrap_or(f64::NAN), 800.0));
        let g = MapGraph {
            version: Value::from(1),
            canvas: Canvas { width, height },
            environment: Environment {
                rect: Rect::new(0.0, 0.0, width, height),
                id: SHARED_COMPARTMENT.into(),
                label: String::new(),
                hidden: Some(true),
                extra: Map::new(),
            },
            cells: Vec::new(),
            medium: Medium::default(),
            nodes: IndexMap::new(),
            reactions: IndexMap::new(),
            segments: Vec::new(),
            texts: IndexMap::new(),
            label_settings: None,
            node_style: None,
            extra: Map::new(),
        };
        EditableMap::from_graph(g, None)
    }

    /// `EditableMap.fromJSON(doc)`: rebuild a map from `toJSON()` output.
    ///
    /// JSON has no object identity, so the endpoints a segment shares with its
    /// reaction's axis arrive as separate copies. They are re-linked here,
    /// which keeps the invariant the geometry relies on: the axis is one
    /// object, and moving it moves everything pinned to it.
    pub fn from_json(doc: &Value) -> Result<EditableMap, MapError> {
        let data: MapData = serde_json::from_value(doc.clone()).map_err(|e| MapError::Invalid(e.to_string()))?;
        Ok(EditableMap::from_data(data))
    }

    /// [`EditableMap::from_json`] from already typed map data.
    pub fn from_data(data: MapData) -> EditableMap {
        let mut g = MapGraph::from_data(data);
        // Only the fields the map knows are carried; JavaScript drops the rest.
        // The one it knows that the shared graph has no field for is the
        // cofactor style, kept among the extra fields.
        let style = normalize_cofactor_style(g.extra.get(COFACTOR_STYLE));
        g.extra = Map::new();
        if let Some(style) = style {
            g.extra.insert(COFACTOR_STYLE.into(), Value::from(style));
        }
        g.version = Value::from(1);
        EditableMap::from_graph(g, None)
    }

    /// `toJSON()`: the graphical state, detached from the map. Boundaries are
    /// left out on purpose: they are derived from the cells and the
    /// environment, and storing them would let a file contradict itself.
    pub fn to_json(&self) -> Value {
        let mut data = self.g.to_data();
        data.extra = Map::new();
        if let Some(style) = self.cofactor_style() {
            data.extra.insert(COFACTOR_STYLE.into(), Value::from(style));
        }
        let mut v = serde_json::to_value(&data).expect("map data always serialises");
        if let Value::Object(m) = &mut v {
            if self.g.label_settings.is_none() {
                m.remove("labelSettings");
            }
        }
        self.write_live_lane_entries(&mut v);
        js::normalize_json_numbers(&mut v);
        v
    }

    /// The map as typed data (what [`EditableMap::to_json`] prints).
    pub fn to_data(&self) -> MapData {
        serde_json::from_value(self.to_json()).expect("the map's own JSON reads back")
    }

    // Reading

    /// The live graph, for reading. Edits go through the methods.
    pub fn graph(&self) -> &MapGraph {
        &self.g
    }
    pub fn width(&self) -> f64 {
        self.g.canvas.width
    }
    pub fn height(&self) -> f64 {
        self.g.canvas.height
    }
    pub fn canvas(&self) -> Canvas {
        self.g.canvas
    }
    pub fn environment(&self) -> &Environment {
        &self.g.environment
    }
    pub fn cells(&self) -> &[MapCell] {
        &self.g.cells
    }
    pub fn medium(&self) -> &Medium {
        &self.g.medium
    }
    pub fn nodes(&self) -> &IndexMap<String, Node> {
        &self.g.nodes
    }
    pub fn reactions(&self) -> &IndexMap<String, MapReaction> {
        &self.g.reactions
    }
    pub fn segments(&self) -> &[Segment] {
        &self.g.segments
    }
    pub fn texts(&self) -> &IndexMap<String, MapText> {
        &self.g.texts
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.g.nodes.get(id)
    }
    pub fn reaction(&self, id: &str) -> Option<&MapReaction> {
        self.g.reactions.get(id)
    }
    pub fn text(&self, id: &str) -> Option<&MapText> {
        self.g.texts.get(id)
    }
    pub fn cell(&self, id: &str) -> Option<&MapCell> {
        self.g.cells.iter().find(|c| c.id == id)
    }
    /// Which axis point each segment end is the same object as, parallel to
    /// [`EditableMap::segments`].
    pub fn segment_links(&self) -> &[SegLinks] {
        &self.links
    }
    /// Which label source each kind of object shows, map-wide.
    pub fn label_settings(&self) -> Option<&IndexMap<String, String>> {
        self.g.label_settings.as_ref()
    }
    /// `cofactorStyle`: how the secondary metabolites are drawn, `quiet`
    /// (smaller and lighter, so the pathways read first) or `None` for the
    /// full size every map has by default.
    pub fn cofactor_style(&self) -> Option<&'static str> {
        normalize_cofactor_style(self.g.extra.get(COFACTOR_STYLE))
    }

    /// Change the graph directly, for what no method covers (a field of a
    /// node, a hand-built fixture). The map's own book-keeping is put right
    /// afterwards as far as it can be: if segments were added or removed, the
    /// record of which ends are a reaction's axis points is derived again
    /// from the axis rule, as on loading. Boundaries are *not* refreshed.
    /// Call [`EditableMap::refresh_boundaries`] after changing cells, as the
    /// JavaScript callers do.
    pub fn edit_graph<R>(&mut self, edit: impl FnOnce(&mut MapGraph) -> R) -> R {
        let before = self.g.segments.len();
        let out = edit(&mut self.g);
        if self.g.segments.len() != before || self.links.len() != self.g.segments.len() {
            self.links = initial_links(&self.g);
        }
        out
    }

    // Membership

    pub fn has_reaction(&self, id: &str) -> bool {
        self.g.reactions.contains_key(id)
    }
    pub fn has_node(&self, id: &str) -> bool {
        self.g.nodes.contains_key(id)
    }
    pub fn has_boundary(&self, id: &str) -> bool {
        self.boundary_index.contains_key(id)
    }
    pub fn reaction_ids(&self) -> Vec<String> {
        self.g.reactions.keys().cloned().collect()
    }
    pub fn node_ids(&self) -> Vec<String> {
        self.g.nodes.keys().cloned().collect()
    }
    pub fn boundaries(&self) -> &[Boundary] {
        &self.boundaries
    }
    pub fn boundary(&self, id: &str) -> Option<&Boundary> {
        self.boundary_index.get(id).map(|&i| &self.boundaries[i])
    }

    /// Re-derive the boundaries after the cells or the environment changed.
    pub fn refresh_boundaries(&mut self) -> &[Boundary] {
        self.boundaries = derive_boundaries(&self.g);
        self.boundary_index = HashMap::new();
        for (i, b) in self.boundaries.iter().enumerate() {
            // A later duplicate id wins, as `new Map(entries)` keeps the last.
            self.boundary_index.insert(b.id.clone(), i);
        }
        &self.boundaries
    }

    /// The boundaries belonging to one cell; `None` gives the environment's.
    pub fn boundaries_of(&self, cell_id: Option<&str>) -> Vec<&Boundary> {
        self.boundaries.iter().filter(|b| b.cell_id.as_deref() == cell_id).collect()
    }

    /// Segments belonging to one reaction.
    pub fn segments_of(&self, reaction_id: &str) -> Vec<&Segment> {
        self.g.segments.iter().filter(|s| s.reaction_id == reaction_id).collect()
    }

    /// Segments whose metabolite end is this node.
    pub fn segments_at(&self, node_id: &str) -> Vec<&Segment> {
        self.g.segments.iter().filter(|s| s.node_id.as_deref() == Some(node_id)).collect()
    }

    /// Nothing drawn inside the shell: no reactions and no metabolites.
    pub fn is_empty(&self) -> bool {
        if !self.g.reactions.is_empty() {
            return false;
        }
        if self.g.nodes.values().any(|n| is_metabolite_kind(&n.kind)) {
            return false;
        }
        self.g.segments.is_empty()
    }

    /// `stats()`: how many of each kind of object the map holds.
    pub fn stats(&self) -> MapStats {
        let mut s = MapStats::default();
        for node in self.g.nodes.values() {
            match node.kind {
                NodeKind::Metabolite => s.metabolites += 1,
                NodeKind::Currency => s.currency += 1,
                NodeKind::Medium => s.medium += 1,
                _ => {}
            }
        }
        s.cells = self.g.cells.len();
        s.compartments = self.g.cells.iter().map(|c| c.compartments.len()).sum();
        s.boundaries = self.boundaries.len();
        s.reactions = self.g.reactions.len();
        s.nodes = self.g.nodes.len();
        s.texts = self.g.texts.len();
        s.segments = self.g.segments.len();
        s
    }

    // Segment and axis plumbing

    /// Replace one end of segment `i` with a fresh point (`seg.from = {…}`):
    /// whatever axis point it was linked to, it no longer is.
    pub(crate) fn set_seg(&mut self, i: usize, end: SegEnd, p: Point) {
        match end {
            SegEnd::From => {
                self.g.segments[i].from = p;
                self.links[i].0 = None;
            }
            SegEnd::To => {
                self.g.segments[i].to = p;
                self.links[i].1 = None;
            }
        }
    }

    /// `seg.from = rxn.axis.a1` and the like: the end becomes that axis point.
    pub(crate) fn alias_seg(&mut self, i: usize, end: SegEnd, axis_end: AxisEnd) {
        let rid = self.g.segments[i].reaction_id.clone();
        let Some(axis) = self.g.reactions.get(&rid).and_then(|r| r.axis) else { return };
        let p = match axis_end {
            AxisEnd::A1 => axis.a1,
            AxisEnd::A2 => axis.a2,
        };
        match end {
            SegEnd::From => {
                self.g.segments[i].from = p;
                self.links[i].0 = Some(axis_end);
            }
            SegEnd::To => {
                self.g.segments[i].to = p;
                self.links[i].1 = Some(axis_end);
            }
        }
    }

    /// `rxn.axis.a1 = {…}`: the axis point is a new object, so no segment
    /// end is that object any more.
    pub(crate) fn replace_axis_point(&mut self, rid: &str, end: AxisEnd, p: Point) {
        if let Some(axis) = self.g.reactions.get_mut(rid).and_then(|r| r.axis.as_mut()) {
            match end {
                AxisEnd::A1 => axis.a1 = p,
                AxisEnd::A2 => axis.a2 = p,
            }
        }
        for (seg, link) in self.g.segments.iter().zip(self.links.iter_mut()) {
            if seg.reaction_id != rid {
                continue;
            }
            if link.0 == Some(end) {
                link.0 = None;
            }
            if link.1 == Some(end) {
                link.1 = None;
            }
        }
    }

    /// `rxn.axis = {a1, a2, dir}`: a whole new axis object.
    pub(crate) fn replace_axis(&mut self, rid: &str, axis: coxeter_core::mapdata::Axis) {
        self.replace_axis_point(rid, AxisEnd::A1, axis.a1);
        self.replace_axis_point(rid, AxisEnd::A2, axis.a2);
        if let Some(r) = self.g.reactions.get_mut(rid) {
            r.axis = Some(axis);
        }
    }

    /// `rxn.axis.a1.x += dx` (and the other three): move the axis points in
    /// place, which moves every segment end that is the same object.
    pub(crate) fn shift_axis_in_place(&mut self, rid: &str, dx: f64, dy: f64) {
        let Some(axis) = self.g.reactions.get_mut(rid).and_then(|r| r.axis.as_mut()) else { return };
        axis.a1.x += dx;
        axis.a1.y += dy;
        axis.a2.x += dx;
        axis.a2.y += dy;
        let (a1, a2) = (axis.a1, axis.a2);
        for (seg, link) in self.g.segments.iter_mut().zip(&self.links) {
            if seg.reaction_id != rid {
                continue;
            }
            let at = |e: AxisEnd| if e == AxisEnd::A1 { a1 } else { a2 };
            if let Some(e) = link.0 {
                seg.from = at(e);
            }
            if let Some(e) = link.1 {
                seg.to = at(e);
            }
        }
    }

    /// Keep only the segments `keep` says yes to, links alongside.
    pub(crate) fn retain_segments(&mut self, mut keep: impl FnMut(&Segment) -> bool) {
        let mut links = std::mem::take(&mut self.links).into_iter();
        let mut kept_links = Vec::with_capacity(self.g.segments.len());
        self.g.segments.retain(|seg| {
            let link = links.next().unwrap_or((None, None));
            let k = keep(seg);
            if k {
                kept_links.push(link);
            }
            k
        });
        self.links = kept_links;
    }

    /// Append a segment with its links.
    pub(crate) fn push_segment(&mut self, seg: Segment, links: SegLinks) {
        self.g.segments.push(seg);
        self.links.push(links);
    }

    // Transport lanes

    /// Write the live `node` of every lane entry built in this session as the
    /// node now is: in JavaScript the entry holds the node object itself.
    fn write_live_lane_entries(&self, v: &mut Value) {
        if self.live_lanes.is_empty() {
            return;
        }
        let Some(reactions) = v.get_mut("reactions").and_then(Value::as_array_mut) else { return };
        for rxn in reactions {
            let Some(id) = rxn.get("id").and_then(Value::as_str) else { continue };
            let Some(live) = self.live_lanes.get(id) else { continue };
            let Some(lanes) = rxn.get_mut("transportLanes").and_then(Value::as_array_mut) else { continue };
            for lane in lanes {
                let Some(entries) = lane.get_mut("entries").and_then(Value::as_array_mut) else { continue };
                for entry in entries {
                    let Some(node_id) = entry.get("node").and_then(|n| n.get("id")).and_then(Value::as_str) else { continue };
                    if !live.contains(node_id) {
                        continue;
                    }
                    if let Some(node) = self.g.nodes.get(node_id) {
                        entry["node"] = serde_json::to_value(node).expect("nodes serialise");
                    }
                }
            }
        }
    }

    /// A copy of a reaction whose live lane entries hold the nodes as they are
    /// now (what a copy made in JavaScript would go on pointing at).
    pub(crate) fn reaction_with_current_lanes(&self, rid: &str) -> Option<MapReaction> {
        let mut rxn = self.g.reactions.get(rid)?.clone();
        let Some(live) = self.live_lanes.get(rid) else { return Some(rxn) };
        for lane in rxn.transport_lanes.iter_mut().flatten() {
            let Some(Value::Array(entries)) = lane.extra.get_mut("entries") else { continue };
            for entry in entries {
                let Some(node_id) = entry.get("node").and_then(|n| n.get("id")).and_then(Value::as_str) else { continue };
                if !live.contains(node_id) {
                    continue;
                }
                if let Some(node) = self.g.nodes.get(node_id) {
                    entry["node"] = serde_json::to_value(node).expect("nodes serialise");
                }
            }
        }
        Some(rxn)
    }

    /// Before a node leaves the map, freeze the lane entries that hold it as
    /// they are: JavaScript keeps the removed object in the entry.
    pub(crate) fn freeze_lane_entries_of(&mut self, node_id: &str) {
        if self.live_lanes.is_empty() {
            return;
        }
        let Some(node) = self.g.nodes.get(node_id) else { return };
        let snapshot = serde_json::to_value(node).expect("nodes serialise");
        for (rid, live) in self.live_lanes.iter_mut() {
            if !live.remove(node_id) {
                continue;
            }
            let Some(rxn) = self.g.reactions.get_mut(rid) else { continue };
            for lane in rxn.transport_lanes.iter_mut().flatten() {
                let Some(Value::Array(entries)) = lane.extra.get_mut("entries") else { continue };
                for entry in entries {
                    if entry.get("node").and_then(|n| n.get("id")).and_then(Value::as_str) == Some(node_id) {
                        entry["node"] = snapshot.clone();
                    }
                }
            }
        }
    }

    /// Remove a node from the map (`nodes.delete(id)`), keeping lane entries
    /// that point at it as they were.
    pub(crate) fn delete_node(&mut self, id: &str) -> Option<Node> {
        self.freeze_lane_entries_of(id);
        self.g.nodes.shift_remove(id)
    }

    // Edits

    /// `addCell`: draw one species/cell boundary on the map.
    ///
    /// The cell boundary is also the outer/root extracellular compartment of
    /// that species: its outside is the shared medium and its inside is the
    /// species' extracellular space. This is map geometry only; the
    /// biological model is never modified.
    pub fn add_cell(&mut self, spec: &CellSpec) -> Option<AddedShell> {
        let species_id = spec.species_id.trim().to_string();
        let outer_id = spec.outer_compartment_id.as_deref().unwrap_or("e").trim().to_string();
        if species_id.is_empty() || outer_id.is_empty() {
            return None;
        }
        if self.g.cells.iter().any(|c| c.id == species_id) {
            return None;
        }
        let rect = match normalize_rect(spec.rect) {
            Some(requested) => clamp_cell_rect_to_canvas(self, requested),
            None => find_free_cell_rect(self, None),
        };

        let mut names = Names::new();
        names.insert(outer_id.clone(), spec.outer_label.clone().unwrap_or_default());
        let outer_visual = cell_compartment_visual_id(&outer_id, &names);
        let compartment = MapCompartment {
            id: outer_id.clone(),
            rect,
            depth: 0.0,
            parent_id: None,
            label: cell_compartment_name(&outer_id, &names, "en"),
            membrane: non_empty(spec.outer_membrane.as_deref())
                .map(str::to_string)
                .unwrap_or_else(|| membrane_style(&outer_visual, &Names::new()).as_str().into()),
            visual_id: outer_visual,
            fill: None,
            stroke: None,
            shape: None,
            extra: Map::new(),
        };
        let cell = MapCell {
            id: species_id.clone(),
            name: non_empty(spec.cell_name.as_deref()).unwrap_or(&species_id).to_string(),
            organism: Some(spec.organism.clone().unwrap_or_default()),
            note: Some(spec.note.clone().unwrap_or_default()),
            color: non_empty(spec.color.as_deref()).unwrap_or("#64748b").to_string(),
            abundance: Some(spec.abundance.filter(|a| a.is_finite()).unwrap_or(1.0)),
            rect,
            compartments: vec![compartment.clone()],
            outer_compartment: Some(outer_id.clone()),
            stroke: None,
            manually_drawn: Some(true),
            extra: Map::new(),
        };
        self.g.cells.push(cell.clone());
        self.refresh_boundaries();
        let boundary = self.boundary(&boundary_key(Some(&species_id), &outer_id)).cloned();
        Some(AddedShell { cell, compartment, boundary })
    }

    /// `addCompartment`: add one biological compartment shell by hand.
    ///
    /// If the species has no drawn cell yet, adding its first root
    /// compartment creates the cell shell as well. Children can only be added
    /// after their biological parent is already drawn.
    pub fn add_compartment(&mut self, spec: &CompartmentSpec) -> Option<AddedShell> {
        let species_id = spec.species_id.trim().to_string();
        let compartment_id = spec.compartment_id.trim().to_string();
        if species_id.is_empty() || compartment_id.is_empty() {
            return None;
        }
        let existing = self.g.cells.iter().position(|c| c.id == species_id);
        if let Some(ci) = existing {
            if self.g.cells[ci].compartments.iter().any(|c| c.id == compartment_id) {
                return None;
            }
        }
        let parent_id = spec.parent_id.clone();
        if let Some(pid) = &parent_id {
            let has = existing.is_some_and(|ci| self.g.cells[ci].compartments.iter().any(|c| &c.id == pid));
            if !has {
                return None;
            }
        }
        let ci = match existing {
            Some(ci) => ci,
            None => {
                let rect = find_free_cell_rect(self, spec.rect);
                self.g.cells.push(MapCell {
                    id: species_id.clone(),
                    name: non_empty(spec.cell_name.as_deref()).unwrap_or(&species_id).to_string(),
                    organism: Some(spec.organism.clone().unwrap_or_default()),
                    note: Some(spec.note.clone().unwrap_or_default()),
                    color: non_empty(spec.color.as_deref()).unwrap_or("#64748b").to_string(),
                    abundance: Some(spec.abundance.filter(|a| a.is_finite()).unwrap_or(1.0)),
                    rect,
                    compartments: Vec::new(),
                    outer_compartment: Some(compartment_id.clone()),
                    stroke: None,
                    manually_drawn: None,
                    extra: Map::new(),
                });
                self.g.cells.len() - 1
            }
        };
        let cell = &self.g.cells[ci];
        let depth = match &parent_id {
            Some(pid) => cell.compartments.iter().find(|c| &c.id == pid).map(|c| c.depth).unwrap_or(-1.0) + 1.0,
            None => 0.0,
        };
        let rect = match &parent_id {
            Some(pid) => find_free_child_rect(cell, pid, spec.rect),
            None if cell.compartments.is_empty() => Some(cell.rect),
            None => Some(find_free_root_rect(cell, spec.rect)),
        }?;

        let mut names = Names::new();
        names.insert(compartment_id.clone(), spec.label.clone().unwrap_or_default());
        let visual_id = cell_compartment_visual_id(&compartment_id, &names);
        let compartment = MapCompartment {
            id: compartment_id.clone(),
            rect,
            depth,
            parent_id: parent_id.clone(),
            label: cell_compartment_name(&compartment_id, &names, "en"),
            membrane: non_empty(spec.membrane.as_deref())
                .map(str::to_string)
                .unwrap_or_else(|| membrane_style(&visual_id, &Names::new()).as_str().into()),
            visual_id,
            fill: None,
            stroke: None,
            shape: None,
            extra: Map::new(),
        };
        let cell = &mut self.g.cells[ci];
        cell.compartments.push(compartment.clone());
        if parent_id.is_none() {
            if cell.outer_compartment.as_deref().unwrap_or("").is_empty() {
                cell.outer_compartment = Some(compartment_id.clone());
            }
            let roots: Vec<Rect> = cell.compartments.iter().filter(|c| c.parent_id.is_none()).map(|c| c.rect).collect();
            cell.rect = union_rects(&roots).unwrap_or(compartment.rect);
        }
        let cell = cell.clone();
        self.refresh_boundaries();
        Some(AddedShell { cell, compartment, boundary: None })
    }

    /// `setCanvasSize(width, height, move)`: resize the canvas, optionally
    /// moving everything on it first.
    ///
    /// Growing from the right or the bottom only changes the paper. Growing
    /// from the top or the left moves the edge the drawing is measured from,
    /// so the drawing has to move with it or it would slide off the side the
    /// user is pulling. The clamped size comes back so the caller can see what
    /// it actually got.
    pub fn set_canvas_size(&mut self, width: Option<f64>, height: Option<f64>, shift_x: f64, shift_y: f64) -> Canvas {
        let shift_x = crate::util::num_or(shift_x, 0.0);
        let shift_y = crate::util::num_or(shift_y, 0.0);
        if shift_x != 0.0 || shift_y != 0.0 {
            crate::geometry_edit::translate_whole_map(self, shift_x, shift_y);
        }
        let min = self.minimum_canvas_size(40.0);
        let w = js::max(min.width, crate::util::num_or(width.unwrap_or(f64::NAN), self.g.canvas.width));
        let h = js::max(min.height, crate::util::num_or(height.unwrap_or(f64::NAN), self.g.canvas.height));
        self.g.canvas.width = w;
        self.g.canvas.height = h;
        if self.g.environment.hidden == Some(true) {
            self.g.environment.rect = Rect::new(0.0, 0.0, w, h);
        }
        Canvas { width: w, height: h }
    }

    /// `minimumCanvasSize(padding)`: the least canvas that holds the drawing.
    pub fn minimum_canvas_size(&self, padding: f64) -> Canvas {
        let mut max_x = 320.0 - padding;
        let mut max_y = 240.0 - padding;
        for cell in &self.g.cells {
            max_x = js::max(max_x, cell.rect.x + cell.rect.w);
            max_y = js::max(max_y, cell.rect.y + cell.rect.h);
        }
        for node in self.g.nodes.values() {
            max_x = js::max(max_x, node.x + node.r);
            max_y = js::max(max_y, node.y + node.r);
        }
        for rxn in self.g.reactions.values() {
            max_x = js::max(max_x, rxn.x + 25.0);
            max_y = js::max(max_y, rxn.y + 25.0);
        }
        for t in self.g.texts.values() {
            max_x = js::max(max_x, t.x + 20.0);
            max_y = js::max(max_y, t.y + 20.0);
        }
        Canvas { width: js::max(320.0, (max_x + padding).ceil()), height: js::max(240.0, (max_y + padding).ceil()) }
    }

    /// `removeReaction`: remove one reaction from the drawing only.
    ///
    /// The biological model and the LP are untouched. Reaction segments
    /// disappear, reaction-specific currency nodes disappear, and any
    /// primary/medium metabolite node left with no surviving segment is
    /// pruned from the map.
    pub fn remove_reaction(&mut self, reaction_id: &str) -> Option<RemovedReaction> {
        let reaction = self.g.reactions.shift_remove(reaction_id)?;
        self.live_lanes.remove(reaction_id);

        // Only nodes touched by the removed reaction are candidates for
        // pruning. This protects any unrelated hand-placed/orphan node that
        // may already be present on an edited map.
        let candidates: HashSet<String> = self
            .g
            .segments
            .iter()
            .filter(|s| s.reaction_id == reaction_id)
            .filter_map(|s| s.node_id.clone().filter(|n| !n.is_empty()))
            .collect();
        self.retain_segments(|s| s.reaction_id != reaction_id);

        let referenced: HashSet<String> =
            self.g.segments.iter().filter_map(|s| s.node_id.clone().filter(|n| !n.is_empty())).collect();

        let mut removed = Vec::new();
        let ids: Vec<String> = self.g.nodes.keys().cloned().collect();
        for id in ids {
            let node = &self.g.nodes[&id];
            let primary_placed = node.reaction_local_for.as_deref() == Some(reaction_id);
            let satellite_owned = node.reaction_id.as_deref() == Some(reaction_id);
            let owned = primary_placed || satellite_owned;
            let reaction_owned = owned && !referenced.contains(&id);
            let became_orphan = candidates.contains(&id)
                && is_metabolite_kind(&node.kind)
                && node.manual_duplicate != Some(true)
                && node.shared_instance != Some(true)
                && !referenced.contains(&id);
            if reaction_owned || became_orphan {
                self.delete_node(&id);
                removed.push(id);
            } else if owned && referenced.contains(&id) {
                // The node has become genuinely shared through an explicit
                // merge. Clear stale placement ownership so re-adding the
                // removed reaction gets a fresh visual instance instead of
                // accidentally reusing this node.
                let node = self.g.nodes.get_mut(&id).expect("listed");
                if primary_placed {
                    node.reaction_local_for = None;
                }
                if satellite_owned {
                    node.reaction_id = None;
                }
                node.reaction_local = Some(false);
                node.shared_instance = Some(true);
            }
        }

        let nodes = &self.g.nodes;
        let reactions = &self.g.reactions;
        self.g.medium.nodes.retain(|id| nodes.contains_key(id));
        self.g.medium.exchanges.retain(|id| id != reaction_id && reactions.contains_key(id));

        Some(RemovedReaction { reaction, nodes: removed })
    }

    /// `moveNode`: move a metabolite node, clamped to its compartment, with
    /// the segment ends attached to it. A straight reaction has no place of
    /// its own: it goes where its two metabolites go.
    pub fn move_node(&mut self, node_id: &str, x: f64, y: f64) -> Option<&Node> {
        let moved = crate::drag::move_layout_node(self, node_id, x, y);
        if moved {
            crate::reaction_edit::straighten_linear_reactions(self, Some(&[node_id.to_string()]));
            return self.g.nodes.get(node_id);
        }
        None
    }

    /// `setNodeRadius`: compatibility setter. Coxeter has only two metabolite
    /// sizes, so a requested radius snaps to the nearest visual role.
    pub fn set_node_radius(&mut self, node_id: &str, radius: f64) -> Option<&Node> {
        let node = self.g.nodes.get(node_id)?;
        if !is_primary_kind(&node.kind) {
            return None;
        }
        let primary = if radius.is_finite() {
            (radius - SIZES.primary_radius).abs() <= (radius - SIZES.currency_radius).abs()
        } else {
            node.primary != Some(false)
        };
        self.set_node_primary(node_id, primary)
    }

    /// All visual instances representing one biological metabolite. Multiple
    /// nodes may deliberately share the same merged id.
    pub fn nodes_for_metabolite(&self, merged_id: &str) -> Vec<&Node> {
        if merged_id.is_empty() {
            return vec![];
        }
        self.g
            .nodes
            .values()
            .filter(|n| is_primary_kind(&n.kind) && n.merged_id.as_deref() == Some(merged_id))
            .collect()
    }

    /// Another visual instance of the same biological metabolite whose circle
    /// overlaps the dragged node. Different metabolites can overlap without
    /// being merged.
    pub fn overlapping_metabolite_target(&self, node_id: &str) -> Option<&Node> {
        let source = self.g.nodes.get(node_id)?;
        if !is_metabolite_kind(&source.kind) {
            return None;
        }
        let merged = source.merged_id.as_deref().filter(|m| !m.is_empty())?;
        let mut best: Option<(&Node, f64)> = None;
        for target in self.g.nodes.values() {
            if target.id == source.id || target.hidden == Some(true) {
                continue;
            }
            if !is_metabolite_kind(&target.kind) || target.merged_id.as_deref() != Some(merged) {
                continue;
            }
            let distance = js::hypot(target.x - source.x, target.y - source.y);
            let threshold = js::max(8.0, source.r + target.r);
            if distance > threshold {
                continue;
            }
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((target, distance));
            }
        }
        best.map(|(n, _)| n)
    }

    /// `mergeMetaboliteNodes`: merge two visual instances of the same
    /// biological metabolite. All branches that ended at the source are
    /// rewired to the target, then the source visual node is removed. The
    /// biological model is never modified.
    pub fn merge_metabolite_nodes(&mut self, source_id: &str, target_id: &str) -> Option<MergeResult> {
        if source_id == target_id {
            return None;
        }
        let source = self.g.nodes.get(source_id)?.clone();
        let target = self.g.nodes.get(target_id)?.clone();
        if !is_metabolite_kind(&source.kind) || !is_metabolite_kind(&target.kind) {
            return None;
        }
        let merged = source.merged_id.as_deref().filter(|m| !m.is_empty())?;
        if Some(merged) != target.merged_id.as_deref() {
            return None;
        }
        let dx = target.x - source.x;
        let dy = target.y - source.y;
        let mut rewired = 0;
        for i in 0..self.g.segments.len() {
            if self.g.segments[i].node_id.as_deref() != Some(source_id) {
                continue;
            }
            self.g.segments[i].node_id = Some(target_id.to_string());
            let seg = &self.g.segments[i];
            let key = if seg.is_substrate() {
                SegEnd::From
            } else if seg.is_product() {
                SegEnd::To
            } else {
                let from_d = js::hypot(seg.from.x - source.x, seg.from.y - source.y);
                let to_d = js::hypot(seg.to.x - source.x, seg.to.y - source.y);
                if from_d <= to_d { SegEnd::From } else { SegEnd::To }
            };
            self.set_seg(i, key, Point::new(target.x, target.y));
            // b1 is the metabolite-side Escher control point.
            if let Some(b1) = self.g.segments[i].b1 {
                self.g.segments[i].b1 = Some(Point::new(b1.x + dx, b1.y + dy));
            }
            rewired += 1;
        }

        self.delete_node(source_id);
        self.g.medium.nodes.retain(|id| id != source_id);
        if target.kind == NodeKind::Medium && !self.g.medium.nodes.iter().any(|id| id == target_id) {
            self.g.medium.nodes.push(target_id.to_string());
        }
        {
            let t = self.g.nodes.get_mut(target_id).expect("target exists");
            t.hidden = Some(false);
            t.shared_instance = Some(true);
        }
        crate::reaction_edit::replace_linear_end(self, source_id, target_id);
        crate::reaction_edit::straighten_linear_reactions(self, Some(&[target_id.to_string()]));
        let target = self.g.nodes[target_id].clone();
        Some(MergeResult { source, target, rewired })
    }

    /// `addMetabolite`: a free-standing visual instance of a metabolite.
    pub fn add_metabolite(&mut self, community: &CommunityModel, merged_id: &str, x: f64, y: f64) -> Option<&Node> {
        let metabolite = community.merged.metabolites.get(merged_id)?;
        let owner = community.metabolite_owner.get(merged_id)?;
        let species = owner.species_id.as_deref().filter(|s| !s.is_empty())?;
        let base = format!("met:{merged_id}:manual");
        let mut n = 1;
        let mut id = format!("{base}:{n}");
        while self.g.nodes.contains_key(&id) {
            n += 1;
            id = format!("{base}:{n}");
        }
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let mut node = blank_node(&id, NodeKind::Metabolite);
        node.merged_id = Some(merged_id.to_string());
        node.local_id = Some(owner.original_id.clone());
        node.base = Some(crate::place_reaction::local_metabolite_base(community, merged_id, Some(metabolite)));
        node.name = Some(metabolite.name.clone());
        node.formula = Some(metabolite.formula.clone());
        node.compartment = Some(owner.compartment.clone());
        node.visual_compartment = Some(owner.compartment.clone());
        node.cell_id = Some(species.to_string());
        node.x = x;
        node.y = y;
        node.r = SIZES.primary_radius;
        node.primary = Some(true);
        node.manual_duplicate = Some(true);
        node.reaction_local = Some(false);
        node.shared_instance = Some(false);
        self.g.nodes.insert(id.clone(), node);
        self.g.nodes.get(&id)
    }

    /// Merge a dragged metabolite if it currently overlaps an identical copy.
    pub fn merge_overlapping_metabolite(&mut self, node_id: &str) -> Option<MergeResult> {
        let target = self.overlapping_metabolite_target(node_id)?.id.clone();
        self.merge_metabolite_nodes(node_id, &target)
    }

    /// `duplicateMetaboliteNode`: duplicate one metabolite graphically. The
    /// copy receives its own node id, position, radius and visibility while
    /// retaining the same merged/local identity used for reaction discovery.
    pub fn duplicate_metabolite_node(&mut self, node_id: &str, opts: &DuplicateOptions) -> Option<&Node> {
        let source = self.g.nodes.get(node_id)?.clone();
        if !is_primary_kind(&source.kind) {
            return None;
        }
        let id = opts.id.clone().unwrap_or_else(|| next_duplicate_node_id(&self.g.nodes, &source));
        if self.g.nodes.contains_key(&id) {
            return None;
        }
        let offset = opts.offset.filter(|o| o.is_finite()).unwrap_or(42.0);
        let rx = opts.x.filter(|v| v.is_finite()).unwrap_or(source.x + offset);
        let ry = opts.y.filter(|v| v.is_finite()).unwrap_or(source.y + offset);
        let mut copy = source.clone();
        copy.id = id.clone();
        copy.hidden = Some(false);
        copy.manual_duplicate = Some(true);
        copy.duplicate_of = Some(source.duplicate_of.clone().unwrap_or_else(|| source.id.clone()));
        copy.reaction_id = None;
        copy.reaction_local_for = None;
        copy.reaction_local = Some(false);
        copy.shared_instance = Some(false);
        let medium = copy.kind == NodeKind::Medium;
        self.g.nodes.insert(id.clone(), copy);
        crate::drag::move_layout_node(self, &id, rx, ry);
        if medium && !self.g.medium.nodes.contains(&id) {
            self.g.medium.nodes.push(id.clone());
        }
        self.g.nodes.get(&id)
    }

    /// `setNodePrimary`: mark one visual metabolite instance as primary or
    /// secondary. Map-only; the role also fixes its radius (13 or 7 px).
    pub fn set_node_primary(&mut self, node_id: &str, primary: bool) -> Option<&Node> {
        let node = self.g.nodes.get_mut(node_id)?;
        if !is_primary_kind(&node.kind) {
            return None;
        }
        node.primary = Some(primary);
        node.visual_role = Some(if primary { "primary" } else { "secondary" }.into());
        node.r = if primary { SIZES.primary_radius } else { SIZES.currency_radius };
        Some(node)
    }

    /// `setNodeVisualRole(id, 'primary' | 'secondary')`.
    pub fn set_node_visual_role(&mut self, node_id: &str, role: &str) -> Option<&Node> {
        match role {
            "primary" => self.set_node_primary(node_id, true),
            "secondary" => self.set_node_primary(node_id, false),
            _ => None,
        }
    }

    /// Map-only colours of a cell. `fill` is the colour of the cell body,
    /// which is its outer compartment, so it is stored there and the two
    /// cannot disagree; `stroke` draws the cell boundary.
    pub fn set_cell_colors(&mut self, cell_id: &str, fill: &ColorChange, stroke: &ColorChange) -> Option<&MapCell> {
        let cell = self.g.cells.iter_mut().find(|c| c.id == cell_id)?;
        let outer = cell.outer_compartment.clone();
        let body = cell
            .compartments
            .iter()
            .position(|c| Some(&c.id) == outer.as_ref())
            .or_else(|| cell.compartments.iter().position(|c| c.parent_id.is_none()));
        if let Some(b) = body {
            apply_map_color(&mut cell.compartments[b].fill, fill);
        }
        apply_map_color(&mut cell.stroke, stroke);
        Some(cell)
    }

    /// Map-only colours of one compartment: `fill` for its space, `stroke` for
    /// its membrane.
    pub fn set_compartment_colors(
        &mut self,
        cell_id: &str,
        compartment_id: &str,
        fill: &ColorChange,
        stroke: &ColorChange,
    ) -> Option<&MapCompartment> {
        let cell = self.g.cells.iter_mut().find(|c| c.id == cell_id)?;
        let comp = cell.compartments.iter_mut().find(|c| c.id == compartment_id)?;
        apply_map_color(&mut comp.fill, fill);
        apply_map_color(&mut comp.stroke, stroke);
        Some(comp)
    }

    /// `setCofactorStyle(style)`: `quiet`, or `None` for full size. Returns
    /// the style kept (anything unknown is the default).
    pub fn set_cofactor_style(&mut self, style: Option<&str>) -> Option<&'static str> {
        let kept = normalize_cofactor_style(style.map(Value::from).as_ref());
        match kept {
            Some(style) => self.g.extra.insert(COFACTOR_STYLE.into(), Value::from(style)),
            None => self.g.extra.remove(COFACTOR_STYLE),
        };
        kept
    }

    /// `setNodeStyle({ fill, stroke })`: the colours every metabolite circle
    /// is drawn with unless it has its own; `Reset` gives one back to the
    /// theme.
    pub fn set_node_style(&mut self, fill: &ColorChange, stroke: &ColorChange) -> coxeter_core::mapdata::NodeStyle {
        let mut style = self.g.node_style();
        apply_map_color(&mut style.fill, fill);
        apply_map_color(&mut style.stroke, stroke);
        self.g.set_node_style(&style);
        style
    }

    /// `setNodeColors(nodeIds, { fill, stroke })`: colour some metabolite
    /// circles; `Reset` returns one to the map's. The ids of the nodes changed.
    pub fn set_node_colors(&mut self, node_ids: &[String], fill: &ColorChange, stroke: &ColorChange) -> Vec<String> {
        let mut changed = Vec::new();
        for id in node_ids {
            let Some(node) = self.g.nodes.get_mut(id) else { continue };
            if !is_metabolite_kind(&node.kind) {
                continue;
            }
            apply_extra_color(&mut node.extra, "fill", fill);
            apply_extra_color(&mut node.extra, "stroke", stroke);
            changed.push(id.clone());
        }
        changed
    }

    /// `setReactionColor(reactionIds, color)`: the colour some reactions are
    /// drawn in when lines are not coloured by flux, over their pathway's;
    /// `Reset` gives them back their pathway's. The ids changed.
    pub fn set_reaction_color(&mut self, reaction_ids: &[String], color: &ColorChange) -> Vec<String> {
        let mut changed = Vec::new();
        for id in reaction_ids {
            let Some(rxn) = self.g.reactions.get_mut(id) else { continue };
            apply_extra_color(&mut rxn.extra, "color", color);
            changed.push(id.clone());
        }
        changed
    }

    /// `geometryTarget(cellId, compartmentId)`.
    pub fn geometry_target(&self, cell_id: Option<&str>, compartment_id: &str) -> Option<GeometryTarget> {
        crate::geometry_edit::geometry_target(self, cell_id, compartment_id)
    }

    /// `editGeometryRect`: move or resize a compartment, a cell or the
    /// environment. A compartment that moves takes one end of a straight
    /// reaction with it.
    pub fn edit_geometry_rect(
        &mut self,
        cell_id: Option<&str>,
        compartment_id: &str,
        rect: Rect,
    ) -> Option<crate::geometry_edit::GeometryEdit> {
        let lines = straight_lines(self);
        let result = crate::geometry_edit::edit_geometry_rect(self, cell_id, compartment_id, rect);
        if result.is_some() {
            restraighten(self, &lines);
        }
        result
    }

    /// `editCompartmentShape`: corner softness and ovalness of one
    /// compartment (see `geometry::compartment_shape`).
    pub fn edit_compartment_shape(
        &mut self,
        cell_id: Option<&str>,
        compartment_id: &str,
        shape: crate::geometry_edit::ShapePatch,
    ) -> Option<crate::geometry_edit::ShapeEdit> {
        let lines = straight_lines(self);
        let result = crate::geometry_edit::edit_compartment_shape(self, cell_id, compartment_id, shape);
        if result.is_some() {
            restraighten(self, &lines);
        }
        result
    }

    /// `setCompartmentLabelPosition`: where along its membrane a
    /// compartment's label sits, as a fraction of its box.
    pub fn set_compartment_label_position(
        &mut self,
        cell_id: Option<&str>,
        compartment_id: &str,
        x: f64,
        y: f64,
    ) -> Option<Point> {
        let target = self.geometry_target(cell_id, compartment_id)?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (rect, corners, extra) = match target {
            GeometryTarget::Environment => {
                let env = &mut self.g.environment;
                (env.rect, None, &mut env.extra)
            }
            GeometryTarget::Compartment { cell, compartment } => {
                let comp = &mut self.g.cells[cell].compartments[compartment];
                (comp.rect, Some(compartment_corners(comp)), &mut comp.extra)
            }
        };
        let p = match corners {
            Some(c) => closest_perimeter_point_corners(&rect, Point::new(x, y), c),
            None => closest_perimeter_point(&rect, Point::new(x, y), 26.0),
        };
        let pos = Point::new((p.x - rect.x) / rect.w, (p.y - rect.y) / rect.h);
        extra.insert("labelPosition".into(), json!({ "x": pos.x, "y": pos.y }));
        Some(pos)
    }

    /// Change one label's orbit without moving its circle or attached branches.
    pub fn set_metabolite_label_angle(&mut self, node_id: &str, angle: f64) -> Option<&Node> {
        let node = self.g.nodes.get_mut(node_id)?;
        if !is_metabolite_kind(&node.kind) || !angle.is_finite() {
            return None;
        }
        node.label_angle = Some(metabolite_label_angle(Some(angle)));
        Some(node)
    }

    /// Hide/show one metabolite graphically. Its model membership is untouched.
    pub fn set_node_hidden(&mut self, node_id: &str, hidden: bool) -> Option<&Node> {
        let node = self.g.nodes.get_mut(node_id)?;
        if !is_primary_kind(&node.kind) {
            return None;
        }
        node.hidden = Some(hidden);
        Some(node)
    }

    /// How many primary metabolites are hidden.
    pub fn hidden_node_count(&self) -> usize {
        self.g.nodes.values().filter(|n| n.hidden == Some(true) && is_primary_kind(&n.kind)).count()
    }

    /// Show every hidden metabolite; the ids that changed.
    pub fn show_all_hidden_nodes(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        for node in self.g.nodes.values_mut() {
            if node.hidden != Some(true) || !is_primary_kind(&node.kind) {
                continue;
            }
            node.hidden = Some(false);
            changed.push(node.id.clone());
        }
        changed
    }

    /// Is this node one the user is allowed to move?
    pub fn can_move_node(&self, node_id: &str) -> bool {
        crate::drag::is_draggable_node(self.g.nodes.get(node_id))
    }

    // Free map text

    /// `addText(text, x, y, opts)`.
    pub fn add_text(&mut self, text: &str, x: f64, y: f64, opts: &TextOptions) -> Option<&MapText> {
        let value = text.trim().to_string();
        if value.is_empty() || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let id = opts.id.clone().unwrap_or_else(|| next_text_id(&self.g.texts));
        if self.g.texts.contains_key(&id) {
            return None;
        }
        let t = MapText {
            id: id.clone(),
            text: value,
            x,
            y,
            font_size: Some(clamp_text_size(opts.font_size.unwrap_or(coxeter_core::text_style::TEXT_SIZE.default))),
            font_family: Some(normalize_text_font(opts.font_family.as_deref().unwrap_or("")).to_string()),
            extra: Map::new(),
        };
        self.g.texts.insert(id.clone(), t);
        self.g.texts.get(&id)
    }

    /// `updateText(id, text)`: replace a text's words. Blank text is refused
    /// (`None`), so a text cannot be emptied in place.
    pub fn update_text(&mut self, id: &str, text: &str) -> Option<&MapText> {
        let value = text.trim();
        let t = self.g.texts.get_mut(id)?;
        if value.is_empty() {
            return None;
        }
        t.text = value.to_string();
        Some(t)
    }

    /// `styleText(id, {fontSize, fontFamily})`: a size in px (8–144) and/or a
    /// font key. A field left out is left as it is.
    pub fn style_text(&mut self, id: &str, font_size: Option<f64>, font_family: Option<&str>) -> Option<&MapText> {
        let t = self.g.texts.get_mut(id)?;
        if let Some(size) = font_size {
            t.font_size = Some(clamp_text_size(size));
        }
        if let Some(family) = font_family {
            t.font_family = Some(normalize_text_font(family).to_string());
        }
        Some(t)
    }

    /// `moveText(id, x, y)`: put a text at `(x, y)`. `None` for an unknown id
    /// or a point that is not finite.
    pub fn move_text(&mut self, id: &str, x: f64, y: f64) -> Option<&MapText> {
        let t = self.g.texts.get_mut(id)?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        t.x = x;
        t.y = y;
        Some(t)
    }

    /// `removeText(id)`: take a text off the map, returning it.
    pub fn remove_text(&mut self, id: &str) -> Option<MapText> {
        self.g.texts.shift_remove(id)
    }

    /// The box a text is selected and aligned by.
    pub fn text_box(&self, id: &str) -> Option<TextBox> {
        self.g.texts.get(id).map(crate::multi_selection::text_box)
    }

    // Label settings

    /// Which label source a kind of object shows (`metabolite`: `base`,
    /// anything else: `id`, unless the map says otherwise).
    pub fn label_source(&self, kind: &str) -> String {
        self.g
            .label_settings
            .as_ref()
            .and_then(|s| s.get(kind).cloned())
            .unwrap_or_else(|| if kind == "metabolite" { "base".into() } else { "id".into() })
    }

    /// Set the map-wide label source of a kind of object.
    pub fn set_label_source(&mut self, kind: &str, source: &str) {
        self.g.label_settings.get_or_insert_with(IndexMap::new).insert(kind.to_string(), source.to_string());
    }

    /// Set (or, with `None`, clear) the label source of one node or reaction.
    pub fn set_object_label_source(&mut self, kind: &str, id: &str, source: Option<&str>) -> bool {
        let extra = if kind == "metabolite" {
            self.g.nodes.get_mut(id).map(|n| &mut n.extra)
        } else {
            self.g.reactions.get_mut(id).map(|r| &mut r.extra)
        };
        let Some(extra) = extra else { return false };
        match source {
            Some(s) => {
                extra.insert("labelSource".into(), Value::String(s.to_string()));
            }
            None => {
                extra.shift_remove("labelSource");
            }
        }
        true
    }

    // Removal and subsets

    /// A primary/medium metabolite may be removed only when no segment uses it.
    pub fn can_remove_node(&self, node_id: &str) -> bool {
        let Some(node) = self.g.nodes.get(node_id) else { return false };
        is_primary_kind(&node.kind) && self.segments_at(node_id).is_empty()
    }

    /// `removeCompartment(cellId, compartmentId)`: take a compartment of a
    /// cell off the drawing, with everything that cannot be drawn without it:
    /// the compartments nested in it, the reactions that run in any of them or
    /// cross their membranes, the metabolites drawn there and the texts
    /// written inside it. A cell's outermost compartment is the cell, so the
    /// whole cell goes. The biological model is untouched, and the compartment
    /// can be drawn again.
    pub fn remove_compartment(&mut self, cell_id: &str, compartment_id: &str) -> Option<RemovedCompartment> {
        let ci = self.g.cells.iter().position(|c| c.id == cell_id)?;
        let compartment = self.g.cells[ci].compartments.iter().find(|c| c.id == compartment_id)?.clone();
        let cell_removed = compartment.parent_id.is_none();
        let subtree = crate::geometry_edit::descendant_ids(&self.g.cells[ci], compartment_id);

        let mut reactions = Vec::new();
        let mut nodes = Vec::new();
        let doomed: Vec<String> = self
            .g
            .reactions
            .values()
            .filter(|r| crate::geometry_edit::reaction_in_subtree(r, cell_id, &subtree))
            .map(|r| r.id.clone())
            .collect();
        for id in doomed {
            let Some(result) = self.remove_reaction(&id) else { continue };
            reactions.push(id);
            nodes.extend(result.nodes);
        }
        // What is left inside: metabolites drawn on their own, or kept as shared.
        let inside: Vec<String> = self
            .g
            .nodes
            .values()
            .filter(|n| n.cell_id.as_deref() == Some(cell_id) && n.compartment.as_ref().is_some_and(|c| subtree.contains(c)))
            .map(|n| n.id.clone())
            .collect();
        for id in inside {
            self.delete_node(&id);
            nodes.push(id);
        }
        let live = &self.g.nodes;
        let keep: HashSet<String> = live.keys().cloned().collect();
        self.retain_segments(|seg| seg.node_id.as_deref().is_none_or(|n| n.is_empty() || keep.contains(n)));
        let live = &self.g.nodes;
        self.g.medium.nodes.retain(|id| live.contains_key(id));
        // The texts written inside it are its pathways' names: left behind,
        // they labelled an empty space.
        let area = compartment.rect;
        let written: Vec<String> = self
            .g
            .texts
            .values()
            .filter(|t| t.x.is_finite() && t.y.is_finite())
            .filter(|t| !(t.x < area.x || t.x > area.x + area.w || t.y < area.y || t.y > area.y + area.h))
            .map(|t| t.id.clone())
            .collect();
        let mut texts = Vec::new();
        for id in written {
            if self.remove_text(&id).is_some() {
                texts.push(id);
            }
        }

        let cell = &self.g.cells[ci];
        let label = if cell_removed {
            if cell.name.is_empty() { cell_id.to_string() } else { cell.name.clone() }
        } else if compartment.label.is_empty() {
            compartment_id.to_string()
        } else {
            compartment.label.clone()
        };
        // The subtree in the order it was found: the compartment, then what
        // is nested in it.
        let compartment_ids = crate::geometry_edit::descendant_list(cell, compartment_id);
        if cell_removed {
            self.g.cells.remove(ci);
        } else {
            self.g.cells[ci].compartments.retain(|c| !subtree.contains(&c.id));
        }
        self.refresh_boundaries();

        Some(RemovedCompartment { cell_id: cell_id.to_string(), compartment_ids, cell_removed, label, reactions, nodes, texts })
    }

    /// Remove one orphan metabolite from the drawing only.
    pub fn remove_node(&mut self, node_id: &str) -> Option<Node> {
        if !self.can_remove_node(node_id) {
            return None;
        }
        let node = self.delete_node(node_id);
        self.g.medium.nodes.retain(|id| id != node_id);
        node
    }

    /// `subsetByReactions`: a new map drawing only the listed reactions.
    ///
    /// Segments follow their reaction. A node is kept when a surviving
    /// segment still ends at it (a metabolite nobody reacts with has nothing
    /// to say on a map), so the result is always internally consistent,
    /// including for the empty selection. The geometry is copied, so the two
    /// maps can be edited independently.
    pub fn subset_by_reactions(&self, reaction_ids: &[String]) -> EditableMap {
        let keep: HashSet<&str> = reaction_ids.iter().map(String::as_str).collect();
        let reactions: IndexMap<String, MapReaction> = self
            .g
            .reactions
            .keys()
            .filter(|id| keep.contains(id.as_str()))
            .map(|k| (k.clone(), self.reaction_with_current_lanes(k).expect("listed")))
            .collect();
        let mut segments = Vec::new();
        let mut links = Vec::new();
        for (seg, link) in self.g.segments.iter().zip(&self.links) {
            if reactions.contains_key(&seg.reaction_id) {
                segments.push(seg.clone());
                links.push(*link);
            }
        }
        let referenced: HashSet<&str> = segments.iter().filter_map(|s| s.node_id.as_deref()).filter(|n| !n.is_empty()).collect();
        let nodes: IndexMap<String, Node> =
            self.g.nodes.iter().filter(|(id, _)| referenced.contains(id.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let medium = Medium {
            nodes: self.g.medium.nodes.iter().filter(|id| nodes.contains_key(*id)).cloned().collect(),
            exchanges: self.g.medium.exchanges.iter().filter(|id| reactions.contains_key(*id)).cloned().collect(),
            extra: Map::new(),
        };
        let g = MapGraph {
            version: Value::from(1),
            canvas: self.g.canvas,
            environment: self.g.environment.clone(),
            cells: self.g.cells.clone(),
            medium,
            nodes,
            reactions,
            segments,
            texts: self.g.texts.clone(),
            // `new EditableMap({...})` is not handed the label settings.
            label_settings: None,
            node_style: None,
            extra: Map::new(),
        };
        EditableMap::from_graph(g, Some(links))
    }

    /// A copy with nothing drawn in it, keeping the shell.
    pub fn cleared(&self) -> EditableMap {
        self.subset_by_reactions(&[])
    }
}

// Helpers

/// Where a straight reaction and its two ends stood before a geometry edit.
struct StraightLine {
    rxn: Point,
    from: Point,
    to: Point,
}

/// `straightLines(map)`: where each straight reaction and its two ends
/// stand, before a geometry edit. Straightening redraws a line's branches
/// from scratch, so after the edit only the lines it pulled out of true are
/// straightened: one end moved and the reaction did not move with it.
/// Straightening every line on the map would reset the bends drawn into
/// lines nowhere near the compartment being edited.
fn straight_lines(map: &EditableMap) -> IndexMap<String, StraightLine> {
    let mut out = IndexMap::new();
    for rxn in map.g.reactions.values() {
        let Some(linear) = &rxn.linear else { continue };
        let from = linear.from.as_deref().and_then(|id| map.g.nodes.get(id));
        let to = linear.to.as_deref().and_then(|id| map.g.nodes.get(id));
        let (Some(from), Some(to)) = (from, to) else { continue };
        out.insert(rxn.id.clone(), StraightLine { rxn: Point::new(rxn.x, rxn.y), from: from.pos(), to: to.pos() });
    }
    out
}

/// `restraighten(map, lines)`.
fn restraighten(map: &mut EditableMap, lines: &IndexMap<String, StraightLine>) {
    let shift = |now: Point, before: Point| Point::new(now.x - before.x, now.y - before.y);
    let same = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
    for (id, before) in lines {
        let Some(rxn) = map.g.reactions.get(id) else { continue };
        let Some(linear) = &rxn.linear else { continue };
        let from = linear.from.as_deref().and_then(|n| map.g.nodes.get(n));
        let to = linear.to.as_deref().and_then(|n| map.g.nodes.get(n));
        let (Some(from), Some(to)) = (from, to) else { continue };
        let moved = shift(Point::new(rxn.x, rxn.y), before.rxn);
        if !same(shift(from.pos(), before.from), moved) || !same(shift(to.pos(), before.to), moved) {
            crate::reaction_edit::straighten_reaction(map, id);
        }
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

/// The links a freshly built or freshly loaded map has: what
/// `relinkAxisEndpoints` (or `buildSegments`) makes the same object.
pub(crate) fn initial_links(g: &MapGraph) -> Vec<SegLinks> {
    g.segments
        .iter()
        .map(|seg| match g.reactions.get(&seg.reaction_id).and_then(|r| r.axis.as_ref()) {
            Some(_) => axis_links(seg),
            None => (None, None),
        })
        .collect()
}

fn apply_map_color(target: &mut Option<String>, change: &ColorChange) {
    match change {
        ColorChange::Keep => {}
        ColorChange::Reset => *target = None,
        ColorChange::Set(value) => {
            if let Some(c) = normalize_hex_color(value) {
                *target = Some(c);
            }
        }
    }
}

/// `applyMapColor(target, key, value)` on a field kept among an object's
/// other fields.
fn apply_extra_color(extra: &mut Map<String, Value>, key: &str, change: &ColorChange) {
    let mut current = extra.get(key).and_then(Value::as_str).map(str::to_string);
    let before = current.clone();
    apply_map_color(&mut current, change);
    if current == before {
        return;
    }
    match current {
        Some(c) => {
            extra.insert(key.into(), Value::String(c));
        }
        None => {
            extra.shift_remove(key);
        }
    }
}

fn migrate_reaction_local_ownership(nodes: &mut IndexMap<String, Node>) {
    for node in nodes.values_mut() {
        if !is_primary_kind(&node.kind) {
            continue;
        }
        if node.reaction_local != Some(true) || node.reaction_id.as_deref().unwrap_or("").is_empty() {
            continue;
        }
        if node.reaction_local_for.as_deref().is_some_and(|s| !s.is_empty()) {
            continue;
        }
        node.reaction_local_for = node.reaction_id.take();
    }
}

/// Give every drawn compartment a readable name and a visual identity.
///
/// A map saved from a model whose compartments are named after their own ids
/// (`"c": "c"`, which is what cobrapy writes for an SBML compartment with no
/// name) has "c" and "h" stored as its labels, and its `i` and `s` drawn as
/// unknown or as the shared medium. Nothing in a file should have to be
/// edited for the cytosol to say Cytosol.
pub(crate) fn name_cell_compartments(cells: &mut [MapCell]) {
    let open = membrane_style(SHARED_COMPARTMENT, &Names::new()).as_str();
    for cell in cells {
        for comp in &mut cell.compartments {
            if comp.id.is_empty() {
                continue;
            }
            let mut names = Names::new();
            names.insert(comp.id.clone(), comp.label.clone());
            let resolved = cell_compartment_visual_id(&comp.id, &names);
            if comp.visual_id.is_empty() || comp.visual_id == comp.id {
                // The one membrane a stale id gets wrong: a cell's own `s` was
                // stroked like the open shared medium.
                if resolved == "es" && comp.membrane == open {
                    comp.membrane = membrane_style(&resolved, &Names::new()).as_str().into();
                }
                comp.visual_id = resolved;
            }
            if comp.label.is_empty() || comp.label == comp.id {
                comp.label = cell_compartment_name(&comp.id, &names, "en");
            }
        }
    }
}

/// Maps saved before the labels were fixed carry the species in front of
/// every hand-placed metabolite: `base` is the label, and it was read off the
/// merged id, so a node of the model `iDemo` is named `iDemo@g6p`. Mend them on
/// the way in.
fn strip_species_from_labels(nodes: &mut IndexMap<String, Node>) {
    for node in nodes.values_mut() {
        if !is_metabolite_kind(&node.kind) {
            continue;
        }
        let Some(base) = node.base.clone() else { continue };
        let merged = node.merged_id.clone().unwrap_or_default();
        let prefix = merged.find('@').map(|i| merged[..i].to_string());
        let owners = [node.cell_id.clone(), prefix];
        let owner = owners.iter().flatten().find(|id| !id.is_empty() && base.starts_with(&format!("{id}@")));
        if let Some(owner) = owner {
            node.base = Some(base[owner.len() + 1..].to_string());
        }
    }
}

fn normalize_visual_roles(nodes: &mut IndexMap<String, Node>) {
    for node in nodes.values_mut() {
        if !is_metabolite_kind(&node.kind) {
            continue;
        }
        if node.kind == NodeKind::Currency {
            node.primary = Some(false);
            node.visual_role = Some("secondary".into());
            node.r = SIZES.currency_radius;
            continue;
        }
        match node.visual_role.as_deref() {
            Some("secondary") => node.primary = Some(false),
            Some("primary") => node.primary = Some(true),
            _ if node.primary == Some(false) => node.visual_role = Some("secondary".into()),
            _ => {
                node.primary = Some(true);
                node.visual_role = Some("primary".into());
            }
        }
        node.r = if node.primary == Some(true) { SIZES.primary_radius } else { SIZES.currency_radius };
    }
}

fn normalize_metabolite_label_angles(nodes: &mut IndexMap<String, Node>) {
    for node in nodes.values_mut() {
        if !is_metabolite_kind(&node.kind) {
            continue;
        }
        if let Some(angle) = node.label_angle {
            node.label_angle = angle.is_finite().then(|| metabolite_label_angle(Some(angle)));
        }
    }
}

/// `normalizeRect`: a rectangle with its corners in order, or None.
pub(crate) fn normalize_rect(rect: Option<Rect>) -> Option<Rect> {
    let r = rect?;
    if ![r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x2, y2) = (r.x + r.w, r.y + r.h);
    Some(Rect::new(js::min(r.x, x2), js::min(r.y, y2), r.w.abs(), r.h.abs()))
}

fn clamp_cell_rect_to_canvas(map: &EditableMap, requested: Rect) -> Rect {
    let pad = 18.0;
    let (min_w, min_h) = (180.0, 120.0);
    let env = map.g.environment.rect;
    let max_w = js::max(min_w, env.w - pad * 2.0);
    let max_h = js::max(min_h, env.h - pad * 2.0);
    let w = js::min(max_w, js::max(min_w, requested.w));
    let h = js::min(max_h, js::max(min_h, requested.h));
    let x = js::min(js::max(requested.x, env.x + pad), env.x + env.w - pad - w);
    let y = js::min(js::max(requested.y, env.y + pad), env.y + env.h - pad - h);
    Rect::new(x, y, w, h)
}

fn all_finite(r: &Rect) -> bool {
    [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite())
}

fn find_free_cell_rect(map: &mut EditableMap, requested: Option<Rect>) -> Rect {
    if let Some(r) = requested.filter(all_finite) {
        return r;
    }
    let (width, height) = (map.g.canvas.width, map.g.canvas.height);
    let w = js::min(520.0, js::max(260.0, width * 0.42));
    let h = js::min(380.0, js::max(200.0, height * 0.42));
    let gap = 36.0;
    let mut y = 50.0;
    while y + h <= height - 30.0 {
        let mut x = 50.0;
        while x + w <= width - 30.0 {
            let candidate = Rect::new(x, y, w, h);
            if map.g.cells.iter().all(|c| !rects_overlap(&candidate, &c.rect, gap)) {
                return candidate;
            }
            x += 50.0;
        }
        y += 50.0;
    }
    // If no free slot fits, grow the canvas to the right and use the new area.
    let x = map.g.cells.iter().fold(40.0, |m, c| js::max(m, c.rect.x + c.rect.w)) + gap;
    map.g.canvas.width = js::max(map.g.canvas.width, x + w + 50.0);
    if map.g.environment.hidden == Some(true) {
        map.g.environment.rect.w = map.g.canvas.width;
    }
    Rect::new(x, 50.0, w, h)
}

fn find_free_child_rect(cell: &MapCell, parent_id: &str, requested: Option<Rect>) -> Option<Rect> {
    let parent = cell.compartments.iter().find(|c| c.id == parent_id)?;
    let p = parent.rect;
    let children: Vec<&MapCompartment> =
        cell.compartments.iter().filter(|c| c.parent_id.as_deref() == Some(parent_id)).collect();

    // Every new child is inset from its biological parent. This guarantees a
    // visible parent space around it (periplasm around cytosol,
    // mitochondrial intermembrane space around matrix, cytosol around
    // organelles, etc.).
    let shell_pad = js::max(10.0, js::min(28.0, js::min(p.w, p.h) * 0.055));
    if let Some(r) = requested.filter(all_finite) {
        return clamp_child_rect(r, &p, shell_pad);
    }
    let available_w = js::max(90.0, p.w - shell_pad * 2.0);
    let available_h = js::max(64.0, p.h - shell_pad * 2.0);
    let share = if children.is_empty() { 0.72 } else { 0.42 };
    let w = js::max(90.0, js::min(available_w, p.w * share));
    let h = js::max(64.0, js::min(available_h, p.h * share));
    let candidates: Vec<Rect> = [
        Rect::new(p.x + (p.w - w) / 2.0, p.y + (p.h - h) / 2.0, w, h),
        Rect::new(p.x + shell_pad, p.y + shell_pad, w, h),
        Rect::new(p.x + p.w - shell_pad - w, p.y + shell_pad, w, h),
        Rect::new(p.x + shell_pad, p.y + p.h - shell_pad - h, w, h),
        Rect::new(p.x + p.w - shell_pad - w, p.y + p.h - shell_pad - h, w, h),
    ]
    .iter()
    .filter_map(|r| clamp_child_rect(*r, &p, shell_pad))
    .collect();
    candidates
        .iter()
        .find(|r| children.iter().all(|c| !rects_overlap(r, &c.rect, 10.0)))
        .or_else(|| candidates.get(children.len() % candidates.len().max(1)))
        .copied()
}

fn clamp_child_rect(requested: Rect, parent: &Rect, pad: f64) -> Option<Rect> {
    let n = normalize_rect(Some(requested))?;
    let min_w = js::min(90.0, js::max(24.0, parent.w - pad * 2.0));
    let min_h = js::min(64.0, js::max(20.0, parent.h - pad * 2.0));
    let max_w = js::max(min_w, parent.w - pad * 2.0);
    let max_h = js::max(min_h, parent.h - pad * 2.0);
    let w = js::min(max_w, js::max(min_w, n.w));
    let h = js::min(max_h, js::max(min_h, n.h));
    let x = js::min(js::max(n.x, parent.x + pad), parent.x + parent.w - pad - w);
    let y = js::min(js::max(n.y, parent.y + pad), parent.y + parent.h - pad - h);
    Some(Rect::new(x, y, w, h))
}

fn find_free_root_rect(cell: &MapCell, requested: Option<Rect>) -> Rect {
    if let Some(r) = requested.filter(all_finite) {
        return r;
    }
    let roots = cell.compartments.iter().filter(|c| c.parent_id.is_none()).count() as f64;
    let p = cell.rect;
    let gap = 14.0;
    let w = js::max(90.0, (p.w - gap * (roots + 2.0)) / js::max(2.0, roots + 1.0));
    let h = js::max(64.0, p.h - gap * 2.0);
    Rect::new(p.x + gap + roots * (w + gap), p.y + gap, w, h)
}

pub(crate) fn union_rects(rects: &[Rect]) -> Option<Rect> {
    if rects.is_empty() {
        return None;
    }
    let x1 = js::min_n(rects.iter().map(|r| r.x));
    let y1 = js::min_n(rects.iter().map(|r| r.y));
    let x2 = js::max_n(rects.iter().map(|r| r.x + r.w));
    let y2 = js::max_n(rects.iter().map(|r| r.y + r.h));
    Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
}

pub(crate) fn rects_overlap(a: &Rect, b: &Rect, gap: f64) -> bool {
    !(a.x + a.w + gap <= b.x || b.x + b.w + gap <= a.x || a.y + a.h + gap <= b.y || b.y + b.h + gap <= a.y)
}

fn next_duplicate_node_id(nodes: &IndexMap<String, Node>, source: &Node) -> String {
    let root = source.duplicate_of.clone().unwrap_or_else(|| source.id.clone());
    let mut index = 2;
    let mut id = format!("{root}#{index}");
    while nodes.contains_key(&id) {
        index += 1;
        id = format!("{root}#{index}");
    }
    id
}

fn next_text_id(texts: &IndexMap<String, MapText>) -> String {
    let mut i = 1;
    while texts.contains_key(&format!("text-{i}")) {
        i += 1;
    }
    format!("text-{i}")
}

/// `deriveBoundaries`: every membrane in the drawing, as a first-class
/// object. Each carries who it belongs to, which two compartments it lies
/// between, and the rectangle it is drawn as.
pub fn derive_boundaries(g: &MapGraph) -> Vec<Boundary> {
    let env = &g.environment;
    let env_id = if env.id.is_empty() { SHARED_COMPARTMENT.to_string() } else { env.id.clone() };
    let mut out = vec![Boundary {
        id: environment_key(),
        kind: BoundaryKind::Environment,
        cell_id: None,
        inside: env_id.clone(),
        outside: None,
        compartment_id: env_id.clone(),
        parent_id: None,
        depth: -1.0,
        membrane: membrane_style(&env_id, &Names::new()).as_str().into(),
        rect: Some(env.rect),
        label: env.label.clone(),
    }];
    for cell in &g.cells {
        for comp in &cell.compartments {
            out.push(Boundary {
                id: boundary_key(Some(&cell.id), &comp.id),
                kind: if comp.parent_id.is_none() { BoundaryKind::Cell } else { BoundaryKind::Membrane },
                cell_id: Some(cell.id.clone()),
                inside: comp.id.clone(),
                // The outermost membrane of a cell faces the shared medium.
                outside: Some(comp.parent_id.clone().unwrap_or_else(|| SHARED_COMPARTMENT.into())),
                compartment_id: comp.id.clone(),
                parent_id: comp.parent_id.clone(),
                depth: comp.depth,
                membrane: if comp.membrane.is_empty() {
                    membrane_style(&comp.id, &Names::new()).as_str().into()
                } else {
                    comp.membrane.clone()
                },
                rect: Some(comp.rect),
                label: comp.label.clone(),
            });
        }
    }
    out
}

/// The environment's own label, as the constructor gives a map without one.
pub fn default_environment_label() -> String {
    let mut names = Names::new();
    names.insert(SHARED_COMPARTMENT.into(), "Shared medium".into());
    compartment_name(SHARED_COMPARTMENT, &names, "en")
}

/// `isSecondaryNode` over the map's own node record.
pub fn is_secondary(node: &Node) -> bool {
    is_secondary_node(node)
}

// Editing, by delegation

use crate::multi_selection::{Arranged, LineAlignment, Moved, Selection, SelectionRect};
use crate::place_reaction::{PlaceOptions, Placed};
use crate::reaction_edit::{ControlPoint, Handle, SideEntry};

impl EditableMap {
    /// `addReaction(community, id, opts)`: add one biological reaction to
    /// this map without rebuilding or solving.
    pub fn add_reaction(&mut self, community: &CommunityModel, reaction_id: &str, opts: &PlaceOptions) -> Option<Placed> {
        crate::place_reaction::place_reaction(self, community, reaction_id, opts)
    }

    /// Whether a reaction's centre, or a transporter along its membrane, may
    /// be moved.
    pub fn can_move_reaction(&self, reaction_id: &str) -> bool {
        crate::reaction_edit::can_move_reaction(self, reaction_id)
    }

    /// `moveReaction`: a transporter slides along its membrane, a straight
    /// reaction along its line, anything else moves with its skeleton.
    pub fn move_reaction(&mut self, reaction_id: &str, x: f64, y: f64) -> Option<&MapReaction> {
        crate::reaction_edit::move_reaction(self, reaction_id, x, y).then(|| &self.g.reactions[reaction_id])
    }

    /// Whether the backbone of a reaction may be turned about its centre
    /// (internal reactions only).
    pub fn can_rotate_reaction(&self, reaction_id: &str) -> bool {
        crate::reaction_edit::can_rotate_reaction(self, reaction_id)
    }
    /// Direction of a reaction's backbone as drawn, in radians.
    pub fn reaction_angle(&self, reaction_id: &str) -> Option<f64> {
        crate::reaction_edit::reaction_angle(self, reaction_id)
    }
    /// Turn a reaction's backbone by `angle` radians (positive is clockwise on
    /// screen) about its centre.
    pub fn rotate_reaction(&mut self, reaction_id: &str, angle: f64) -> bool {
        crate::reaction_edit::rotate_reaction(self, reaction_id, angle)
    }
    /// Turn a reaction's backbone so that it points along `angle` (radians).
    pub fn set_reaction_angle(&mut self, reaction_id: &str, angle: f64) -> bool {
        crate::reaction_edit::set_reaction_angle(self, reaction_id, angle)
    }

    /// Whether a reaction has a metabolite on each side to be kept straight
    /// between.
    pub fn can_linearize_reaction(&self, reaction_id: &str) -> bool {
        crate::reaction_edit::can_linearize_reaction(self, reaction_id)
    }
    /// Keep a reaction straight between two of its metabolites, so that it
    /// follows them when they are moved. `on = false` gives it back its own
    /// position.
    pub fn set_reaction_linear(&mut self, reaction_id: &str, on: bool, from: Option<&str>, to: Option<&str>) -> bool {
        crate::reaction_edit::set_reaction_linear(self, reaction_id, on, from, to)
    }
    /// The two metabolites a reaction's line runs through, or would if made straight.
    pub fn reaction_line_ends(&self, reaction_id: &str) -> (Option<String>, Option<String>) {
        crate::reaction_edit::reaction_line_ends(self, reaction_id)
    }
    /// Choose one of the two metabolites a reaction's line runs through.
    pub fn set_reaction_line_end(&mut self, reaction_id: &str, node_id: &str) -> bool {
        crate::reaction_edit::set_reaction_line_end(self, reaction_id, node_id)
    }
    /// Lay a reaction out along a compass direction (`N`, `NE`, … `NW`),
    /// substrate to product, through its marker, and keep it straight.
    pub fn align_reaction(&mut self, reaction_id: &str, direction: &str, from: Option<&str>, to: Option<&str>) -> bool {
        crate::reaction_edit::align_reaction(self, reaction_id, direction, from, to)
    }
    /// Put a reaction's cofactors and curves back in their base shape.
    pub fn tidy_reaction(&mut self, reaction_id: &str) -> bool {
        crate::reaction_edit::tidy_reaction(self, reaction_id)
    }
    /// The metabolites drawn on each side of a reaction: (substrates, products).
    pub fn reaction_sides(&self, reaction_id: &str) -> (Vec<SideEntry>, Vec<SideEntry>) {
        crate::reaction_edit::reaction_sides(self, reaction_id)
    }
    /// Exchange the places of two metabolites on the same side of a reaction.
    pub fn swap_reaction_metabolites(&mut self, reaction_id: &str, a: &str, b: &str) -> bool {
        crate::reaction_edit::swap_reaction_metabolites(self, reaction_id, a, b)
    }
    /// Exchange the substrate side and the product side of a reaction's drawing.
    pub fn flip_reaction(&mut self, reaction_id: &str) -> bool {
        crate::reaction_edit::flip_reaction(self, reaction_id)
    }
    /// Mirror a reaction's drawing in its own backbone.
    pub fn mirror_reaction(&mut self, reaction_id: &str) -> bool {
        crate::reaction_edit::mirror_reaction(self, reaction_id)
    }
    /// Move one of the Escher-style cubic Bézier control handles of a branch.
    pub fn move_branch_control(&mut self, reaction_id: &str, node_id: &str, handle: Handle, x: f64, y: f64) -> Option<Point> {
        crate::reaction_edit::move_branch_control(self, reaction_id, node_id, handle, x, y)
    }
    /// The b1, b2 and backbone handles of every branch of a reaction.
    pub fn reaction_control_points(&mut self, reaction_id: &str) -> Vec<ControlPoint> {
        crate::reaction_edit::reaction_control_points(self, reaction_id)
    }

    /// What a selection rectangle catches (see
    /// `multi_selection::selectable_in_rect`).
    pub fn selectable_in_rect(&self, rect: SelectionRect, include_secondary: bool) -> Selection {
        crate::multi_selection::selectable_in_rect(self, rect, include_secondary)
    }
    /// Move every object of a selection by `(dx, dy)`.
    pub fn translate_selection(&mut self, selection: &Selection, dx: f64, dy: f64) -> Option<Moved> {
        crate::multi_selection::translate_selection(self, selection, dx, dy)
    }
    /// Move a selection as one rigid body, with the compartments in it (see
    /// `multi_selection::drag_selection`).
    pub fn drag_selection(&mut self, selection: &Selection, dx: f64, dy: f64) -> Option<Moved> {
        crate::multi_selection::drag_selection(self, selection, dx, dy)
    }
    /// Align a selection: `horizontal`, `vertical`, or by its
    /// `left`/`right`/`top`/`bottom` edges.
    pub fn align_selection(&mut self, selection: &Selection, mode: &str) -> Arranged {
        crate::multi_selection::align_selection(self, selection, mode)
    }
    /// Spread a selection evenly along `horizontal` or `vertical`.
    pub fn distribute_selection(&mut self, selection: &Selection, axis: &str) -> Arranged {
        crate::multi_selection::distribute_selection(self, selection, axis)
    }
    /// Escher's Align vertical / Align horizontal for a selection with
    /// reactions in it.
    pub fn align_selection_in_line(&mut self, selection: &Selection, mode: &str) -> Option<LineAlignment> {
        crate::multi_selection::align_selection_in_line(self, selection, mode)
    }
    /// Whether a selection has a reaction Align vertical / horizontal can lay
    /// on a line.
    pub fn selection_has_line_reactions(&self, selection: &Selection) -> bool {
        crate::multi_selection::selection_has_line_reactions(self, selection)
    }
}
