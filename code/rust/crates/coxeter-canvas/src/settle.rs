//! The settled layer: once the camera and the map have been still for a
//! moment ([`SETTLE_SECONDS`]) and no gesture holds the stage, the part of the
//! map in view is rasterised by resvg at the screen's own pixels (the
//! element tree drawn, styled by the stylesheet's resolver and inlined, as the
//! PNG export and the painter parity tests rasterise it) and shown over the
//! meshes, exactly covering them. That is what a browser does with the SVG on
//! the stage: exact coverage antialiasing, so thin lines and small type are as
//! sharp as the HTML app's. The meshes stay underneath and take over the
//! moment anything moves.
//!
//! The raster is made on a thread of its own; the interface never waits for
//! it. Each request has a generation: a camera that moves, a repaint, a
//! gesture or a change of theme starts a new one, which hides what is shown
//! and makes any raster still being drawn stale (it stops between its steps
//! and its picture is thrown away). Hit-testing is the meshes' and is not
//! affected.
//!
//! What is drawn is the tree cut down to what is in view (bar `defs`), with
//! what `applyCamera()` sets on the DOM at the zoom (`updateHitTargets`,
//! `updateEditHandles`), the hovered element hovered.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, LazyLock};

use coxeter_core::js;
use coxeter_render::{Element, InlineOptions, Node, inline_styles};
use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use resvg::{tiny_skia, usvg};

use crate::camera::{Camera, MIN_HIT_PX, ZoomVars};
use crate::scene::Scene;
use crate::style::PageTheme;

/// How long the camera and the map must be still before the map is
/// rasterised (seconds).
pub const SETTLE_SECONDS: f64 = 0.18;
/// The largest raster made, in pixels (16 megapixels, 64 MB), and the
/// longest side (what a GPU texture surely takes).
pub const MAX_PIXELS: u64 = 1 << 24;
pub const MAX_SIDE: u32 = 8192;

/// What a raster shows: when any of it changes, the raster is stale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettleKey {
    /// The scene's revision (a repaint, a hover, a zoom's restyle).
    pub revision: u64,
    pub camera: [f64; 3],
    /// The raster's box in physical pixels: left, top, width, height.
    pub pixels: [i64; 4],
    pub pixels_per_point: f32,
    pub dark: bool,
}

/// What [`Schedule::frame`] asks of the widget.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Plan {
    /// Hide (and free) the raster shown, if any.
    pub hide: bool,
    /// Start a raster of this generation.
    pub start: Option<u64>,
    /// Paint again in this many seconds (the wait for stillness).
    pub wait: Option<f64>,
}

/// When to make a raster, when it is current, when to drop it.
#[derive(Debug, Clone, Default)]
pub struct Schedule {
    key: Option<SettleKey>,
    /// When `key` was first seen (seconds).
    since: f64,
    generation: u64,
    /// The generation of the raster being made, if one is.
    in_flight: Option<u64>,
    /// The raster of `key` is done (shown, or there is nothing to show).
    settled: bool,
}

impl Schedule {
    /// One frame: what is in view now (`None`: nothing to rasterise), whether
    /// a gesture holds the stage, and the time.
    pub fn frame(&mut self, key: Option<SettleKey>, busy: bool, now: f64) -> Plan {
        let mut plan = Plan::default();
        let key = if busy { None } else { key };
        if key != self.key {
            plan.hide = self.settled || self.in_flight.is_some();
            self.key = key;
            self.since = now;
            self.settled = false;
            // A raster under way is stale from now on.
            self.generation += 1;
        }
        if self.key.is_none() || self.settled {
            return plan;
        }
        // One raster at a time: a stale one is let finish first.
        if self.in_flight.is_some() {
            return plan;
        }
        let still = now - self.since;
        if still < SETTLE_SECONDS {
            plan.wait = Some(SETTLE_SECONDS - still);
            return plan;
        }
        self.in_flight = Some(self.generation);
        plan.start = Some(self.generation);
        plan
    }

    /// A raster of `generation` is done: whether it is the current one (to
    /// show; `false`: throw it away).
    pub fn finished(&mut self, generation: u64) -> bool {
        if self.in_flight == Some(generation) {
            self.in_flight = None;
        }
        let current = generation == self.generation && self.key.is_some();
        if current {
            self.settled = true;
        }
        current
    }

    /// The generation the current raster must have.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether a raster is being made.
    pub fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }
}

/// A raster to make, with everything it needs (made on the interface
/// thread, drawn on another).
pub struct Job {
    pub generation: u64,
    /// The tree in view.
    pub tree: Element,
    /// Where the hovered element is in it: the child slots from the root.
    pub hovered: Option<Vec<usize>>,
    pub theme: PageTheme,
    /// The camera scale (map units to points).
    pub scale: f64,
    /// Map units to raster pixels: `x_px = k·x + tx`.
    pub k: f64,
    pub tx: f64,
    pub ty: f64,
    pub width: u32,
    pub height: u32,
    pub background: Color32,
}

/// The settled layer of a [`crate::MapView`].
#[derive(Default)]
pub struct SettledLayer {
    pub schedule: Schedule,
    texture: Option<(Rect, TextureHandle)>,
    /// The raster under way: its generation, and where it will arrive.
    results: Option<(u64, Receiver<Option<ColorImage>>)>,
    /// The generation a raster under way must still have (it stops early
    /// otherwise).
    current: Arc<AtomicU64>,
}

impl SettledLayer {
    /// Whether a raster is shown.
    pub fn shown(&self) -> bool {
        self.texture.is_some()
    }

    /// One frame of the widget: schedule, collect, and paint the raster
    /// over the meshes, clipped to `clip` (what the meshes are clipped to).
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        ui: &egui::Ui,
        scene: Option<&Scene>,
        cam: &Camera,
        view: Rect,
        clip: Rect,
        busy: bool,
        background: Color32,
    ) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let ppp = ctx.pixels_per_point();
        let key = scene.and_then(|s| settle_key(s, cam, clip, ppp));
        let plan = self.schedule.frame(key, busy, now);
        self.current.store(self.schedule.generation(), Ordering::Relaxed);
        if plan.hide {
            self.texture = None;
        }
        if let Some(wait) = plan.wait {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        if let (Some(generation), Some(scene), Some(key)) = (plan.start, scene, key) {
            let job = job(scene, cam, view, &key, generation, background);
            let (tx, rx) = channel();
            self.results = Some((generation, rx));
            let current = self.current.clone();
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new().name("coxeter-canvas-settle".into()).spawn(move || {
                let image = rasterize(job, &current);
                let _ = tx.send(image);
                repaint.request_repaint();
            });
            if spawned.is_err() {
                self.results = None;
                self.schedule.finished(generation);
            }
        }
        if let Some((generation, rx)) = &self.results {
            let generation = *generation;
            match rx.try_recv() {
                Ok(image) => {
                    self.results = None;
                    if self.schedule.finished(generation)
                        && let (Some(image), Some(key)) = (image, key)
                    {
                        let [x, y, w, h] = key.pixels;
                        let rect = Rect::from_min_size(
                            Pos2::new(x as f32 / ppp, y as f32 / ppp),
                            Vec2::new(w as f32 / ppp, h as f32 / ppp),
                        );
                        self.texture = Some((rect, ctx.load_texture("coxeter-settled-map", image, TextureOptions::NEAREST)));
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    // The thread died without a picture.
                    self.results = None;
                    self.schedule.finished(generation);
                }
            }
        }
        if let Some((rect, texture)) = &self.texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().with_clip_rect(clip).image(texture.id(), *rect, uv, Color32::WHITE);
        }
    }
}

/// What a raster of the scene seen through `cam`, clipped to `clip`
/// (points), would show; `None` when there is nothing to draw or the raster
/// would be too large.
pub fn settle_key(scene: &Scene, cam: &Camera, clip: Rect, ppp: f32) -> Option<SettleKey> {
    if !clip.is_positive() || !(ppp > 0.0) || !(cam.scale > 0.0) {
        return None;
    }
    let x0 = (clip.min.x * ppp).floor() as i64;
    let y0 = (clip.min.y * ppp).floor() as i64;
    let x1 = (clip.max.x * ppp).ceil() as i64;
    let y1 = (clip.max.y * ppp).ceil() as i64;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0 || h <= 0 || w > MAX_SIDE as i64 || h > MAX_SIDE as i64 || (w * h) as u64 > MAX_PIXELS {
        return None;
    }
    Some(SettleKey {
        revision: scene.revision,
        camera: [cam.x, cam.y, cam.scale],
        pixels: [x0, y0, w, h],
        pixels_per_point: ppp,
        dark: scene.theme == PageTheme::dark(),
    })
}

/// The raster to make for a key: the tree cut down to what is in view, and
/// where it goes.
pub fn job(scene: &Scene, cam: &Camera, view: Rect, key: &SettleKey, generation: u64, background: Color32) -> Job {
    let [x0, y0, w, h] = key.pixels;
    let ppp = key.pixels_per_point as f64;
    // The part of the map the raster covers, in map units.
    let s = cam.scale;
    let origin = view.min;
    let map_view = [
        (x0 as f64 / ppp - origin.x as f64 - cam.x) / s,
        (y0 as f64 / ppp - origin.y as f64 - cam.y) / s,
        ((x0 + w) as f64 / ppp - origin.x as f64 - cam.x) / s,
        ((y0 + h) as f64 / ppp - origin.y as f64 - cam.y) / s,
    ];
    let keep = in_view(scene, &map_view);
    let mut index = 0;
    let mut slots = Vec::new();
    let mut hovered = None;
    let tree = prune(&scene.doc.root, &keep, &mut index, scene.hovered.map(|h| h as usize), &mut slots, &mut hovered)
        .unwrap_or_else(|| scene.doc.root.clone());
    Job {
        generation,
        tree,
        hovered,
        theme: scene.theme.clone(),
        scale: s,
        k: ppp * s,
        tx: ppp * (origin.x as f64 + cam.x) - x0 as f64,
        ty: ppp * (origin.y as f64 + cam.y) - y0 as f64,
        width: w as u32,
        height: h as u32,
        background,
    }
}

/// Which elements a raster of `view` (map units) needs: those drawn there,
/// their ancestors, and what other elements refer to (`defs`, patterns,
/// filters). Everything when the stylesheet looks at siblings: a cut tree
/// would not be styled as the whole is.
fn in_view(scene: &Scene, view: &[f64; 4]) -> Vec<bool> {
    let n = scene.doc.len();
    if crate::style::sibling_sensitive() {
        return vec![true; n];
    }
    let mut keep = vec![false; n];
    for i in (0..n).rev() {
        let d = &scene.elems[i];
        let name = scene.doc.element(i).name;
        let own = if d.hidden {
            // `defs` and what is in them; titles draw nothing.
            !matches!(name, "title" | "desc" | "metadata")
        } else {
            d.shown(&scene.zoom)
                && (crate::painter::meets(scene, i, view)
                    || d.hit_bounds.is_some_and(|b| crate::painter::box_meets(scene, i, b, view)))
        };
        if own || i == 0 {
            keep[i] = true;
        }
        if keep[i]
            && let Some(p) = scene.doc.parent(i)
        {
            keep[p] = true;
        }
    }
    keep
}

/// A copy of the tree with only the elements kept (text kept), and where
/// the hovered element ended up in it.
fn prune(
    e: &Element,
    keep: &[bool],
    index: &mut usize,
    hovered: Option<usize>,
    slots: &mut Vec<usize>,
    found: &mut Option<Vec<usize>>,
) -> Option<Element> {
    let me = *index;
    *index += 1;
    let size = |e: &Element| -> usize {
        fn count(e: &Element) -> usize {
            1 + e.elements().map(count).sum::<usize>()
        }
        count(e)
    };
    if !keep.get(me).copied().unwrap_or(false) {
        *index = me + size(e);
        return None;
    }
    if hovered == Some(me) {
        *found = Some(slots.clone());
    }
    let mut out = Element { name: e.name, attrs: e.attrs.clone(), children: Vec::new(), close_ws: e.close_ws, paired: e.paired };
    for c in &e.children {
        match c {
            Node::Text(t) => out.children.push(Node::Text(t.clone())),
            Node::Element(child) => {
                slots.push(out.children.len());
                if let Some(kept) = prune(child, keep, index, hovered, slots, found) {
                    out.children.push(Node::Element(kept));
                }
                slots.pop();
            }
        }
    }
    Some(out)
}

/// `updateHitTargets()` and `updateEditHandles()` of `app.js`: what the
/// camera writes on the DOM at a zoom (the scene draws them by its groups).
pub fn apply_camera(e: &mut Element, scale: f64) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let wanted = MIN_HIT_PX / scale;
    let number = |e: &Element, name: &str| e.get(name).map(js::string_to_number).unwrap_or(f64::NAN);
    let or = |v: f64, d: f64| if v == 0.0 || v.is_nan() { d } else { v };
    if e.has_class("emc-multi-drag-hit") {
        e.set("stroke-width", js::number_to_string(js::max(12.0, wanted)));
    }
    if e.has_class("emc-hit") {
        let base = or(number(e, "data-hit-base"), 0.0);
        e.set("r", js::number_to_string(js::max(base, wanted)));
    }
    if e.get("data-edit-handle-base").is_some() {
        let base = or(number(e, "data-edit-handle-base"), 5.5);
        e.set("r", js::number_to_string(js::min(28.0, js::max(base, 7.0 / scale))));
        let width = js::number_to_string(js::min(6.0, js::max(1.8, 1.8 / scale)));
        let style = match e.get("style").filter(|s| !s.trim().is_empty()) {
            Some(s) => format!("{s}; stroke-width: {width}"),
            None => format!("stroke-width: {width}"),
        };
        e.set("style", style);
    }
    let handle = e.get("data-rotate-handle").is_some();
    if handle || e.get("data-rotate-guide").is_some() {
        let arm = coxeter_render::rotation_arm(MIN_HIT_PX / scale);
        let x = number(e, "data-rotate-ox") + number(e, "data-rotate-ux") * arm;
        let y = number(e, "data-rotate-oy") + number(e, "data-rotate-uy") * arm;
        if x.is_finite() && y.is_finite() {
            let (nx, ny) = if handle { ("cx", "cy") } else { ("x2", "y2") };
            e.set(nx, js::number_to_string(x));
            e.set(ny, js::number_to_string(y));
        }
    }
    for c in &mut e.children {
        if let Node::Element(child) = c {
            apply_camera(child, scale);
        }
    }
}

/// The system fonts, for usvg, read once.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    });
    DB.clone()
}

/// Draw a job: the tree as the camera leaves it, styled and inlined,
/// parsed by usvg and rasterised by resvg on the stage's background.
/// `None` when it went stale (`current` moved on) or could not be drawn.
pub fn rasterize(mut job: Job, current: &AtomicU64) -> Option<ColorImage> {
    let stale = || current.load(Ordering::Relaxed) != job.generation;
    apply_camera(&mut job.tree, job.scale);
    let hovered = job.hovered.as_ref().and_then(|slots| {
        let mut e = &job.tree;
        for &k in slots {
            e = match e.children.get(k)? {
                Node::Element(c) => c,
                Node::Text(_) => return None,
            };
        }
        Some(e)
    });
    let opts = InlineOptions { resolve: job.theme.options(ZoomVars::at(job.scale), hovered), device_scale: job.scale };
    let inlined = inline_styles(&job.tree, coxeter_render::css::map_stylesheet(), &opts);
    if stale() {
        return None;
    }
    let xml = inlined.to_xml();
    drop(inlined);
    let options = usvg::Options { fontdb: fonts(), ..usvg::Options::default() };
    let tree = usvg::Tree::from_str(&xml, &options).ok()?;
    if stale() {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(job.width, job.height)?;
    let bg = job.background;
    pixmap.fill(tiny_skia::Color::from_rgba8(bg.r(), bg.g(), bg.b(), 255));
    let (k, tx, ty) = (job.k as f32, job.tx as f32, job.ty as f32);
    resvg::render(&tree, tiny_skia::Transform::from_row(k, 0.0, 0.0, k, tx, ty), &mut pixmap.as_mut());
    if stale() {
        return None;
    }
    Some(ColorImage::from_rgba_premultiplied([job.width as usize, job.height as usize], pixmap.data()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(revision: u64, x: f64) -> Option<SettleKey> {
        Some(SettleKey { revision, camera: [x, 0.0, 1.0], pixels: [0, 0, 100, 80], pixels_per_point: 1.0, dark: false })
    }

    #[test]
    fn a_raster_is_made_once_still_and_dropped_on_any_change() {
        let mut s = Schedule::default();
        // Still for less than the delay: wait.
        let p = s.frame(key(1, 0.0), false, 10.0);
        assert_eq!((p.start, p.hide), (None, false));
        assert!(p.wait.is_some_and(|w| (w - SETTLE_SECONDS).abs() < 1e-9));
        assert_eq!(s.frame(key(1, 0.0), false, 10.1).start, None);
        // Still long enough: one raster, once.
        let g = s.frame(key(1, 0.0), false, 10.0 + SETTLE_SECONDS + 0.01).start.expect("a raster");
        assert_eq!(s.frame(key(1, 0.0), false, 10.5).start, None, "one at a time");
        assert!(s.finished(g), "current: shown");
        assert_eq!(s.frame(key(1, 0.0), false, 11.0), Plan::default(), "nothing more to do");
        // A pan hides it at once, and the wait starts again.
        let p = s.frame(key(1, 5.0), false, 11.1);
        assert!(p.hide && p.start.is_none());
        let g = s.frame(key(1, 5.0), false, 11.1 + SETTLE_SECONDS + 0.01).start.expect("a raster of the new view");
        // The map repainted while it was being made: that raster is stale.
        let p = s.frame(key(2, 5.0), false, 11.4);
        assert!(p.hide);
        assert!(!s.finished(g), "stale: thrown away");
        let g = s.frame(key(2, 5.0), false, 11.4 + SETTLE_SECONDS + 0.01).start.expect("a raster of the repaint");
        assert!(s.finished(g));
        // A gesture hides it, and nothing is made while it lasts.
        assert!(s.frame(key(2, 5.0), true, 12.0).hide);
        assert_eq!(s.frame(key(2, 5.0), true, 13.0).start, None);
        // Nothing to draw (switched off, too large): nothing made.
        assert_eq!(s.frame(None, false, 14.0).start, None);
        assert_eq!(s.frame(None, false, 15.0).start, None);
    }

    #[test]
    fn the_camera_resizes_handles_as_app_js_does() {
        let mut tree = Element::paired("svg")
            .child(Element::new("circle").with("class", "emc-hit").with("data-hit-base", "4").with("r", "4"))
            .child(Element::new("circle").with("data-edit-handle-base", "5.5").with("r", "5.5"));
        apply_camera(&mut tree, 0.5);
        let kids: Vec<&Element> = tree.elements().collect();
        assert_eq!(kids[0].get("r"), Some("24"));
        assert_eq!(kids[1].get("r"), Some("14"));
        assert_eq!(kids[1].get("style"), Some("stroke-width: 3.6"));
    }
}
