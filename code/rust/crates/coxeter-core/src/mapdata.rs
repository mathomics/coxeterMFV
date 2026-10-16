//! The map as it is saved: the JSON of `EditableMap#toJSON()` and of the
//! versioned map document around it (`src/map/map-document.js`).
//!
//! Every crate that reads or draws a map shares these types. They list every
//! field the JavaScript application is known to write (taken from the bundled
//! scenarios, the gut9 map snapshots and maps saved by users). Anything else a
//! file carries is kept in `extra` and written back unchanged, so a map opened
//! and saved by the Rust build loses nothing.
//!
//! # Points shared with a reaction's axis
//!
//! In JavaScript some segment endpoints *are* the reaction's axis points (the
//! same object), so moving the axis moves them. Rust keeps copies instead, and
//! [`axis_links`] says which endpoint follows which axis point, by the rule of
//! `relinkAxisEndpoints` in `editable-map.js`. Code that moves an axis point
//! calls [`relink_axis_endpoints`] (or updates the linked endpoints itself)
//! so the copies never drift apart.

use indexmap::IndexMap;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A number that JSON may have written as `null` (JavaScript writes NaN that
/// way). Read back as NaN rather than refusing the whole map.
fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(v) => crate::js::to_number(&v),
        None => f64::NAN,
    })
}

fn lenient_opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        None | Some(Value::Null) => None,
        Some(v) => Some(crate::js::to_number(&v)),
    })
}

/// A field whose JSON `null` must survive a round trip: absent reads as `None`,
/// `null` as `Some(Value::Null)`. Plain `Option<Value>` folds both into `None`,
/// and the JS writes `terminalMetaboliteId: null` on every hand-placed reaction.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    #[serde(deserialize_with = "lenient_f64")]
    pub x: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    #[serde(deserialize_with = "lenient_f64")]
    pub x: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub y: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub w: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Canvas {
    #[serde(deserialize_with = "lenient_f64")]
    pub width: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub height: f64,
}

/// The shared medium drawn around every cell.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub rect: Rect,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One drawn compartment of a cell.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapCompartment {
    pub id: String,
    pub rect: Rect,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub depth: f64,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub label: String,
    /// `bilayer`, `wall` or `open`.
    #[serde(default)]
    pub membrane: String,
    #[serde(default)]
    pub visual_id: String,
    /// Map-only colours chosen by the user (`#rrggbb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    /// `{ corner, oval }`, each 0..1, once its shape was edited (see
    /// `geometry::compartment_shape`). Kept as written, so a hand-edited file
    /// round-trips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One drawn cell (a species) with its compartments, outermost first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapCell {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organism: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_f64")]
    pub abundance: Option<f64>,
    pub rect: Rect,
    #[serde(default)]
    pub compartments: Vec<MapCompartment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_compartment: Option<String>,
    /// Map-only colour of the cell boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manually_drawn: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The medium nodes and community exchanges drawn between the cells.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Medium {
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub exchanges: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $variant:ident = $text:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vm])* $variant,)*
            /// A value this build does not know, kept as written.
            Other(String),
        }
        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)*
                    $name::Other(s) => s,
                }
            }
            pub fn parse(s: &str) -> $name {
                match s {
                    $($text => $name::$variant,)*
                    other => $name::Other(other.to_string()),
                }
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Ok($name::parse(&s))
            }
        }
    };
}

string_enum!(
    /// What a map node stands for.
    NodeKind {
        /// A metabolite inside a cell (shared, or local to one reaction).
        Metabolite = "metabolite",
        /// A metabolite of the shared medium between cells.
        Medium = "medium",
        /// A per-reaction copy of a currency metabolite (ATP, H2O, H+…).
        Currency = "currency",
    }
);

string_enum!(
    /// What a segment draws.
    SegmentKind {
        /// The reaction's own short axis between its two attachment points.
        Axis = "axis",
        /// An exchange's stub out to the edge of the system.
        Boundary = "boundary",
        /// A curve to a primary metabolite.
        Primary = "primary",
        /// A curve to a currency (secondary) metabolite.
        Currency = "currency",
        /// One lane of a multi-lane transporter across a membrane.
        TransportLane = "transport-lane",
    }
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<String>,
    /// The label the map prints by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compartment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_compartment: Option<String>,
    /// The cell (species) it belongs to, null for the shared medium.
    #[serde(default)]
    pub cell_id: Option<String>,
    /// For currency nodes: the one reaction this copy belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_id: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub x: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub y: f64,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub r: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
    /// `primary` or `secondary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escher_secondary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_f64")]
    pub degree: Option<f64>,
    /// The reaction that placed this reaction-local metabolite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_local_for: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_local: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_instance: Option<bool>,
    /// Where the label sits around the node, in radians.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_f64")]
    pub label_angle: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_duplicate: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A colour kept in an object's own fields (`node.fill`, `rxn.color`):
/// a string that is not empty.
fn color_field<'a>(extra: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    extra.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl Node {
    /// `node.fill`: the circle's own fill, over the map's (`setNodeColors`).
    pub fn own_fill(&self) -> Option<&str> {
        color_field(&self.extra, "fill")
    }
    /// `node.stroke`: the circle's own outline, over the map's.
    pub fn own_stroke(&self) -> Option<&str> {
        color_field(&self.extra, "stroke")
    }
    /// A node that stands for a metabolite (not map furniture).
    pub fn is_metabolite_kind(&self) -> bool {
        matches!(self.kind, NodeKind::Metabolite | NodeKind::Medium | NodeKind::Currency)
    }
    pub fn pos(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// A reaction's short axis: substrates meet at `a1`, products leave `a2`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Axis {
    pub a1: Point,
    pub a2: Point,
    pub dir: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Bounds {
    #[serde(deserialize_with = "lenient_f64")]
    pub lower: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub upper: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportLane {
    pub base: String,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub offset: f64,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub direction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_point: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside_point: Option<Point>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Which metabolite nodes a reaction's line runs between.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LineEnds {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A reaction drawn as a straight line between two nodes, at fraction `t`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Linear {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt_f64", skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapReaction {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(default)]
    pub cell_id: Option<String>,
    /// The merged model's role: `metabolic`, `transport`, `transfer`,
    /// `exchange`, `biomass`, `internal`…
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub compartments: Vec<String>,
    /// The compartment whose membrane a transporter sits on.
    #[serde(default)]
    pub membrane: Option<String>,
    #[serde(default)]
    pub on_membrane: bool,
    #[serde(default)]
    pub normal: Option<Point>,
    /// The reaction marker (midpoint of the axis).
    #[serde(deserialize_with = "lenient_f64")]
    pub x: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    /// An exchange's outer end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membrane_visual_id: Option<String>,
    #[serde(default)]
    pub boundary_id: Option<String>,
    #[serde(default)]
    pub placement_direction: Option<Value>,
    #[serde(default)]
    pub axis_direction_hint: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub terminal_metabolite_id: Option<Value>,
    /// `uniport`, `symport`, `antiport`…
    #[serde(default)]
    pub transport_mode: Option<String>,
    #[serde(default)]
    pub transport_lanes: Option<Vec<TransportLane>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_ends: Option<LineEnds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear: Option<Linear>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbml_reversible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genes: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl MapReaction {
    /// `rxn.color`: the colour set on the reaction, used over its pathway's
    /// when lines are not coloured by flux (`setReactionColor`).
    pub fn own_color(&self) -> Option<&str> {
        color_field(&self.extra, "color")
    }
}

/// `layout.nodeStyle`: the colours metabolite circles take unless one sets
/// its own. Each `None` is the theme's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub reaction_id: String,
    pub kind: SegmentKind,
    pub from: Point,
    pub to: Point,
    #[serde(default)]
    pub arrow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_f64")]
    pub coefficient: Option<f64>,
    /// `substrate` or `product`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Escher-style branch with its own attachment point on the backbone.
    #[serde(default, skip_serializing_if = "is_false")]
    pub backbone_anchor: bool,
    /// Cubic Bézier control points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b1: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b2: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_base: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub transport_lane: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_f64")]
    pub transport_direction: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Segment {
    pub fn is_substrate(&self) -> bool {
        self.side.as_deref() == Some("substrate")
    }
    pub fn is_product(&self) -> bool {
        self.side.as_deref() == Some("product")
    }
}

/// A free text placed on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapText {
    pub id: String,
    pub text: String,
    #[serde(deserialize_with = "lenient_f64")]
    pub x: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_f64")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `EditableMap#toJSON()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapData {
    #[serde(default = "one")]
    pub version: Value,
    #[serde(default)]
    pub canvas: Option<Canvas>,
    /// Legacy maps give the size at the top level instead of in `canvas`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(default)]
    pub environment: Option<Environment>,
    #[serde(default)]
    pub cells: Vec<MapCell>,
    #[serde(default)]
    pub medium: Option<Medium>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub reactions: Vec<MapReaction>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub texts: Vec<MapText>,
    /// Which label source each kind of object shows (`metabolite`, `reaction`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_settings: Option<IndexMap<String, String>>,
    /// The colours metabolite circles take unless one sets its own, saved
    /// only once set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_style: Option<NodeStyle>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn one() -> Value {
    Value::from(1)
}

impl MapData {
    /// The canvas size, from `canvas` or, in a legacy map, from the top-level
    /// `width` and `height`.
    pub fn canvas_size(&self) -> Canvas {
        self.canvas.unwrap_or(Canvas { width: self.width.unwrap_or(0.0), height: self.height.unwrap_or(0.0) })
    }
}

pub const MAP_DOCUMENT_FORMAT: &str = "EscherMCMCMap";
pub const MAP_DOCUMENT_VERSION: i64 = 3;

/// The versioned envelope a map is saved in (format `EscherMCMCMap`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapDocument {
    pub format: String,
    pub version: i64,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub pathway_manager: Option<Value>,
    #[serde(default)]
    pub flux_color_stops: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study: Option<Value>,
    pub map: MapData,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Which end of a reaction's axis a segment endpoint is the same point as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisEnd {
    A1,
    A2,
}

/// `(from, to)`: which axis point each end of `seg` follows, if any. The
/// rule of `relinkAxisEndpoints` in `editable-map.js`.
pub fn axis_links(seg: &Segment) -> (Option<AxisEnd>, Option<AxisEnd>) {
    match seg.kind {
        SegmentKind::Axis => (Some(AxisEnd::A1), Some(AxisEnd::A2)),
        SegmentKind::Boundary => (Some(AxisEnd::A2), None),
        // Transport lanes and Escher-style backbone branches keep their own
        // reaction-side attachment point instead of collapsing to a1/a2.
        _ if seg.transport_lane || seg.backbone_anchor => (None, None),
        _ if seg.is_substrate() => (None, Some(AxisEnd::A1)),
        _ if seg.is_product() => (Some(AxisEnd::A2), None),
        _ => (None, None),
    }
}

fn axis_point(axis: &Axis, end: AxisEnd) -> Point {
    match end {
        AxisEnd::A1 => axis.a1,
        AxisEnd::A2 => axis.a2,
    }
}

/// Copy each reaction's axis points into the segment endpoints that follow
/// them, which is what sharing one object did in JavaScript. Pass the reactions by id.
pub fn relink_axis_endpoints<'a>(axis_of: impl Fn(&str) -> Option<&'a Axis>, segments: &mut [Segment]) {
    for seg in segments {
        let Some(axis) = axis_of(&seg.reaction_id) else { continue };
        let (from, to) = axis_links(seg);
        if let Some(end) = from {
            seg.from = axis_point(axis, end);
        }
        if let Some(end) = to {
            seg.to = axis_point(axis, end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_saved_map_round_trips() {
        // Bundled snapshots: read, write, read again, and nothing is lost.
        let dir = crate::testkit::repo_root().join("docs/gut9-map/maps");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let raw: Value = serde_json::from_str(&text).unwrap();
            let map_raw = raw.get("map").cloned().unwrap_or(raw.clone());
            if map_raw.get("nodes").is_none() {
                continue; // not a map (cross-feeding data)
            }
            let map: MapData = serde_json::from_value(map_raw.clone())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let back = serde_json::to_value(&map).unwrap();
            // Null and absent are the same to every reader of a map, so compare
            // with nulls dropped on both sides.
            if let Some(d) = crate::testkit::json_diff(&drop_nulls(&back), &drop_nulls(&map_raw), 0.0) {
                panic!("{}: {d}", path.display());
            }
            checked += 1;
        }
        assert!(checked >= 8);
    }

    fn drop_nulls(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), drop_nulls(v))).collect()),
            Value::Array(a) => Value::Array(a.iter().map(drop_nulls).collect()),
            other => other.clone(),
        }
    }

    #[test]
    fn substrate_ends_follow_a1() {
        let seg: Segment = serde_json::from_value(serde_json::json!({
            "reactionId": "r", "kind": "primary", "from": {"x": 0, "y": 0}, "to": {"x": 1, "y": 1}, "side": "substrate"
        }))
        .unwrap();
        assert_eq!(axis_links(&seg), (None, Some(AxisEnd::A1)));
        let mut segs = vec![seg];
        let axis = Axis { a1: Point::new(5.0, 6.0), a2: Point::new(7.0, 8.0), dir: Point::new(0.0, 1.0) };
        relink_axis_endpoints(|_| Some(&axis), &mut segs);
        assert_eq!(segs[0].to, Point::new(5.0, 6.0));
    }
}

/// The live map as `EditableMap` holds it and the renderer reads it: nodes,
/// reactions and texts by id in drawing order, segments in drawing order.
/// The editing operations live in the `coxeter-map` crate. This is the data
/// every crate can share without depending on it.
#[derive(Debug, Clone, PartialEq)]
pub struct MapGraph {
    pub version: Value,
    pub canvas: Canvas,
    pub environment: Environment,
    pub cells: Vec<MapCell>,
    pub medium: Medium,
    pub nodes: IndexMap<String, Node>,
    pub reactions: IndexMap<String, MapReaction>,
    pub segments: Vec<Segment>,
    pub texts: IndexMap<String, MapText>,
    pub label_settings: Option<IndexMap<String, String>>,
    /// `nodeStyle`: see [`MapGraph::node_style`].
    pub node_style: Option<NodeStyle>,
    /// Top-level fields this build does not know, written back unchanged.
    pub extra: Map<String, Value>,
}

impl MapGraph {
    pub fn width(&self) -> f64 {
        self.canvas.width
    }
    pub fn height(&self) -> f64 {
        self.canvas.height
    }

    /// The default environment of a canvas (`EditableMap` constructor).
    pub fn default_environment(canvas: Canvas) -> Environment {
        let mut names = crate::compartments::Names::new();
        names.insert("s".into(), "Shared medium".into());
        Environment {
            rect: Rect::new(0.0, 0.0, canvas.width, canvas.height),
            id: crate::community::SHARED_COMPARTMENT.into(),
            label: crate::compartments::compartment_name(crate::community::SHARED_COMPARTMENT, &names, "en"),
            hidden: None,
            extra: Map::new(),
        }
    }

    /// Adopt saved map data (`EditableMap.fromJSON`, before the editable
    /// map's own normalisations): ids become keys, and segment endpoints that
    /// share a point with their reaction's axis are re-linked to it.
    pub fn from_data(data: MapData) -> MapGraph {
        let canvas = data.canvas_size();
        let reactions: IndexMap<String, MapReaction> = data.reactions.into_iter().map(|r| (r.id.clone(), r)).collect();
        let mut segments = data.segments;
        relink_axis_endpoints(|id| reactions.get(id).and_then(|r| r.axis.as_ref()), &mut segments);
        MapGraph {
            version: data.version,
            environment: data.environment.unwrap_or_else(|| MapGraph::default_environment(canvas)),
            canvas,
            cells: data.cells,
            medium: data.medium.unwrap_or_default(),
            nodes: data.nodes.into_iter().map(|n| (n.id.clone(), n)).collect(),
            reactions,
            segments,
            texts: data.texts.into_iter().map(|t| (t.id.clone(), t)).collect(),
            label_settings: data.label_settings,
            node_style: data.node_style,
            extra: data.extra,
        }
    }

    /// `EditableMap#toJSON()`.
    pub fn to_data(&self) -> MapData {
        MapData {
            version: Value::from(1),
            canvas: Some(self.canvas),
            width: None,
            height: None,
            environment: Some(self.environment.clone()),
            cells: self.cells.clone(),
            medium: Some(self.medium.clone()),
            nodes: self.nodes.values().cloned().collect(),
            reactions: self.reactions.values().cloned().collect(),
            segments: self.segments.clone(),
            texts: self.texts.values().cloned().collect(),
            label_settings: self.label_settings.clone(),
            node_style: self.node_style.clone(),
            extra: self.extra.clone(),
        }
    }

    /// `nodeStyle`: the colours metabolite circles take unless one sets its
    /// own (each `None`: the theme's).
    pub fn node_style(&self) -> NodeStyle {
        self.node_style.clone().unwrap_or_default()
    }

    /// Set `nodeStyle` (`null` when it holds nothing).
    pub fn set_node_style(&mut self, style: &NodeStyle) {
        let empty = style.fill.is_none() && style.stroke.is_none() && style.extra.is_empty();
        self.node_style = (!empty).then(|| style.clone());
    }

    /// Copy every reaction's axis points into the segment ends that follow
    /// them. Call after moving any axis point.
    pub fn relink_axes(&mut self) {
        let reactions = &self.reactions;
        relink_axis_endpoints(|id| reactions.get(id).and_then(|r| r.axis.as_ref()), &mut self.segments);
    }

    /// Re-link only the segments of one reaction.
    pub fn relink_axis_of(&mut self, reaction_id: &str) {
        let Some(axis) = self.reactions.get(reaction_id).and_then(|r| r.axis) else { return };
        for seg in self.segments.iter_mut().filter(|s| s.reaction_id == reaction_id) {
            let (from, to) = axis_links(seg);
            if let Some(end) = from {
                seg.from = axis_point(&axis, end);
            }
            if let Some(end) = to {
                seg.to = axis_point(&axis, end);
            }
        }
    }
}
