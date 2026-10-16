//! The stage of `src/ui/app.js` without a window: the camera, the drawing
//! (`paintMap()` → [`Scene`]), the gestures and the wheel, fed with plain
//! events. [`crate::widget::MapView`] puts it on egui; tests drive it directly.
//!
//! The drawing is rebuilt when the caller's revision changes (the store
//! emitted), when a gesture changed the map (`schedulePaint()`), and once the
//! zoom settles far from the one the pointer targets were sized for
//! (`mapRenderState().minHitRadius`). A rebuild of a tree of the same shape
//! restyles and re-tessellates only the elements that differ, which is what
//! keeps a drag cheap.

use std::time::Instant;

use coxeter_core::mapdata::Point;
use coxeter_render::render_map;

use crate::camera::{self, Camera, StageRect, min_hit_radius};
use crate::gestures::{CanvasEvent, CanvasKey, Gestures, KeyMods, KeyOutcome, PointerInput, Stage, Tool};
use crate::hit;
use crate::host::{MapHost, RenderOptions};
use crate::marks::apply_marks;
use crate::scene::Scene;
use crate::style::PageTheme;
use crate::wheel::{self, WheelAction, WheelInput, WheelNavigation};

/// How far the zoom may drift from the one the drawing was made for before
/// the pointer targets are made again (a factor).
const RESIZE_TARGETS_AT: f64 = 1.5;
/// How long the wheel must rest before that happens (ms).
const ZOOM_SETTLE_MS: f64 = 180.0;

/// What the last rebuild cost.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RebuildStats {
    /// `render_map`, in ms.
    pub render_ms: f64,
    /// Styling and tessellation, in ms.
    pub scene_ms: f64,
    /// Elements restyled (every one on a full build).
    pub rebuilt: usize,
    pub elements: usize,
}

/// The stage: camera, drawing, gestures.
#[derive(Debug, Clone)]
pub struct MapCanvas {
    pub camera: Camera,
    /// The stage's box in client coordinates.
    pub rect: StageRect,
    pub scene: Option<Scene>,
    pub gestures: Gestures,
    pub wheel: WheelNavigation,
    /// `previousStageSize` of `syncStageViewport`.
    previous_size: Option<(f64, f64)>,
    built_revision: Option<u64>,
    /// The camera scale the drawing's pointer targets were made for.
    built_scale: f64,
    /// When the camera last zoomed (ms).
    last_zoom_ms: f64,
    pub stats: RebuildStats,
}

impl Default for MapCanvas {
    fn default() -> Self {
        MapCanvas {
            camera: Camera::default(),
            rect: StageRect::default(),
            scene: None,
            gestures: Gestures::new(),
            wheel: WheelNavigation::new(),
            previous_size: None,
            built_revision: None,
            built_scale: 1.0,
            last_zoom_ms: f64::NEG_INFINITY,
            stats: RebuildStats::default(),
        }
    }
}

impl MapCanvas {
    pub fn new() -> MapCanvas {
        MapCanvas::default()
    }

    /// The stage moved or changed size (`ResizeObserver` → `syncStageViewport`):
    /// a fitted camera keeps the middle of the view where it was.
    pub fn set_rect(&mut self, rect: StageRect) {
        // Maximised results give the map a zero-height track: keep the last
        // usable viewport so restoring the dock does not recentre twice.
        if rect.width <= 0.0 || rect.height <= 0.0 {
            self.rect.left = rect.left;
            self.rect.top = rect.top;
            return;
        }
        camera::sync_stage_viewport(&mut self.camera, self.previous_size, rect.width, rect.height);
        self.previous_size = Some((rect.width, rect.height));
        self.rect = rect;
    }

    fn stage(&mut self) -> (Stage<'_>, &mut Gestures) {
        (Stage { camera: &mut self.camera, rect: self.rect, scene: self.scene.as_ref() }, &mut self.gestures)
    }

    /// Bring the drawing up to date: rebuilt if the store changed
    /// (`revision`), a gesture moved something, or the zoom settled far from
    /// the one its pointer targets were made for; then fitted on the first
    /// paint, and its zoom-dependent parts brought to the camera.
    pub fn sync(&mut self, host: &dyn MapHost, revision: u64, now_ms: f64) {
        if host.map().is_none() {
            self.scene = None;
            self.built_revision = Some(revision);
            self.gestures.take_paint();
            return;
        }
        let settled = !self.gestures.active() && now_ms - self.last_zoom_ms > ZOOM_SETTLE_MS;
        let drift = (self.camera.scale / self.built_scale).max(self.built_scale / self.camera.scale);
        let stale_targets = settled && drift > RESIZE_TARGETS_AT;
        if self.gestures.take_paint() || self.built_revision != Some(revision) || self.scene.is_none() || stale_targets {
            self.rebuild(host);
            self.built_revision = Some(revision);
        }
        if !self.camera.fitted {
            self.fit_to_stage(host);
        }
        if let Some(scene) = &mut self.scene {
            scene.set_scale(self.camera.scale);
        }
    }

    /// `paintMap()`: draw the map as the store has it, with the marks of a
    /// drag in progress.
    pub fn rebuild(&mut self, host: &dyn MapHost) {
        let Some(map) = host.map() else {
            self.scene = None;
            return;
        };
        let t0 = Instant::now();
        let opts = RenderOptions { reaction_edit: self.gestures.tool == Tool::Edit, min_hit_radius: min_hit_radius(self.camera.scale) };
        let state = host.render_state(opts);
        let theme = PageTheme::for_dark(state.dark);
        let mut tree = render_map(map.graph(), &state);
        apply_marks(&mut tree, &self.gestures.marks(host));
        let t1 = Instant::now();
        match &mut self.scene {
            Some(scene) if scene.theme == theme => scene.update(tree),
            _ => self.scene = Some(Scene::build(tree, theme, self.camera.scale)),
        }
        self.built_scale = self.camera.scale;
        let scene = self.scene.as_ref().map(|s| (s.stats.rebuilt, s.elems.len())).unwrap_or_default();
        self.stats = RebuildStats {
            render_ms: (t1 - t0).as_secs_f64() * 1e3,
            scene_ms: t1.elapsed().as_secs_f64() * 1e3,
            rebuilt: scene.0,
            elements: scene.1,
        };
    }

    /// `fitToStage()`: the whole canvas, centred.
    pub fn fit_to_stage(&mut self, host: &dyn MapHost) {
        let Some(map) = host.map() else { return };
        if self.rect.width <= 0.0 || self.rect.height <= 0.0 {
            return;
        }
        camera::fit_to_stage(&mut self.camera, map.width(), map.height(), self.rect);
        self.camera_moved();
    }

    /// The zoom bar's fit button and the `0` key: `camera.fitted = false;
    /// fitToStage()`.
    pub fn fit(&mut self, host: &dyn MapHost) {
        self.camera.fitted = false;
        self.fit_to_stage(host);
    }

    /// `zoomBy(factor, origin)`: about a client point, or the stage's centre.
    pub fn zoom_by(&mut self, factor: f64, origin: Option<Point>) {
        camera::zoom_camera_at(&mut self.camera, factor, self.rect, origin);
        self.camera_moved();
    }

    fn camera_moved(&mut self) {
        if let Some(scene) = &mut self.scene {
            scene.set_scale(self.camera.scale);
        }
    }

    /// The stage's `wheel`: pan or zoom, as the classifier reads the event.
    pub fn wheel(&mut self, host: &dyn MapHost, event: &WheelInput, client: Point, now_ms: f64) {
        match self.wheel.classify(Some(event), host.scroll_input(), now_ms) {
            Some(WheelAction::Pan) => {
                let (dx, dy) = wheel::wheel_pan_delta(Some(event), wheel::DEFAULT_LINE_HEIGHT, self.rect.width, self.rect.height);
                self.camera.x -= dx;
                self.camera.y -= dy;
                self.camera.fitted = true;
            }
            Some(WheelAction::Zoom) => {
                self.zoom_by(wheel::wheel_zoom_factor(event.delta_y), Some(client));
                self.last_zoom_ms = now_ms;
            }
            None => {}
        }
    }

    /// A pinch the platform reports as one zoom factor (a touchpad's): about
    /// the pointer.
    pub fn zoom_gesture(&mut self, factor: f64, client: Point, now_ms: f64) {
        self.zoom_by(factor, Some(client));
        self.last_zoom_ms = now_ms;
    }

    /// The stage's `pointerdown`.
    pub fn pointer_down(&mut self, host: &mut dyn MapHost, ev: &PointerInput) {
        let (mut stage, gestures) = self.stage();
        gestures.pointer_down(&mut stage, host, ev);
        self.after_input();
    }

    /// The stage's `pointermove` at time `now_ms` (a pinch counts as a zoom).
    pub fn pointer_move(&mut self, host: &mut dyn MapHost, ev: &PointerInput, now_ms: f64) {
        let pinching = self.gestures.pinch.is_some();
        let (mut stage, gestures) = self.stage();
        gestures.pointer_move(&mut stage, host, ev);
        if pinching {
            self.last_zoom_ms = now_ms;
        }
        self.after_input();
    }

    /// The stage's `pointerup` at time `now_ms` (for the double click).
    pub fn pointer_up(&mut self, host: &mut dyn MapHost, ev: &PointerInput, now_ms: f64) {
        let (mut stage, gestures) = self.stage();
        gestures.pointer_up(&mut stage, host, ev, now_ms);
        self.after_input();
    }

    /// The stage's `pointercancel`.
    pub fn pointer_cancel(&mut self, host: &mut dyn MapHost, ev: &PointerInput) {
        self.gestures.pointer_cancel(host, ev);
        self.after_input();
    }

    /// A canvas key; whether it was used.
    pub fn key_down(&mut self, host: &mut dyn MapHost, key: CanvasKey, mods: KeyMods) -> bool {
        let (mut stage, gestures) = self.stage();
        let outcome = gestures.key_down(key, mods, &mut stage, host);
        self.after_input();
        match outcome {
            KeyOutcome::Ignored => false,
            KeyOutcome::Used => true,
            KeyOutcome::Fit => {
                self.fit(host);
                true
            }
            KeyOutcome::Zoom(f) => {
                self.zoom_by(f, None);
                true
            }
        }
    }

    /// `activateTool(tool)`.
    pub fn activate_tool(&mut self, tool: Tool, host: &mut dyn MapHost) {
        self.gestures.activate_tool(tool, host);
    }

    pub fn tool(&self) -> Tool {
        self.gestures.tool
    }

    /// The requests made since the last call.
    pub fn take_events(&mut self) -> Vec<CanvasEvent> {
        self.gestures.take_events()
    }

    fn after_input(&mut self) {
        if self.gestures.take_camera_moved() {
            self.camera_moved();
        }
    }

    /// `screenToMap(clientX, clientY)`.
    pub fn screen_to_map(&self, client: Point) -> Point {
        camera::screen_to_map(&self.camera, self.rect, client)
    }

    /// Where a map point is drawn, in client coordinates.
    pub fn map_to_screen(&self, p: Point) -> Point {
        camera::map_to_screen(&self.camera, self.rect, p)
    }

    /// The element the pointer is over, for `:hover` (none while a gesture
    /// holds the pointer: a captured pointer hovers the stage).
    pub fn hover_at(&mut self, client: Option<Point>) {
        let filter = self.gestures.hit_filter();
        let captured = self.gestures.captured.is_some() || self.gestures.active();
        let Some(scene) = &mut self.scene else { return };
        let hovered = match client {
            Some(c) if !captured => {
                let at = camera::screen_to_map(&self.camera, self.rect, c);
                hit::hit_test(scene, at, filter).map(|i| i as u32)
            }
            _ => None,
        };
        scene.set_hover(hovered);
    }

    /// `Number(camera.scale * 100)%`, the zoom readout.
    pub fn zoom_readout(&self) -> String {
        camera::zoom_readout(self.camera.scale)
    }
}
