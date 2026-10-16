//! How fast the canvas is on a big map.
//!
//! ```text
//! cargo run --release -p coxeter-canvas --example big_map [map.json] [frames]
//! ```
//!
//! Loads a saved map (default: the largest gut9 snapshot, 1171 reactions,
//! 3264 metabolites), and reports: the time `render_map` takes to draw the
//! element tree; styling and tessellation of the whole scene; the per-frame
//! cost of drawing it through the camera (pan and zoom, which re-tessellate
//! nothing) on the CPU path; one drag step (a metabolite moved: the tree drawn
//! again, only what changed restyled and tessellated); the hover hit-test;
//! and a whole drag through the stage's gestures (`MapCanvas` over the test
//! host): pointer down, moves, up, with the drawing brought up to date after
//! each event as the widget does each frame.

use std::time::Instant;

use coxeter_canvas::camera::{Camera, StageRect, fit_to_stage, min_hit_radius};
use coxeter_canvas::painter;
use coxeter_canvas::scene::Scene;
use coxeter_canvas::style::PageTheme;
use coxeter_core::mapdata::MapData;
use coxeter_map::EditableMap;
use coxeter_render::{RenderState, render_map};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = coxeter_core::testkit::repo_root();
    let path = args
        .get(1)
        .filter(|a| !a.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("docs/gut9-map/maps/12-phase7-Ba-Bt-envelope.json"));
    let frames: usize = args.get(2).and_then(|f| f.parse().ok()).unwrap_or(60);
    let text = std::fs::read_to_string(&path).expect("a map file");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let raw = doc.get("map").cloned().unwrap_or(doc);
    let data: MapData = serde_json::from_value(raw).expect("a map");
    let mut map = EditableMap::from_data(data);
    let g = map.graph();
    println!(
        "map {}: {} reactions, {} metabolites, {} segments, canvas {}x{}",
        path.file_name().unwrap().to_string_lossy(),
        g.reactions.len(),
        g.nodes.len(),
        g.segments.len(),
        g.width(),
        g.height()
    );

    let stage = StageRect::new(0.0, 0.0, 1600.0, 1000.0);
    let mut cam = Camera::default();
    fit_to_stage(&mut cam, g.width(), g.height(), stage);
    let state = RenderState { min_hit_radius: min_hit_radius(cam.scale), ..RenderState::default() };

    let t = Instant::now();
    let tree = render_map(map.graph(), &state);
    let render_ms = t.elapsed().as_secs_f64() * 1e3;
    let count = tree.descendants().len();
    println!("render_map: {render_ms:.1} ms, {count} elements");

    // Fonts are loaded once per process; do it before timing the scene.
    let t = Instant::now();
    drop(coxeter_canvas::text::font_book());
    println!("system fonts: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

    let t = Instant::now();
    let mut scene = Scene::build(tree, PageTheme::light(), cam.scale);
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let s = scene.stats;
    println!(
        "scene build: {build_ms:.1} ms (styles {:.1} ms, tessellation {:.1} ms): {} vertices, {} triangles, {} groups",
        s.style_ms,
        s.tessellate_ms,
        s.vertices,
        s.triangles,
        scene.groups.len() - 1
    );

    if std::env::var("BREAKDOWN").is_ok() {
        breakdown(&scene);
    }
    // Per frame, CPU path: pan across, then zoom in and out (no tessellation).
    let origin = egui::Pos2::ZERO;
    let view = egui::Rect::from_min_size(origin, egui::vec2(stage.width as f32, stage.height as f32));
    let mut times = Vec::new();
    let mut drawn = 0;
    for f in 0..frames {
        let mut c = cam;
        c.x += (f as f64 * 7.0) % 300.0;
        let t = Instant::now();
        scene.ensure_lods(c.scale, painter::view_in_map(&c, origin, view));
        let (mesh, n) = painter::screen_mesh(&scene, &c, origin, view);
        times.push(t.elapsed().as_secs_f64() * 1e3);
        drawn = n;
        std::hint::black_box(mesh);
    }
    report("CPU frame, fitted view (pan)", &mut times, drawn);
    let mut times = Vec::new();
    for f in 0..frames {
        let mut c = cam;
        let factor = 1.0 + (f % 20) as f64 * 0.15;
        coxeter_canvas::camera::zoom_camera_at(&mut c, factor, stage, None);
        let t0 = Instant::now();
        scene.set_scale(c.scale);
        scene.ensure_lods(c.scale, painter::view_in_map(&c, origin, view));
        let (mesh, n) = painter::screen_mesh(&scene, &c, origin, view);
        times.push(t0.elapsed().as_secs_f64() * 1e3);
        drawn = n;
        std::hint::black_box(mesh);
    }
    report("CPU frame, zooming (labels rescaled, live parts redone)", &mut times, drawn);
    scene.set_scale(cam.scale);

    // One drag step: a metabolite moved, the tree drawn again, the scene updated.
    let node = map.graph().nodes.values().find(|n| map.can_move_node(&n.id)).map(|n| (n.id.clone(), n.x, n.y));
    if let Some((id, x, y)) = node {
        let mut times = Vec::new();
        for step in 0..10 {
            map.move_node(&id, x + 3.0 * (step + 1) as f64, y + 2.0);
            let t = Instant::now();
            let tree = render_map(map.graph(), &state);
            let r = t.elapsed().as_secs_f64() * 1e3;
            scene.update(tree);
            times.push(t.elapsed().as_secs_f64() * 1e3);
            if step == 0 {
                println!(
                    "drag step: render_map {r:.1} ms, {} elements redone in {:.1} ms",
                    scene.stats.rebuilt, scene.stats.tessellate_ms
                );
            }
        }
        report("drag step total (render + diff + restyle + tessellate)", &mut times, scene.stats.rebuilt);
    }
    stage_bench(map.clone(), cam, stage);
    compartment_bench(map, stage);
}

/// A whole compartment dragged, with everything in it: by its move handle
/// (`editGeometryRect`) and as a multi-selection that encloses it
/// (`moveMultiSelection`), through the stage's gestures, each step followed
/// by the repaint the widget makes. The camera shows the compartment.
fn compartment_bench(map: EditableMap, stage: StageRect) {
    use coxeter_canvas::MapCanvas;
    use coxeter_canvas::gestures::PointerInput;
    use coxeter_canvas::host::MapHost;
    use coxeter_canvas::test_host::EditableMapHost;
    use coxeter_core::mapdata::Point;

    // The inner compartment with the most metabolites in it, among those
    // with room to move right and down inside their parent.
    let inside = |r: &coxeter_core::mapdata::Rect, x: f64, y: f64| x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h;
    let pick = map
        .cells()
        .iter()
        .flat_map(|c| {
            c.compartments.iter().filter_map(move |k| {
                let parent = c.compartments.iter().find(|p| Some(&p.id) == k.parent_id.as_ref())?;
                let room = (parent.rect.x + parent.rect.w - k.rect.x - k.rect.w).min(parent.rect.y + parent.rect.h - k.rect.y - k.rect.h);
                (room > 250.0).then(|| (c.id.clone(), k.id.clone(), k.rect))
            })
        })
        .max_by_key(|(_, _, r)| map.graph().nodes.values().filter(|n| inside(r, n.x, n.y)).count());
    let Some((cell_id, comp_id, rect)) = pick else { return };
    let count = map.graph().nodes.values().filter(|n| inside(&rect, n.x, n.y)).count();
    println!("compartment {cell_id}/{comp_id}: {count} metabolites inside");

    let mut host = EditableMapHost::new(map);
    let mut canvas = MapCanvas::new();
    canvas.set_rect(stage);
    let scale = (stage.width / rect.w).min(stage.height / rect.h) * 0.8;
    canvas.camera = Camera {
        x: stage.width / 2.0 - (rect.x + rect.w / 2.0) * scale,
        y: stage.height / 2.0 - (rect.y + rect.h / 2.0) * scale,
        scale,
        fitted: true,
    };
    let rev = |h: &EditableMapHost| h.emitted.len() as u64;
    canvas.sync(&host, rev(&host), 0.0);
    let drag = |canvas: &mut MapCanvas, host: &mut EditableMapHost, at: Point, what: &str| {
        let ev = |p: Point| PointerInput::mouse(p);
        canvas.pointer_down(host, &ev(at));
        canvas.sync(host, rev(host), 0.0);
        if !canvas.gestures.active() {
            println!("  ({what}: the press started no drag)");
            return;
        }
        let origin = egui::Pos2::new(stage.left as f32, stage.top as f32);
        let view = egui::Rect::from_min_size(origin, egui::vec2(stage.width as f32, stage.height as f32));
        let mut times = Vec::new();
        let (mut gesture, mut render, mut scene_ms, mut paint) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut rebuilt, mut shifted) = (0, 0);
        for k in 1..=30 {
            // Fractions of a pixel too, as a real pointer gives at most zooms.
            let p = Point::new(at.x + 1.37 * (k + 2) as f64, at.y + 0.93 * (k + 2) as f64);
            let t = Instant::now();
            canvas.pointer_move(host, &PointerInput { button: -1, ..ev(p) }, 0.0);
            gesture.push(t.elapsed().as_secs_f64() * 1e3);
            canvas.sync(host, rev(host), 0.0);
            times.push(t.elapsed().as_secs_f64() * 1e3);
            render.push(canvas.stats.render_ms);
            scene_ms.push(canvas.stats.scene_ms);
            rebuilt = rebuilt.max(canvas.stats.rebuilt);
            shifted = shifted.max(canvas.scene.as_ref().map_or(0, |s| s.stats.shifted));
            // What the widget then paints: the screen mesh of the view.
            let t = Instant::now();
            let cam = canvas.camera;
            let scene = canvas.scene.as_mut().unwrap();
            scene.ensure_lods(cam.scale, painter::view_in_map(&cam, origin, view));
            std::hint::black_box(painter::screen_mesh(scene, &cam, origin, view));
            paint.push(t.elapsed().as_secs_f64() * 1e3);
        }
        report(&format!("{what}: drag step (move + render_map + scene update)"), &mut times, rebuilt);
        println!("{what}:   {shifted} of the {rebuilt} elements redone only moved (shifted, not tessellated)");
        report(&format!("{what}:   of which the gesture (the map edit)"), &mut gesture, 0);
        report(&format!("{what}:   of which render_map"), &mut render, 0);
        report(&format!("{what}:   of which scene update"), &mut scene_ms, 0);
        report(&format!("{what}: then the screen mesh of the view"), &mut paint, 0);
        canvas.pointer_up(host, &ev(Point::new(at.x + 80.0, at.y + 50.0)), 0.0);
        canvas.sync(host, rev(host), 0.0);
    };

    // The move handle: select the compartment, press its middle.
    host.select_geometry(Some(&cell_id), &comp_id);
    canvas.sync(&host, rev(&host), 0.0);
    let handle = canvas.scene.as_ref().and_then(|s| {
        (0..s.doc.len()).map(|i| s.doc.element(i)).find(|e| e.has_class("emc-geometry-move")).map(|e| {
            let n = |a: &str| e.get(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            Point::new(n("cx"), n("cy"))
        })
    });
    if let Some(h) = handle {
        let at = canvas.map_to_screen(h);
        drag(&mut canvas, &mut host, at, "compartment move handle");
    }
    host.clear_geometry_selection();

    // The multi-selection: a rectangle around the compartment, then a press
    // on a selected metabolite inside it.
    let rect = host.map().unwrap().cells().iter().find(|c| c.id == cell_id).and_then(|c| c.compartments.iter().find(|k| k.id == comp_id)).map(|k| k.rect).unwrap();
    host.select_in_rect(rect.x - 2.0, rect.y - 2.0, rect.x + rect.w + 2.0, rect.y + rect.h + 2.0, true);
    canvas.sync(&host, rev(&host), 0.0);
    println!("multi-selection: {} items", host.multi_selection_size());
    let node = host.multi_selection.nodes.iter().find_map(|id| host.map().unwrap().graph().nodes.get(id).map(|n| Point::new(n.x, n.y)));
    if let Some(p) = node {
        let at = canvas.map_to_screen(p);
        drag(&mut canvas, &mut host, at, "multi-selection");
    }

    // The settled layer of this view: cut on the interface thread, drawn by
    // resvg on another (once to read the fonts, then timed).
    use coxeter_canvas::settle;
    let view = egui::Rect::from_min_size(egui::Pos2::new(stage.left as f32, stage.top as f32), egui::vec2(stage.width as f32, stage.height as f32));
    let scene = canvas.scene.as_ref().unwrap();
    let cam = canvas.camera;
    let Some(key) = settle::settle_key(scene, &cam, view, 1.0) else { return };
    let current = std::sync::atomic::AtomicU64::new(0);
    settle::rasterize(settle::job(scene, &cam, view, &key, 0, egui::Color32::WHITE), &current);
    let t = Instant::now();
    let job = settle::job(scene, &cam, view, &key, 0, egui::Color32::WHITE);
    let cut = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let image = settle::rasterize(job, &current);
    println!(
        "settled layer {}x{}: tree cut {cut:.1} ms (interface thread), raster {:.1} ms (its own thread){}",
        key.pixels[2],
        key.pixels[3],
        t.elapsed().as_secs_f64() * 1e3,
        if image.is_some() { "" } else { " — failed" }
    );
}

/// The hover hit-test and a node drag through the gesture state machine.
fn stage_bench(map: EditableMap, cam: Camera, stage: StageRect) {
    use coxeter_canvas::MapCanvas;
    use coxeter_canvas::gestures::PointerInput;
    use coxeter_canvas::test_host::EditableMapHost;
    use coxeter_core::mapdata::Point;

    let mut host = EditableMapHost::new(map);
    let mut canvas = MapCanvas::new();
    canvas.set_rect(stage);
    canvas.camera = cam;
    canvas.sync(&host, 0, 0.0);
    // Hover: the topmost element under the pointer, as each pointer move asks.
    let mut times = Vec::new();
    for k in 0..400 {
        let p = Point::new(stage.left + (k * 37 % 1600) as f64, stage.top + (k * 53 % 1000) as f64);
        let t = Instant::now();
        canvas.hover_at(Some(p));
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    report("hover (hit-test + :hover restyle)", &mut times, 0);
    // A drag: the first movable metabolite in view.
    let target = host.map.as_ref().unwrap().nodes().values().find_map(|n| {
        let at = canvas.map_to_screen(Point::new(n.x, n.y));
        let inside = at.x > stage.left + 40.0 && at.y > stage.top + 40.0 && at.x < stage.left + stage.width - 40.0 && at.y < stage.top + stage.height - 40.0;
        (inside && host.map.as_ref().unwrap().can_move_node(&n.id)).then_some(at)
    });
    let Some(at) = target else { return };
    let mut times = Vec::new();
    let ev = |p: Point| PointerInput::mouse(p);
    canvas.pointer_down(&mut host, &ev(at));
    canvas.sync(&host, host.emitted.len() as u64, 0.0);
    for k in 1..=20 {
        let p = Point::new(at.x + 2.0 * k as f64, at.y + 1.5 * k as f64);
        let t = Instant::now();
        canvas.pointer_move(&mut host, &PointerInput { button: -1, ..ev(p) }, 0.0);
        canvas.sync(&host, host.emitted.len() as u64, 0.0);
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let rebuilt = canvas.stats.rebuilt;
    report("drag step through the gestures (move + render_map + scene update)", &mut times, rebuilt);
    let t = Instant::now();
    canvas.pointer_up(&mut host, &ev(Point::new(at.x + 40.0, at.y + 30.0)), 0.0);
    canvas.sync(&host, host.emitted.len() as u64, 0.0);
    println!("drag release (merge check, history record, repaint): {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
}

fn report(what: &str, times: &mut [f64], n: usize) {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    println!("{what}: median {median:.2} ms, p95 {p95:.2} ms ({n} elements)");
}

/// Print the fifteen element kinds (tag and first class) with the most
/// vertices, to see what a scene's meshes are made of.
#[allow(dead_code)]
pub fn breakdown(scene: &Scene) {
    let mut by: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for (i, d) in scene.elems.iter().enumerate() {
        let e = scene.doc.element(i);
        let cls = e.get("class").unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
        let k = format!("{} {}", e.name, cls);
        let entry = by.entry(k).or_default();
        entry.0 += 1;
        entry.1 += d.mesh.verts.len() + d.lods.iter().flatten().map(|m| m.verts.len()).sum::<usize>();
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by_key(|(_, (_, n))| std::cmp::Reverse(*n));
    for (k, (c, n)) in v.iter().take(15) {
        println!("  {k}: {c} elements, {n} vertices");
    }
}
