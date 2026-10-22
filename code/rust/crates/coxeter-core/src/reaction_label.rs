//! Where a reaction's name is written (`src/map/reaction-label.js`).
//!
//! By default the map places it: beside the marker, or over and under the bar
//! of a straight reaction lying on its side, where its own arms would be in the
//! way. A name dragged elsewhere keeps where it was put as `rxn.labelOffset`,
//! the point it is written from relative to the marker, so it follows the
//! reaction when the reaction is moved. Like a metabolite's label, the text
//! itself stays horizontal.
//!
//! The offset has no field of its own on [`MapReaction`]: it lives among the
//! extra fields under [`LABEL_OFFSET`], so it is saved where the JavaScript
//! saves it, after the fields every reaction has.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::js;
use crate::mapdata::{MapGraph, MapReaction, Node, Point};
use crate::metabolite_label::Anchor;

/// The key of a moved name's offset on a reaction.
pub const LABEL_OFFSET: &str = "labelOffset";

/// `normalizeReactionLabelOffset(value)`: a usable offset, or None: an object
/// with a finite `x` and `y` (map units). Anything else (a number, a string)
/// is not one, so a damaged file loads with the name back in its default place
/// rather than nowhere.
pub fn normalize_reaction_label_offset(value: &Value) -> Option<Point> {
    let x = value.get("x")?.as_f64()?;
    let y = value.get("y")?.as_f64()?;
    (x.is_finite() && y.is_finite()).then_some(Point::new(x, y))
}

/// `rxn.labelOffset`, when it is a usable one.
pub fn reaction_label_offset(rxn: &MapReaction) -> Option<Point> {
    rxn.extra.get(LABEL_OFFSET).and_then(normalize_reaction_label_offset)
}

/// Write `rxn.labelOffset`, or delete it with None.
pub fn set_reaction_label_offset(rxn: &mut MapReaction, offset: Option<Point>) {
    match offset {
        Some(p) => {
            rxn.extra.insert(LABEL_OFFSET.into(), json!({ "x": p.x, "y": p.y }));
        }
        None => {
            rxn.extra.shift_remove(LABEL_OFFSET);
        }
    }
}

/// Where the name of a reaction is written: the point, the text anchor, and
/// the class that sets its baseline (`emc-label-rxn-adjacent` centres it on
/// the point, `-above` writes it above, `-below` below).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReactionLabelPlacement {
    pub x: f64,
    pub y: f64,
    pub anchor: Anchor,
    pub placement: &'static str,
}

/// `reactionLabelPlacement(layout, rxn, nodesOf)`.
///
/// `nodes_of` gives the metabolite nodes the reaction reaches. Only a straight
/// reaction lying on its side needs them, so a caller drawing every name
/// passes one that reads [`reaction_nodes`] built once; [`reaction_label_placement_in`]
/// finds them on demand.
///
/// A moved name is written from the marker plus its offset, aligned away from
/// the marker the way a metabolite's label is aligned away from its circle.
/// The two default places, read as offsets, give the anchor and the baseline
/// they have, so a name picked up does not jump.
pub fn reaction_label_placement<'a, I>(rxn: &MapReaction, nodes_of: impl FnOnce() -> I) -> ReactionLabelPlacement
where
    I: IntoIterator<Item = &'a Node>,
{
    if let Some(offset) = reaction_label_offset(rxn) {
        let length = js::hypot(offset.x, offset.y);
        let cos = if length > 1e-9 { offset.x / length } else { 0.0 };
        let sin = if length > 1e-9 { offset.y / length } else { 0.0 };
        return ReactionLabelPlacement {
            x: rxn.x + offset.x,
            y: rxn.y + offset.y,
            anchor: if cos > 0.3 {
                Anchor::Start
            } else if cos < -0.3 {
                Anchor::End
            } else {
                Anchor::Middle
            },
            placement: if sin < -0.3 {
                "emc-label-rxn-above"
            } else if sin > 0.3 {
                "emc-label-rxn-below"
            } else {
                "emc-label-rxn-adjacent"
            },
        };
    }

    // Place the ID and its value beside the marker, leaving a gap even
    // for rotated transporters with several lanes.
    let angle = rxn.axis.map_or(0.0, |a| js::atan2(a.dir.y, a.dir.x));
    let half_width = if rxn.on_membrane { 13.0 } else { 3.0 };
    let lanes = rxn.transport_lanes.as_ref().map_or(0, Vec::len);
    let half_height = if rxn.on_membrane {
        if lanes >= 2 { (14.0 + lanes as f64 * 10.0) / 2.0 } else { 11.0 }
    } else {
        9.0
    };
    let (cos, sin) = (js::cos(angle), js::sin(angle));
    let marker_right = half_width * cos.abs() + half_height * sin.abs();
    // A reaction kept straight and lying on its side has its own arm where
    // the name would go, so there the name stands over the bar, on the side
    // with fewer of its metabolites.
    if rxn.linear.is_some() && rxn.axis.is_some() && !rxn.on_membrane && sin.abs() < 0.5 {
        let marker_top = half_width * sin.abs() + half_height * cos.abs();
        let below = emptier_side(nodes_of(), rxn) > 0.0;
        return ReactionLabelPlacement {
            x: rxn.x,
            y: rxn.y + (marker_top + 5.0) * if below { 1.0 } else { -1.0 },
            anchor: Anchor::Middle,
            placement: if below { "emc-label-rxn-below" } else { "emc-label-rxn-above" },
        };
    }
    ReactionLabelPlacement { x: rxn.x + marker_right + 8.0, y: rxn.y, anchor: Anchor::Start, placement: "emc-label-rxn-adjacent" }
}

/// [`reaction_label_placement`] of a reaction on `map`, reading the
/// metabolites it reaches from the map's segments when it needs them.
pub fn reaction_label_placement_in(map: &MapGraph, rxn: &MapReaction) -> ReactionLabelPlacement {
    reaction_label_placement(rxn, || {
        map.segments
            .iter()
            .filter(|seg| seg.reaction_id == rxn.id)
            .filter_map(|seg| seg.node_id.as_deref().filter(|id| !id.is_empty()).and_then(|id| map.nodes.get(id)))
    })
}

/// `reactionNodes(layout)`: the metabolite nodes each reaction reaches.
pub fn reaction_nodes(map: &MapGraph) -> HashMap<&str, Vec<&Node>> {
    let mut by_reaction: HashMap<&str, Vec<&Node>> = HashMap::new();
    for seg in &map.segments {
        let Some(node) = seg.node_id.as_deref().filter(|id| !id.is_empty()).and_then(|id| map.nodes.get(id)) else {
            continue;
        };
        if seg.reaction_id.is_empty() {
            continue;
        }
        by_reaction.entry(seg.reaction_id.as_str()).or_default().push(node);
    }
    by_reaction
}

/// Above (-1) or below (+1) a reaction lying on its side: where fewer of its metabolites stand.
fn emptier_side<'a>(nodes: impl IntoIterator<Item = &'a Node>, rxn: &MapReaction) -> f64 {
    let mut above = 0;
    let mut below = 0;
    for node in nodes {
        if node.y < rxn.y - 4.0 {
            above += 1;
        } else if node.y > rxn.y + 4.0 {
            below += 1;
        }
    }
    if below < above { 1.0 } else { -1.0 }
}
