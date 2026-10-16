//! Drawing the scene through the camera with egui's own painter: every
//! visible element's mesh, placed by its group and the camera, as one egui
//! mesh in screen coordinates (`applyCamera()`'s
//! `translate(x, y) scale(scale)`).
//!
//! Nothing is tessellated here. The screen mesh is rebuilt only when the
//! camera, the view or the scene changes; when nothing moves the previous one
//! is drawn again. This path works on every egui backend (wgpu and glow
//! alike); the screen mesh is uploaded once per change.

use std::sync::Arc;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Pos2, Rect, Shape};

use crate::camera::Camera;
use crate::scene::Scene;

/// What a screen mesh was made for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    revision: u64,
    camera: [f64; 3],
    view: [f32; 4],
    origin: [f32; 2],
}

/// The last screen mesh, kept while nothing changes.
#[derive(Debug, Default, Clone)]
pub struct MeshCache {
    key: Option<Key>,
    mesh: Option<Arc<Mesh>>,
    /// Elements drawn in the last mesh.
    pub drawn: usize,
}

/// The part of the map a screen rectangle shows, in map coordinates.
pub fn view_in_map(cam: &Camera, origin: Pos2, view: Rect) -> [f64; 4] {
    let s = if cam.scale > 0.0 { cam.scale } else { 1.0 };
    [
        (view.min.x as f64 - origin.x as f64 - cam.x) / s,
        (view.min.y as f64 - origin.y as f64 - cam.y) / s,
        (view.max.x as f64 - origin.x as f64 - cam.x) / s,
        (view.max.y as f64 - origin.y as f64 - cam.y) / s,
    ]
}

/// Per group: anchor, translation, k and m, as f32.
fn group_params(scene: &Scene) -> Vec<[f32; 6]> {
    scene
        .groups
        .iter()
        .map(|g| [g.anchor[0] as f32, g.anchor[1] as f32, g.t[0] as f32, g.t[1] as f32, g.k as f32, g.m as f32])
        .collect()
}

/// Whether an element's bounds (base map coordinates, moved by its shift)
/// meet the view, once placed by its group.
pub fn meets(scene: &Scene, i: usize, view: &[f64; 4]) -> bool {
    let Some(b) = scene.elems[i].bounds else { return false };
    box_meets(scene, i, [b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64], view)
}

/// Whether a box around element `i`'s geometry as it was made (base map
/// coordinates; its shift is added) meets the view, once placed by its group.
pub fn box_meets(scene: &Scene, i: usize, b: [f64; 4], view: &[f64; 4]) -> bool {
    let d = &scene.elems[i];
    let [sx, sy] = d.shift;
    let b = [b[0] + sx, b[1] + sy, b[2] + sx, b[3] + sy];
    let (x0, y0, x1, y1) = if d.group == 0 {
        (b[0], b[1], b[2], b[3])
    } else {
        let g = &scene.groups[d.group as usize];
        let (ax, ay) = g.place(b[0], b[1]);
        let (bx, by) = g.place(b[2], b[3]);
        // Strokes keep or scale their width: pad for the difference.
        let pad = ((g.m - g.k).abs() * 8.0).max(0.0);
        (ax.min(bx) - pad, ay.min(by) - pad, ax.max(bx) + pad, ay.max(by) + pad)
    };
    x1 >= view[0] && x0 <= view[2] && y1 >= view[1] && y0 <= view[3]
}

/// The mesh an element is drawn with at a camera scale: its own, or for a
/// text the level of detail its size on screen calls for (the nearest one
/// placed, if that one is not yet).
pub fn element_mesh(scene: &Scene, i: usize, scale: f64) -> Option<&crate::tess::MeshBuf> {
    let d = &scene.elems[i];
    if d.text.is_none() {
        return (!d.mesh.is_empty()).then_some(&d.mesh);
    }
    let want = scene.text_level(i, scale);
    let order: [usize; 3] = match want {
        0 => [0, 1, 2],
        1 => [1, 0, 2],
        _ => [2, 1, 0],
    };
    order.iter().find_map(|l| d.lods[*l].as_ref()).filter(|m| !m.is_empty())
}

/// The screen mesh of the visible part of the scene.
pub fn screen_mesh(scene: &Scene, cam: &Camera, origin: Pos2, view: Rect) -> (Mesh, usize) {
    let s = (if cam.scale > 0.0 { cam.scale } else { 1.0 }) as f32;
    let ox = origin.x + cam.x as f32;
    let oy = origin.y + cam.y as f32;
    let groups = group_params(scene);
    let map_view = view_in_map(cam, origin, view);
    let zoom = scene.zoom;
    let mut mesh = Mesh::default();
    let mut drawn = 0;
    // Size the buffers once.
    let mut nv = 0;
    let mut ni = 0;
    let mut show: Vec<usize> = Vec::with_capacity(scene.elems.len());
    for (i, d) in scene.elems.iter().enumerate() {
        let Some(m) = element_mesh(scene, i, cam.scale) else { continue };
        if !d.shown(&zoom) || !meets(scene, i, &map_view) {
            continue;
        }
        nv += m.verts.len();
        ni += m.indices.len();
        show.push(i);
    }
    mesh.vertices.reserve(nv);
    mesh.indices.reserve(ni);
    for i in show {
        let d = &scene.elems[i];
        let Some(src) = element_mesh(scene, i, cam.scale) else { continue };
        let base = mesh.vertices.len() as u32;
        // A shifted element: its vertices moved by `shift`, which its group
        // scales by its `k` like any other move of a base position.
        let k = if d.group == 0 { 1.0 } else { scene.groups[d.group as usize].k };
        let ox = ox + (s as f64 * k * d.shift[0]) as f32;
        let oy = oy + (s as f64 * k * d.shift[1]) as f32;
        if d.group == 0 {
            mesh.vertices.extend(src.verts.iter().map(|v| Vertex {
                pos: Pos2::new(ox + s * (v.pos[0] + v.offset[0]), oy + s * (v.pos[1] + v.offset[1])),
                uv: WHITE_UV,
                color: v.color,
            }));
        } else {
            let g = groups[d.group as usize];
            mesh.vertices.extend(src.verts.iter().map(|v| {
                let x = g[0] + g[2] + g[4] * (v.pos[0] - g[0]) + g[5] * v.offset[0];
                let y = g[1] + g[3] + g[4] * (v.pos[1] - g[1]) + g[5] * v.offset[1];
                Vertex { pos: Pos2::new(ox + s * x, oy + s * y), uv: WHITE_UV, color: v.color }
            }));
        }
        mesh.indices.extend(src.indices.iter().map(|k| k + base));
        drawn += 1;
    }
    (mesh, drawn)
}

/// Draw the scene: `view` is the screen rectangle to fill (the stage), clipped
/// to the canvas (the SVG's own viewport).
pub fn paint(painter: &egui::Painter, scene: &Scene, cam: &Camera, origin: Pos2, view: Rect, cache: &mut MeshCache) {
    let key = Key {
        revision: scene.revision,
        camera: [cam.x, cam.y, cam.scale],
        view: [view.min.x, view.min.y, view.max.x, view.max.y],
        origin: [origin.x, origin.y],
    };
    let mesh = match (&cache.key, &cache.mesh) {
        (Some(k), Some(m)) if *k == key => m.clone(),
        _ => {
            let (mesh, drawn) = screen_mesh(scene, cam, origin, view);
            let mesh = Arc::new(mesh);
            cache.key = Some(key);
            cache.mesh = Some(mesh.clone());
            cache.drawn = drawn;
            mesh
        }
    };
    let clip = canvas_clip(painter, scene, cam, origin, view);
    if clip.is_positive() {
        painter.with_clip_rect(clip).add(Shape::Mesh(mesh));
    }
}

/// What the map is clipped to on screen: the view, within the canvas (the
/// SVG's own viewport) and the painter's clip.
pub fn canvas_clip(painter: &egui::Painter, scene: &Scene, cam: &Camera, origin: Pos2, view: Rect) -> Rect {
    let (w, h) = scene.canvas_size();
    let s = cam.scale as f32;
    let canvas = Rect::from_min_size(
        Pos2::new(origin.x + cam.x as f32, origin.y + cam.y as f32),
        egui::vec2(w as f32 * s, h as f32 * s),
    );
    view.intersect(canvas).intersect(painter.clip_rect())
}
