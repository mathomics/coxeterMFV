//! [`MapView`]: the map on egui. The stage of `src/ui/app.js` (the camera,
//! the drawing and the gestures of [`MapCanvas`]) fed with egui's input, drawn
//! with egui's painter, with the overlays the JavaScript puts on the stage
//! next to the SVG: the selection rectangle (`selectionOverlay`), the cell and
//! compartment drawing boxes (`cellDrawOverlay`, `compartmentDrawOverlay`),
//! the parent a compartment must be drawn in (`compartmentParentOverlay`), the
//! card on an empty map (`emptyMapOverlay`) and the in-place text editor
//! (`renderTextEditor`).
//!
//! ```ignore
//! let mut view = MapView::new();                    // once
//! // each frame, `revision` bumped whenever the store emits:
//! let out = view.show(ui, &mut store, revision);
//! for event in out.events { /* open menus, status hints, … */ }
//! ```
//!
//! The widget keeps the camera; the caller keeps the store. Keys are read
//! while no text field has the keyboard and the pointer is on the map or the
//! map was the last thing clicked.

use std::collections::HashSet;

use coxeter_core::mapdata::{Point, Rect as MapRect};
use coxeter_core::text_style::{TEXT_FONTS, TEXT_SIZE, clamp_text_size};
use egui::{Color32, CornerRadius, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};

use crate::camera::{Camera, StageRect};
use crate::canvas::MapCanvas;
use crate::gestures::{CanvasEvent, CanvasKey, KeyMods, PointerInput, PointerType, Tool};
use crate::host::MapHost;
use crate::painter::{self, MeshCache};
use crate::settle::SettledLayer;
use crate::wheel::{DeltaMode, WheelInput};

/// The colours of `assets/app.css` the stage uses, light and dark.
#[derive(Debug, Clone, Copy)]
pub struct StageColors {
    pub bg: Color32,
    pub panel: Color32,
    pub ink: Color32,
    pub ink2: Color32,
    pub ink3: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    pub danger: Color32,
}

impl StageColors {
    pub fn for_dark(dark: bool) -> StageColors {
        let c = |hex: u32| Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
        if dark {
            StageColors {
                bg: c(0x0b1220),
                panel: c(0x141d2d),
                ink: c(0xecf1f8),
                ink2: c(0xbac7d9),
                ink3: c(0x9eafc6),
                line: c(0x334157),
                accent: c(0x60a5fa),
                accent_ink: c(0x0b1220),
                danger: c(0xfb7185),
            }
        } else {
            StageColors {
                bg: c(0xf3f6fa),
                panel: c(0xffffff),
                ink: c(0x0f172a),
                ink2: c(0x475569),
                ink3: c(0x5b6b80),
                line: c(0xdde5ef),
                accent: c(0x2563eb),
                accent_ink: c(0xffffff),
                danger: c(0xbe123c),
            }
        }
    }
}

/// `color-mix(in srgb, c p%, transparent)`.
fn mix(c: Color32, p: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (255.0 * p).round() as u8)
}

/// What one frame of the widget gives back.
pub struct MapViewOutput {
    pub response: egui::Response,
    /// Requests for the panels: menus, the status line, the inspector, the tool grid.
    pub events: Vec<CanvasEvent>,
}

/// The map widget.
pub struct MapView {
    pub canvas: MapCanvas,
    cache: MeshCache,
    id: egui::Id,
    touches: HashSet<u64>,
    /// The text editor's box last frame (client coordinates).
    editor_rect: Option<Rect>,
    dark: bool,
    /// Draw the map sharp once the camera settles: the view rasterised by
    /// resvg at the screen's pixels and shown over the meshes until anything
    /// moves ([`crate::settle`]). On by default.
    pub crisp: bool,
    settled: SettledLayer,
    /// The reaction whose row is under the pointer in a reactions table,
    /// ringed on the map (`highlightReactionOnMap`).
    pub table_hover: Option<String>,
}

impl Default for MapView {
    fn default() -> Self {
        MapView::new()
    }
}

impl MapView {
    pub fn new() -> MapView {
        MapView::with_id(egui::Id::new("coxeter-map-view"))
    }

    /// A map view with its own id (two maps on one screen).
    pub fn with_id(id: egui::Id) -> MapView {
        MapView {
            canvas: MapCanvas::new(),
            cache: MeshCache::default(),
            id,
            touches: HashSet::new(),
            editor_rect: None,
            dark: false,
            crisp: true,
            settled: SettledLayer::default(),
            table_hover: None,
        }
    }

    // What the toolbar calls

    pub fn tool(&self) -> Tool {
        self.canvas.tool()
    }

    /// `activateTool(tool)`.
    pub fn activate_tool(&mut self, tool: Tool, host: &mut dyn MapHost) {
        self.canvas.activate_tool(tool, host);
    }

    pub fn camera(&self) -> Camera {
        self.canvas.camera
    }

    /// Set the camera (a saved view, a test).
    pub fn set_camera(&mut self, camera: Camera) {
        self.canvas.camera = camera;
    }

    /// The zoom bar's buttons: `zoomBy(1.3)`, `zoomBy(1 / 1.3)`.
    pub fn zoom_by(&mut self, factor: f64) {
        self.canvas.zoom_by(factor, None);
    }

    /// The zoom bar's fit button.
    pub fn fit(&mut self, host: &dyn MapHost) {
        self.canvas.fit(host);
    }

    /// `centerOnReaction(reactionId)`.
    pub fn center_on_reaction(&mut self, host: &dyn MapHost, reaction_id: &str) -> bool {
        let Some(map) = host.map() else { return false };
        crate::camera::center_on_reaction(&mut self.canvas.camera, map.graph(), reaction_id, self.canvas.rect)
    }

    /// `centerOnPathway(pathwayKey)`, given the pathway's reactions.
    pub fn center_on_reactions(&mut self, host: &dyn MapHost, reaction_ids: &[String]) -> bool {
        let Some(map) = host.map() else { return false };
        crate::camera::center_on_reactions(&mut self.canvas.camera, map.graph(), reaction_ids, self.canvas.rect)
    }

    /// Whether the settled layer (the sharp raster of the view) is shown.
    pub fn crisp_shown(&self) -> bool {
        self.settled.shown()
    }

    /// `${Math.round(camera.scale * 100)}%`.
    pub fn zoom_readout(&self) -> String {
        self.canvas.zoom_readout()
    }

    /// `openTextEditor(...)` for an existing text (the inspector's Edit).
    pub fn edit_text(&mut self, host: &dyn MapHost, text_id: &str) {
        let Some(t) = host.map().and_then(|m| m.text(text_id)).cloned() else { return };
        self.canvas.gestures.open_text_editor(Some(t.id), t.x, t.y, t.text, t.font_size, t.font_family, host);
    }

    // Frame

    /// Draw the map and answer the pointer and the keys. `revision` is the
    /// caller's count of store changes: when it moves the drawing is made
    /// again (a tree of the same shape only where it differs).
    pub fn show(&mut self, ui: &mut egui::Ui, host: &mut dyn MapHost, revision: u64) -> MapViewOutput {
        let size = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        // The map is one widget, with an id of its own so it can hold the
        // keyboard (`aria-label="Metabolic map"` on the stage).
        let response = ui.interact(rect, self.id, Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Metabolic map"));
        let ctx = ui.ctx().clone();
        let now_ms = ui.input(|i| i.time) * 1e3;
        self.canvas.set_rect(StageRect::new(rect.min.x as f64, rect.min.y as f64, rect.width() as f64, rect.height() as f64));
        let colors = StageColors::for_dark(self.dark);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, colors.bg);

        if host.map().is_none() {
            // `No model to display.`
            ui.painter().text(
                rect.min + Vec2::new(30.0, 30.0),
                egui::Align2::LEFT_TOP,
                "No model to display.",
                egui::FontId::proportional(13.0),
                colors.ink3,
            );
            self.canvas.sync(host, revision, now_ms);
            return MapViewOutput { response, events: self.canvas.take_events() };
        }

        // A first paint needs a drawing to hit-test against.
        if self.canvas.scene.is_none() {
            self.canvas.sync(host, revision, now_ms);
        }
        self.handle_input(ui, rect, host, now_ms);
        self.canvas.sync(host, revision, now_ms);
        self.dark = self.canvas.scene.as_ref().is_some_and(|s| s.theme == crate::style::PageTheme::dark());

        // `:hover`, over the map only.
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| self.owns(&ctx, ui, rect, *p));
        self.canvas.hover_at(hover.map(|p| Point::new(p.x as f64, p.y as f64)));

        // The drawing.
        let cam = self.canvas.camera;
        let mut clip = Rect::NOTHING;
        if let Some(scene) = &mut self.canvas.scene {
            scene.ensure_lods(cam.scale, painter::view_in_map(&cam, rect.min, rect));
            let painter = ui.painter_at(rect);
            painter::paint(&painter, scene, &cam, rect.min, rect, &mut self.cache);
            clip = painter::canvas_clip(&painter, scene, &cam, rect.min, rect);
        }
        let colors = StageColors::for_dark(self.dark);
        // Sharp once still, under the overlays.
        let scene = self.canvas.scene.as_ref().filter(|_| self.crisp);
        self.settled.frame(ui, scene, &cam, rect, clip, self.canvas.gestures.active(), colors.bg);

        self.paint_overlays(ui, rect, host, &colors);
        self.empty_map_card(ui, rect, host, &colors);
        self.text_editor(ui, rect, host, &colors);
        if let Some(p) = hover {
            ctx.set_cursor_icon(self.cursor(p));
        } else if self.canvas.gestures.active() {
            ctx.set_cursor_icon(self.cursor(Pos2::ZERO));
        }
        if self.canvas.gestures.active() {
            ctx.request_repaint();
        }
        MapViewOutput { response, events: self.canvas.take_events() }
    }

    /// Whether a point is on the map and nothing of egui's is over it there.
    fn owns(&self, ctx: &egui::Context, ui: &egui::Ui, rect: Rect, p: Pos2) -> bool {
        rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == ui.layer_id()) && !self.editor_rect.is_some_and(|r| r.contains(p))
    }

    fn handle_input(&mut self, ui: &mut egui::Ui, rect: Rect, host: &mut dyn MapHost, now_ms: f64) {
        let ctx = ui.ctx().clone();
        let events = ui.input(|i| i.events.clone());
        let focused = ctx.memory(|m| m.focused());
        let keyboard_ours = !ctx.text_edit_focused() && focused.is_none_or(|f| f == self.id);
        // A touch screen sends touches and, for the first finger, a mouse
        // made of them: the touches are read, the mouse ignored.
        let touch_frame = events.iter().any(|e| matches!(e, egui::Event::Touch { .. })) || !self.touches.is_empty();
        let mut modifiers = ui.input(|i| i.modifiers);
        let mouse = |pos: Pos2, button: i16, shift: bool| PointerInput {
            pointer_id: 1,
            pointer_type: PointerType::Mouse,
            button,
            client: Point::new(pos.x as f64, pos.y as f64),
            shift,
            ..Default::default()
        };
        for event in events {
            match event {
                egui::Event::ModifiersChanged(m) => modifiers = m,
                egui::Event::PointerButton { pos, button, pressed, modifiers: m } if !touch_frame => {
                    let b = match button {
                        egui::PointerButton::Primary => 0,
                        egui::PointerButton::Middle => 1,
                        egui::PointerButton::Secondary => 2,
                        egui::PointerButton::Extra1 => 3,
                        egui::PointerButton::Extra2 => 4,
                    };
                    let ev = mouse(pos, b, m.shift);
                    if pressed {
                        if self.owns(&ctx, ui, rect, pos) {
                            ctx.memory_mut(|mem| mem.request_focus(self.id));
                            self.canvas.pointer_down(host, &ev);
                        }
                    } else if self.canvas.gestures.captured == Some(1) || self.owns(&ctx, ui, rect, pos) {
                        self.canvas.pointer_up(host, &ev, now_ms);
                    }
                }
                egui::Event::PointerMoved(pos) if !touch_frame => {
                    if self.canvas.gestures.active() || self.owns(&ctx, ui, rect, pos) {
                        let ev = PointerInput { button: -1, ..mouse(pos, -1, modifiers.shift) };
                        self.canvas.pointer_move(host, &ev, now_ms);
                    }
                }
                egui::Event::Touch { id, phase, pos, .. } => {
                    let pid = id.0.wrapping_add(2);
                    let ev = PointerInput {
                        pointer_id: pid,
                        pointer_type: PointerType::Touch,
                        button: 0,
                        client: Point::new(pos.x as f64, pos.y as f64),
                        ..Default::default()
                    };
                    match phase {
                        egui::TouchPhase::Start => {
                            if self.owns(&ctx, ui, rect, pos) {
                                self.touches.insert(pid);
                                self.canvas.pointer_down(host, &ev);
                            }
                        }
                        egui::TouchPhase::Move => {
                            if self.touches.contains(&pid) {
                                self.canvas.pointer_move(host, &PointerInput { button: -1, ..ev }, now_ms);
                            }
                        }
                        egui::TouchPhase::End => {
                            if self.touches.remove(&pid) {
                                self.canvas.pointer_up(host, &ev, now_ms);
                            }
                        }
                        egui::TouchPhase::Cancel => {
                            if self.touches.remove(&pid) {
                                self.canvas.pointer_cancel(host, &ev);
                            }
                        }
                    }
                }
                egui::Event::MouseWheel { unit, delta, modifiers: m, .. } => {
                    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { continue };
                    if !self.owns(&ctx, ui, rect, pos) {
                        continue;
                    }
                    // The DOM's deltas run the other way: positive scrolls down.
                    let input = WheelInput {
                        delta_x: -delta.x as f64,
                        delta_y: -delta.y as f64,
                        delta_mode: match unit {
                            egui::MouseWheelUnit::Point => DeltaMode::Pixel,
                            egui::MouseWheelUnit::Line => DeltaMode::Line,
                            egui::MouseWheelUnit::Page => DeltaMode::Page,
                        },
                        ctrl_key: m.ctrl || m.mac_cmd,
                        ..Default::default()
                    };
                    self.canvas.wheel(host, &input, Point::new(pos.x as f64, pos.y as f64), now_ms);
                }
                egui::Event::Zoom(factor) => {
                    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { continue };
                    if self.owns(&ctx, ui, rect, pos) {
                        self.canvas.zoom_gesture(factor as f64, Point::new(pos.x as f64, pos.y as f64), now_ms);
                    }
                }
                egui::Event::Key { key, pressed, modifiers: m, repeat, .. } => {
                    if key == egui::Key::Space && !repeat {
                        // Space is held for as long as the map has the keyboard.
                        self.canvas.gestures.set_space_held(pressed && keyboard_ours);
                    }
                    if !pressed || !keyboard_ours {
                        continue;
                    }
                    let hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
                    if !(hovered || focused == Some(self.id) || self.canvas.gestures.active()) {
                        continue;
                    }
                    let Some(k) = canvas_key(key) else { continue };
                    let mods = KeyMods { command: m.command, shift: m.shift };
                    self.canvas.key_down(host, k, mods);
                }
                egui::Event::WindowFocused(false) => {
                    // The window lost focus: the gesture ends (`pointercancel`).
                    self.canvas.gestures.set_space_held(false);
                    if self.canvas.gestures.active() {
                        let ev = mouse(Pos2::ZERO, 0, false);
                        self.canvas.pointer_cancel(host, &ev);
                    }
                }
                _ => {}
            }
        }
        if focused == Some(self.id) {
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.id,
                    egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true },
                )
            });
        }
    }

    // Overlays

    fn client_rect(a: Point, b: Point) -> Rect {
        Rect::from_two_pos(Pos2::new(a.x as f32, a.y as f32), Pos2::new(b.x as f32, b.y as f32))
    }

    fn paint_overlays(&mut self, ui: &mut egui::Ui, rect: Rect, host: &mut dyn MapHost, colors: &StageColors) {
        let painter = ui.painter_at(rect);
        let g = &self.canvas.gestures;
        // `compartmentParentOverlay()`: the parent a compartment may be drawn
        // in, the rest of the stage dimmed.
        if g.tool == Tool::Compartment
            && let Some(spec) = host.current_compartment_placement()
                && let Some(parent) = host.compartment_parent_rect(&spec.species_id, &spec.compartment_id) {
                    let box_ = self.map_rect_to_client(&parent);
                    let dim = mix(colors.bg, 0.12 * 0.7);
                    for r in outside(rect, box_) {
                        painter.rect_filled(r, CornerRadius::ZERO, dim);
                    }
                    dashed_rect(&painter, box_, Stroke::new(2.0, mix(colors.accent, 0.7)), 6.0, 4.0);
                }
        // `selectionOverlay()`.
        if let Some(s) = &g.selection {
            let r = Self::client_rect(s.start_client, s.current_client);
            painter.rect_filled(r, CornerRadius::same(3), mix(colors.accent, 0.12));
            dashed_rect(&painter, r, Stroke::new(1.5, colors.accent), 4.5, 3.0);
        }
        // `cellDrawOverlay()`.
        if let Some(d) = &g.cell_draw {
            let r = Self::client_rect(d.start_client, d.current_client);
            painter.rect(r, CornerRadius::same(14), mix(colors.accent, 0.09), Stroke::new(2.0, colors.accent), StrokeKind::Inside);
        }
        // `compartmentDrawOverlay()`.
        if let Some(d) = &g.compartment_draw {
            let r = Self::client_rect(d.start_client, d.current_client);
            painter.rect(r, CornerRadius::same(12), mix(colors.accent, 0.10), Stroke::new(2.0, colors.accent), StrokeKind::Inside);
        }
        // `.emc-table-hover`: an amber ring, pulsing, at least 26 map units
        // across and never under 18 px, around the reaction of the table row
        // under the pointer.
        let hovered = self.table_hover.as_deref().and_then(|rid| host.map().and_then(|m| m.reaction(rid))).map(|r| Point::new(r.x, r.y));
        if let Some(at) = hovered {
            let scale = self.canvas.camera.scale;
            let radius = (26.0 * scale).max(18.0) as f32;
            let center = self.canvas.map_to_screen(at);
            let time = ui.input(|i| i.time);
            let pulse = 0.625 + 0.275 * (std::f64::consts::TAU * time / 1.2).cos();
            let amber = Color32::from_rgb(0xf5, 0x9e, 0x0b);
            painter.circle_stroke(Pos2::new(center.x as f32, center.y as f32), radius, Stroke::new(4.0, mix(amber, pulse as f32)));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
        }
    }

    fn map_rect_to_client(&self, r: &MapRect) -> Rect {
        let a = self.canvas.map_to_screen(Point::new(r.x, r.y));
        let b = self.canvas.map_to_screen(Point::new(r.x + r.w, r.y + r.h));
        Self::client_rect(a, b)
    }

    /// `emptyMapOverlay()`: what to do with a blank canvas, where the user is
    /// looking, and that the model is untouched.
    fn empty_map_card(&mut self, ui: &mut egui::Ui, rect: Rect, host: &mut dyn MapHost, colors: &StageColors) {
        let Some(map) = host.map() else { return };
        let s = map.stats();
        if s.reactions > 0 || s.nodes > 0 {
            return;
        }
        let available = host.available_reactions();
        let shell = s.cells > 0;
        // Laying out a genome-scale model in one cell takes a minute or more:
        // say what it costs.
        let big_model = available > 600;
        let core_supported = host.core_overview_supported();
        let mut action = None;
        let card = Rect::from_center_size(rect.center(), Vec2::new(420f32.min(rect.width() - 48.0).max(200.0), 10.0));
        egui::Area::new(self.id.with("empty-map"))
            .order(egui::Order::Middle)
            .fixed_pos(Pos2::new(card.min.x, rect.center().y - 120.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(colors.panel)
                    .stroke(Stroke::new(1.0, colors.line))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(egui::Margin::symmetric(24, 22))
                    .show(ui, |ui| {
                        ui.set_max_width(card.width() - 48.0);
                        ui.label(egui::RichText::new("Nothing drawn yet").size(16.0).strong().color(colors.ink));
                        let p = match (shell, available > 0) {
                            (true, true) => format!("The cells are drawn but empty; {available} reactions can be added."),
                            (true, false) => "The cells are drawn but empty.".to_string(),
                            (false, true) => format!("The map is empty; the model has {available} reactions."),
                            (false, false) => "The map is empty.".to_string(),
                        };
                        ui.label(egui::RichText::new(p).size(13.0).color(colors.ink2));
                        let steps: &[&str] = if shell {
                            &[
                                "Click a compartment or a membrane to add the reactions that belong there.",
                                "Or use Add Reaction and Add Metabolite to place them one at a time.",
                            ]
                        } else {
                            &[
                                "Pick Cell and drag to draw a cell.",
                                "Pick Compartment and drag inside it.",
                                "Click a membrane or a compartment to add the reactions that belong on it.",
                            ]
                        };
                        for (k, step) in steps.iter().enumerate() {
                            ui.label(egui::RichText::new(format!("{}. {step}", k + 1)).size(13.0).color(colors.ink2));
                        }
                        ui.add_space(8.0);
                        let wide = |ui: &mut egui::Ui, text: String, primary: bool| {
                            let mut b = egui::Button::new(egui::RichText::new(text).color(if primary { colors.accent_ink } else { colors.ink }));
                            if primary {
                                b = b.fill(colors.accent);
                            }
                            ui.add_sized([ui.available_width(), 28.0], b).clicked()
                        };
                        if big_model && core_supported {
                            if wide(ui, "Draw central metabolism".into(), true) {
                                action = Some("map-core-overview");
                            }
                            if wide(ui, format!("Draw all {available} reactions (slow)"), false) {
                                action = Some("map-full");
                            }
                        } else if wide(ui, "Or draw the full map".into(), true) {
                            action = Some("map-full");
                        }
                    });
            });
        if let Some(a) = action {
            self.canvas.gestures_event(CanvasEvent::Action(a));
        }
    }

    /// `renderTextEditor()`: the text field, its size and font, and Delete
    /// for an existing text, just above where the text sits.
    fn text_editor(&mut self, ui: &mut egui::Ui, rect: Rect, host: &mut dyn MapHost, colors: &StageColors) {
        let Some(editor) = self.canvas.gestures.text_editor.clone() else {
            self.editor_rect = None;
            return;
        };
        let cam = self.canvas.camera;
        let annotation_size = editor.id.as_ref().and_then(|id| host.map().and_then(|m| m.text(id)).and_then(|t| t.font_size));
        let font_size = annotation_size.unwrap_or(editor.font_size);
        let left = cam.x + editor.x * cam.scale;
        let top = cam.y + editor.y * cam.scale - (font_size * cam.scale * 1.25).max(20.0);
        let pos = Pos2::new(rect.min.x + left as f32, rect.min.y + top as f32 - 2.0);
        let field_id = self.id.with("text-editor-field");
        let mut next = editor.clone();
        let mut commit = false;
        let mut cancel = false;
        let mut delete = false;
        let area = egui::Area::new(self.id.with("text-editor")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(colors.panel)
                .stroke(Stroke::new(1.0, colors.accent))
                .corner_radius(CornerRadius::same(9))
                .inner_margin(egui::Margin::same(5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let family = if next.font_family == "mono" { egui::FontFamily::Monospace } else { egui::FontFamily::Proportional };
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut next.value)
                                .id(field_id)
                                .hint_text("Type text…")
                                .font(egui::FontId::new(15.0, family))
                                .desired_width(260.0),
                        );
                        if !editor.shown {
                            // Opened just now: ready to type over.
                            field.request_focus();
                            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), field_id) {
                                let n = next.value.chars().count();
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(n),
                                )));
                                state.store(ui.ctx(), field_id);
                            }
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            commit = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        }
                        let mut size = next.font_size;
                        let drag = ui.add(
                            egui::DragValue::new(&mut size).range(TEXT_SIZE.min..=TEXT_SIZE.max).speed(1.0).suffix(" px"),
                        );
                        if drag.changed() {
                            next.font_size = clamp_text_size(size);
                        }
                        drag.on_hover_text("Text size (px)");
                        let current = TEXT_FONTS.iter().find(|f| f.id == next.font_family).map_or("Sans serif", |f| f.label);
                        egui::ComboBox::from_id_salt(self.id.with("text-editor-font")).selected_text(current).width(128.0).show_ui(
                            ui,
                            |ui| {
                                for f in TEXT_FONTS.iter() {
                                    ui.selectable_value(&mut next.font_family, f.id.to_string(), f.label);
                                }
                            },
                        );
                        if next.id.is_some() {
                            let del = ui.add(egui::Button::new(egui::RichText::new("×").size(20.0).color(colors.danger)).min_size(Vec2::splat(32.0)));
                            if del.on_hover_text("Delete text").clicked() {
                                delete = true;
                            }
                        }
                    });
                });
        });
        self.editor_rect = Some(area.response.rect);
        next.shown = true;
        if let Some(e) = &mut self.canvas.gestures.text_editor {
            *e = next;
        }
        if delete {
            self.canvas.gestures.delete_edited_text(host);
        } else if commit {
            self.canvas.gestures.commit_text_editor(host);
        } else if cancel {
            self.canvas.gestures.cancel_text_editor();
        }
    }

    // Cursor

    /// The cursor `assets/map.css` and `assets/app.css` give the stage and
    /// what is under the pointer.
    fn cursor(&self, p: Pos2) -> CursorIcon {
        let g = &self.canvas.gestures;
        if g.pan.is_some() || g.node_drag.is_some() || g.group_drag.is_some() || g.metabolite_label_drag.is_some() || g.reaction_label_drag.is_some() {
            return CursorIcon::Grabbing;
        }
        if g.compartment_label_drag.is_some() || g.text_drag.is_some() || g.control_drag.is_some() {
            return CursorIcon::Grabbing;
        }
        if g.cell_draw.is_some() || g.compartment_draw.is_some() {
            return CursorIcon::Crosshair;
        }
        if let Some(r) = &g.canvas_resize {
            return edge_cursor(&r.edge);
        }
        if g.tool == Tool::Pan || g.space_held {
            return CursorIcon::Grab;
        }
        let Some(scene) = &self.canvas.scene else { return CursorIcon::Default };
        let at = self.canvas.screen_to_map(Point::new(p.x as f64, p.y as f64));
        let base = if matches!(g.tool, Tool::Cell | Tool::Compartment) { CursorIcon::Crosshair } else { CursorIcon::Default };
        let Some(i) = crate::hit::hit_test(scene, at, g.hit_filter()) else { return base };
        for a in scene.doc.ancestors(i) {
            let e = scene.doc.element(a);
            if let Some(c) = element_cursor(e, g.tool) {
                return c;
            }
        }
        base
    }
}

impl MapCanvas {
    /// Queue a request from the widget's own overlays.
    pub(crate) fn gestures_event(&mut self, e: CanvasEvent) {
        self.gestures.push_event(e);
    }
}

/// The cursor an element asks for (the `cursor` rules of the stylesheets).
fn element_cursor(e: &coxeter_render::Element, tool: Tool) -> Option<CursorIcon> {
    let has = |c: &str| e.has_class(c);
    if matches!(tool, Tool::Cell) && (has("emc-geometry-handle") || has("emc-cell-boundary") || has("emc-cell-plate")) {
        return Some(CursorIcon::PointingHand);
    }
    if matches!(tool, Tool::Compartment) && (has("emc-geometry-handle") || has("emc-boundary-hit") || has("emc-compartment-space")) {
        return Some(CursorIcon::PointingHand);
    }
    if tool == Tool::Reaction && has("emc-node") && e.get("data-node").is_some() {
        return Some(CursorIcon::Crosshair);
    }
    if has("emc-canvas-resize-right") || has("emc-canvas-resize-left") {
        return Some(CursorIcon::ResizeHorizontal);
    }
    if has("emc-canvas-resize-bottom") || has("emc-canvas-resize-top") {
        return Some(CursorIcon::ResizeVertical);
    }
    if has("emc-canvas-resize-corner") {
        return Some(CursorIcon::ResizeNwSe);
    }
    if has("emc-geometry-move") {
        return Some(CursorIcon::Move);
    }
    if has("emc-geometry-nw") || has("emc-geometry-se") {
        return Some(CursorIcon::ResizeNwSe);
    }
    if has("emc-geometry-ne") || has("emc-geometry-sw") {
        return Some(CursorIcon::ResizeNeSw);
    }
    // Side handles move one side; each shape diamond runs along one edge: the
    // corner one along the top, the oval one down the side.
    if has("emc-geometry-n") || has("emc-geometry-s") || has("emc-geometry-oval") {
        return Some(CursorIcon::ResizeVertical);
    }
    if has("emc-geometry-e") || has("emc-geometry-w") || has("emc-geometry-corner") {
        return Some(CursorIcon::ResizeHorizontal);
    }
    if has("emc-rotate-handle") {
        return Some(CursorIcon::Alias);
    }
    if has("emc-node-medium")
        || has("emc-comp-label")
        || has("emc-control-hit")
        || has("emc-control-b1")
        || has("emc-control-b2")
        || has("emc-control-anchor")
        || has("emc-multi-drag-hit")
        || has("emc-map-text")
        || has("emc-hit-node")
        || has("emc-label-draggable")
        || has("emc-label-rxn")
    {
        return Some(CursorIcon::Grab);
    }
    if has("emc-marker")
        || has("emc-node")
        || has("emc-crossfeed-route")
        || has("emc-cell-plate")
        || has("emc-boundary-hit")
        || has("emc-geometry-handle")
        || has("emc-hit-reaction")
        || has("emc-cell-boundary")
    {
        return Some(CursorIcon::PointingHand);
    }
    None
}

fn edge_cursor(edge: &str) -> CursorIcon {
    match edge {
        "right" | "left" => CursorIcon::ResizeHorizontal,
        "top" | "bottom" => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNwSe,
    }
}

/// The key of a canvas shortcut.
fn canvas_key(key: egui::Key) -> Option<CanvasKey> {
    Some(match key {
        egui::Key::Escape => CanvasKey::Escape,
        egui::Key::Delete => CanvasKey::Delete,
        egui::Key::Backspace => CanvasKey::Backspace,
        egui::Key::Num0 => CanvasKey::Zero,
        egui::Key::Plus | egui::Key::Equals => CanvasKey::Plus,
        egui::Key::Minus => CanvasKey::Minus,
        egui::Key::Z => CanvasKey::Z,
        egui::Key::Y => CanvasKey::Y,
        egui::Key::ArrowLeft => CanvasKey::ArrowLeft,
        egui::Key::ArrowRight => CanvasKey::ArrowRight,
        egui::Key::ArrowUp => CanvasKey::ArrowUp,
        egui::Key::ArrowDown => CanvasKey::ArrowDown,
        _ => return None,
    })
}

/// The parts of `outer` outside `inner`.
fn outside(outer: Rect, inner: Rect) -> Vec<Rect> {
    let inner = inner.intersect(outer);
    if !inner.is_positive() {
        return vec![outer];
    }
    vec![
        Rect::from_min_max(outer.min, Pos2::new(outer.max.x, inner.min.y)),
        Rect::from_min_max(Pos2::new(outer.min.x, inner.max.y), outer.max),
        Rect::from_min_max(Pos2::new(outer.min.x, inner.min.y), Pos2::new(inner.min.x, inner.max.y)),
        Rect::from_min_max(Pos2::new(inner.max.x, inner.min.y), Pos2::new(outer.max.x, inner.max.y)),
    ]
}

/// A dashed rectangle (CSS `border: … dashed`).
fn dashed_rect(painter: &egui::Painter, r: Rect, stroke: Stroke, dash: f32, gap: f32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        painter.extend(Shape::dashed_line(&[w[0], w[1]], stroke, dash, gap));
    }
}
