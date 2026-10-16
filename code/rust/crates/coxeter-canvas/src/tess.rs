//! Tessellation: fills and strokes of lyon paths into triangle meshes in map
//! coordinates, once, so that panning and zooming only move them.
//!
//! A stroke vertex keeps the point of the path it belongs to and its offset
//! from it (the normal times half the line width) apart. That is what lets a
//! group of vertices be scaled about an anchor while its strokes keep their
//! width (a label grown by `--label-scale` keeps a halo of 3.2 units) or
//! scaled together with them (a name plate's `transform: scale(…)`).
//!
//! Dashes are cut here (lyon draws none): the path is flattened, walked along
//! the dash array, and each dash is stroked with the line's caps.

use egui::Color32;
use lyon_tessellation::math::{Point as LPoint, point};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Event, Path};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap as LCap, LineJoin as LJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};

use crate::style::{LineCap, LineJoin};

/// Map units: an error of 0.3 px at the largest zoom (6×).
pub const TOLERANCE: f32 = 0.05;

/// One vertex of the map's meshes.
///
/// Drawn at `anchor + k·(pos − anchor) + m·offset`, where `anchor`, `k` and `m`
/// belong to the vertex's group: group 0 is the static map (`k = m = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MapVertex {
    pub pos: [f32; 2],
    pub offset: [f32; 2],
    pub color: Color32,
    pub group: u32,
}

/// Triangles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshBuf {
    pub verts: Vec<MapVertex>,
    pub indices: Vec<u32>,
}

impl MeshBuf {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn clear(&mut self) {
        self.verts.clear();
        self.indices.clear();
    }

    /// Append another mesh, recoloured and regrouped.
    pub fn append(&mut self, other: &MeshBuf) {
        let base = self.verts.len() as u32;
        self.verts.extend_from_slice(&other.verts);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    /// The bounding box of the vertices, offsets included.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for v in &self.verts {
            let x = v.pos[0] + v.offset[0];
            let y = v.pos[1] + v.offset[1];
            b[0] = b[0].min(x);
            b[1] = b[1].min(y);
            b[2] = b[2].max(x);
            b[3] = b[3].max(y);
        }
        (b[0] <= b[2]).then_some(b)
    }
}

/// How a stroke is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeSpec {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    /// The dash array, in the same units as the path.
    pub dash: Option<Vec<f32>>,
}

impl StrokeSpec {
    pub fn solid(width: f32) -> StrokeSpec {
        StrokeSpec { width, cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 4.0, dash: None }
    }
}

fn lyon_cap(c: LineCap) -> LCap {
    match c {
        LineCap::Butt => LCap::Butt,
        LineCap::Round => LCap::Round,
        LineCap::Square => LCap::Square,
    }
}

fn lyon_join(j: LineJoin) -> LJoin {
    match j {
        // SVG falls back to a bevel past the miter limit, as lyon's MiterClip does not.
        LineJoin::Miter => LJoin::Miter,
        LineJoin::Round => LJoin::Round,
        LineJoin::Bevel => LJoin::Bevel,
    }
}

/// Fill a path (non-zero winding, SVG's default fill rule).
pub fn fill(path: &Path, color: Color32, group: u32, tolerance: f32, out: &mut MeshBuf) {
    let mut buffers: VertexBuffers<MapVertex, u32> = VertexBuffers::new();
    let mut t = FillTessellator::new();
    let opts = FillOptions::tolerance(tolerance).with_fill_rule(FillRule::NonZero);
    let result = t.tessellate_path(
        path,
        &opts,
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| MapVertex {
            pos: v.position().to_array(),
            offset: [0.0, 0.0],
            color,
            group,
        }),
    );
    if result.is_ok() {
        push(out, buffers);
    }
}

fn push(out: &mut MeshBuf, buffers: VertexBuffers<MapVertex, u32>) {
    let base = out.verts.len() as u32;
    out.verts.extend(buffers.vertices);
    out.indices.extend(buffers.indices.into_iter().map(|i| i + base));
}

/// Stroke a path, with its dashes.
pub fn stroke(path: &Path, spec: &StrokeSpec, color: Color32, group: u32, tolerance: f32, out: &mut MeshBuf) {
    if !(spec.width > 0.0) || !spec.width.is_finite() {
        return;
    }
    match &spec.dash {
        Some(dash) if dash.iter().sum::<f32>() > 0.0 => {
            let dashed = dash_path(path, dash, tolerance);
            stroke_solid(&dashed, spec, color, group, tolerance, out);
        }
        _ => stroke_solid(path, spec, color, group, tolerance, out),
    }
}

fn stroke_solid(path: &Path, spec: &StrokeSpec, color: Color32, group: u32, tolerance: f32, out: &mut MeshBuf) {
    let mut buffers: VertexBuffers<MapVertex, u32> = VertexBuffers::new();
    let mut t = StrokeTessellator::new();
    let opts = StrokeOptions::tolerance(tolerance)
        .with_line_width(spec.width)
        .with_line_cap(lyon_cap(spec.cap))
        .with_line_join(lyon_join(spec.join))
        .with_miter_limit(spec.miter_limit.max(1.0));
    let result = t.tessellate_path(
        path,
        &opts,
        &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| {
            let on = v.position_on_path();
            let at = v.position();
            MapVertex { pos: on.to_array(), offset: [at.x - on.x, at.y - on.y], color, group }
        }),
    );
    if result.is_ok() {
        push(out, buffers);
    }
}

/// The dashes of a path as open sub-paths, the pattern starting at the start
/// of each sub-path and running on across corners and the closing segment.
pub fn dash_path(path: &Path, dash: &[f32], tolerance: f32) -> Path {
    let mut b = Path::builder();
    let total: f32 = dash.iter().sum();
    if !(total > 0.0) {
        return path.clone();
    }
    // Flatten into polylines (closed ones repeat their first point).
    let mut polylines: Vec<Vec<LPoint>> = Vec::new();
    let mut current: Vec<LPoint> = Vec::new();
    for ev in path.iter().flattened(tolerance) {
        match ev {
            Event::Begin { at } => {
                current.clear();
                current.push(at);
            }
            Event::Line { to, .. } => current.push(to),
            Event::End { first, close, .. } => {
                if close {
                    current.push(first);
                }
                polylines.push(std::mem::take(&mut current));
            }
            _ => {}
        }
    }
    for line in polylines {
        let mut idx = 0usize; // which dash entry
        let mut left = dash[0]; // how much of it remains
        let mut on = true;
        let mut drawing = false;
        for w in line.windows(2) {
            let (a, c) = (w[0], w[1]);
            let seg = (c - a).length();
            if seg == 0.0 {
                continue;
            }
            let mut t0 = 0.0f32;
            while t0 < seg {
                let step = left.min(seg - t0);
                let p0 = a + (c - a) * (t0 / seg);
                let p1 = a + (c - a) * ((t0 + step) / seg);
                if on {
                    if !drawing {
                        b.begin(p0);
                        drawing = true;
                    }
                    b.line_to(p1);
                }
                t0 += step;
                left -= step;
                if left <= 1e-6 {
                    if on && drawing {
                        b.end(false);
                        drawing = false;
                    }
                    idx = (idx + 1) % dash.len();
                    left = dash[idx];
                    on = idx.is_multiple_of(2);
                    // A zero-length dash still draws its caps.
                    if on && left == 0.0 {
                        let q = a + (c - a) * (t0 / seg);
                        b.begin(q);
                        b.line_to(q);
                        b.end(false);
                        idx = (idx + 1) % dash.len();
                        left = dash[idx];
                        on = false;
                    }
                }
            }
        }
        if drawing {
            b.end(false);
        }
    }
    b.build()
}

/// A polygon given by its points, closed.
pub fn polygon_path(points: &[(f32, f32)]) -> Option<Path> {
    if points.len() < 3 {
        return None;
    }
    let mut b = Path::builder();
    b.begin(point(points[0].0, points[0].1));
    for &(x, y) in &points[1..] {
        b.line_to(point(x, y));
    }
    b.end(true);
    Some(b.build())
}

/// Transform every point of a path.
pub fn transform_path(path: &Path, m: &crate::geom::Mat) -> Path {
    let t = lyon_tessellation::geom::Transform::new(
        m[0] as f32,
        m[1] as f32,
        m[2] as f32,
        m[3] as f32,
        m[4] as f32,
        m[5] as f32,
    );
    path.clone().transformed(&t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_cut_a_line() {
        let mut b = Path::builder();
        b.begin(point(0.0, 0.0));
        b.line_to(point(20.0, 0.0));
        b.end(false);
        let dashed = dash_path(&b.build(), &[4.0, 2.0], 0.05);
        let begins = dashed.iter().filter(|e| matches!(e, Event::Begin { .. })).count();
        assert_eq!(begins, 4, "0-4, 6-10, 12-16, 18-20");
    }

    #[test]
    fn strokes_keep_offsets_apart() {
        let mut b = Path::builder();
        b.begin(point(0.0, 0.0));
        b.line_to(point(10.0, 0.0));
        b.end(false);
        let mut out = MeshBuf::default();
        stroke(&b.build(), &StrokeSpec::solid(2.0), Color32::RED, 0, 0.05, &mut out);
        assert!(!out.is_empty());
        for v in &out.verts {
            assert_eq!(v.pos[1], 0.0);
            assert!((v.offset[1].abs() - 1.0).abs() < 1e-5);
        }
    }
}
