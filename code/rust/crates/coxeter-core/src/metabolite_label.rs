//! Where a metabolite's label sits (`src/map/metabolite-label.js`). Labels
//! orbit their metabolite. The text itself stays horizontal.

use std::f64::consts::PI;

use crate::geometry::SIZES;
use crate::js;
use crate::mapdata::{Node, NodeKind};

pub const DEFAULT_METABOLITE_LABEL_ANGLE: f64 = -PI / 2.0;

/// A finite instance angle, kept in [-π, π).
pub fn metabolite_label_angle(label_angle: Option<f64>) -> f64 {
    let Some(angle) = label_angle.filter(|a| a.is_finite()) else {
        return DEFAULT_METABOLITE_LABEL_ANGLE;
    };
    if (-PI..PI).contains(&angle) {
        return angle;
    }
    let turn = 2.0 * PI;
    let wrapped = angle % turn;
    if wrapped >= PI {
        wrapped - turn
    } else if wrapped < -PI {
        wrapped + turn
    } else {
        wrapped
    }
}

/// A secondary metabolite instance: a small helper node beside a reaction
/// (ATP, NADH, H⁺…), or any instance set to Secondary by hand. Drawn
/// smaller, and what the Cofactors toggles hide.
pub fn is_secondary_node(node: &Node) -> bool {
    node.is_metabolite_kind() && (node.kind == NodeKind::Currency || node.primary == Some(false))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    /// The SVG `text-anchor` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    Auto,
    Middle,
    Hanging,
}

impl Baseline {
    /// The SVG `dominant-baseline` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Baseline::Auto => "auto",
            Baseline::Middle => "middle",
            Baseline::Hanging => "hanging",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelPosition {
    pub x: f64,
    pub y: f64,
    pub anchor: Anchor,
    pub baseline: Baseline,
}

/// Position and align an unrotated label just outside the visible circle.
pub fn metabolite_label_position(node: &Node) -> LabelPosition {
    let secondary = is_secondary_node(node);
    let radius = if secondary { SIZES.currency_radius } else { SIZES.primary_radius };
    let distance = radius + if secondary { 3.5 } else { 5.0 };
    let angle = metabolite_label_angle(node.label_angle);
    // Near-zero cosines and sines are snapped to 0, so a cardinal direction lands
    // exactly on the default coordinates.
    let raw_cos = js::cos(angle);
    let raw_sin = js::sin(angle);
    let cos = if raw_cos.abs() < 1e-12 { 0.0 } else { raw_cos };
    let sin = if raw_sin.abs() < 1e-12 { 0.0 } else { raw_sin };
    LabelPosition {
        x: node.x + distance * cos,
        y: node.y + distance * sin,
        anchor: if cos > 0.3 {
            Anchor::Start
        } else if cos < -0.3 {
            Anchor::End
        } else {
            Anchor::Middle
        },
        baseline: if sin < -0.3 {
            Baseline::Auto
        } else if sin > 0.3 {
            Baseline::Hanging
        } else {
            Baseline::Middle
        },
    }
}
