//! Resolve graphical circle positions against outlines and holes without changing biology (`src/layout/region.js`).

use coxeter_core::geometry::{Corners, clamp_to_shape, inset_all, shape_boundary_point, shape_signed_distance};
use coxeter_core::mapdata::{Point, Rect};

/// Coordinate tolerance for checking a graphical circle against its cage.
const EPS: f64 = 1e-6;

/// A compartment's outline and nested regions a graphical circle must avoid.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub bounds: Rect,
    pub bounds_corners: Corners,
    pub holes: Vec<Rect>,
    pub hole_corners: Vec<Corners>,
    /// A cell margin increases clearance without changing the oval's shape.
    pub hole_inset: f64,
    pub pad: f64,
    pub clearance: f64,
}

pub fn point_fits_region(point: Point, region: &Constraint) -> bool {
    let inside = clamp_to_shape(point, &region.bounds, region.pad, region.bounds_corners);
    if (inside.x - point.x).abs() > EPS || (inside.y - point.y).abs() > EPS {
        return false;
    }
    region.holes.iter().enumerate().all(|(index, hole)| {
        let rect = if region.hole_inset != 0.0 { inset_all(hole, region.hole_inset) } else { *hole };
        shape_signed_distance(&rect, point, region.hole_corners[index]) >= region.clearance + region.hole_inset - EPS
    })
}

/// Pull a point into its outline and out of nested compartments; return None when no attempt fits.
pub fn resolve_region(point: Point, region: &Constraint) -> Option<Point> {
    let mut out = clamp_to_shape(point, &region.bounds, region.pad, region.bounds_corners);
    for _ in 0..6 {
        let mut worst = None;
        let mut worst_corners = Corners::circular(0.0);
        let mut depth = 0.0;
        for (index, hole) in region.holes.iter().enumerate() {
            let rect = if region.hole_inset != 0.0 { inset_all(hole, region.hole_inset) } else { *hole };
            let corners = region.hole_corners[index];
            let distance = region.clearance + region.hole_inset - shape_signed_distance(&rect, out, corners);
            if distance > depth {
                depth = distance;
                worst = Some(rect);
                worst_corners = corners;
            }
        }
        let Some(worst) = worst else {
            return Some(out);
        };
        let edge = shape_boundary_point(&worst, out, worst_corners);
        let clearance = region.clearance + region.hole_inset;
        out = clamp_to_shape(
            Point::new(edge.x + edge.nx * clearance, edge.y + edge.ny * clearance),
            &region.bounds, region.pad, region.bounds_corners,
        );
    }
    if point_fits_region(out, region) { Some(out) } else { None }
}
