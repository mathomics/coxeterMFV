//! The application: `eframe::App` around the [`Shell`], which holds the
//! Store, the map widget and the interface's own state, and draws the whole
//! window each frame. Ports the wiring of `src/ui/app.js`: the boot, the
//! Store subscription (`store.subscribe((_, reason) => …)`, here
//! `take_events()` once per frame), the window `keydown` handler, the
//! debounced and coalesced solves (`debounce`, `scheduleSolve`), the status
//! line's notice clock (`paintStatus`) and the theme (`setTheme`).
//!
//! The cross-cutting actions (open a workspace, pick a tool, export…) are in
//! `actions.rs`; what each panel draws and does is in `panels/`.

use std::path::PathBuf;

use coxeter_canvas::gestures::{CanvasKey, KeyMods};
use coxeter_canvas::{CanvasEvent, MapView};
use coxeter_state::{JobKind, Status, StatusState, Store};
use indexmap::IndexMap;

use crate::files::FileDialogs;
use crate::host::StoreHost;
use crate::panels;
use crate::theme;
use crate::ui_state::UiState;

/// A store call that waits until the control has been still for a moment
/// (`debounce(key, fn, ms)`).
#[derive(Debug, Clone, PartialEq)]
pub enum Deferred {
    /// `store.setAbundance(species, value)`.
    Abundance { species: String, value: String },
    /// `store.setMediumLimit(base, value)`.
    Medium { base: String, value: String },
    /// `store.setPFBABiomassPercent(value)`.
    PfbaPercent(String),
    /// `paintSaveState()` (the badge is brought up to date once changes pause).
    SaveState,
}

/// The status line's notice clock: when the current status was first shown.
#[derive(Debug, Clone, Default)]
pub struct StatusNotice {
    pub status: Option<Status>,
    pub since: Option<std::time::Instant>,
}

/// The whole interface and everything it acts on.
pub struct Shell {
    pub store: Store,
    pub ui: UiState,
    /// The map (`#stage`): camera, drawing, gestures.
    pub map: MapView,
    /// Where files are picked and saved.
    pub dialogs: Box<dyn FileDialogs>,
    /// The map's context menus and the Add Reaction panel.
    pub menus: panels::menus::MapMenus,
    /// Counts store changes that touch the drawing: the map widget redraws
    /// when it moves.
    pub revision: u64,
    /// Debounced store calls, by key, with the time (egui seconds) they run.
    pub deferred: IndexMap<&'static str, (f64, Deferred)>,
    /// `scheduleSolve()`: one solve on the next frame.
    pub pending_solve: bool,
    pub status_notice: StatusNotice,
    /// The reasons the store emitted before this frame (what changed).
    pub reasons: Vec<String>,
    /// `paintSaveState()` as last worked out.
    pub save_state: coxeter_state::UnsavedWork,
    /// When the results bar last flashed a finished run (`runFlash`).
    pub run_flash: Option<(f64, std::time::Instant)>,
    /// The map's box and widget id last frame (keys over the map are the canvas's).
    pub map_rect: Option<egui::Rect>,
    pub map_id: Option<egui::Id>,
    /// Where the theme choice is kept (`localStorage['coxeter-theme']`); none in tests.
    pub settings_path: Option<PathBuf>,
    /// Messages for a terminal (`--screenshot` prints them).
    pub boot_error: Option<String>,
    applied_dark: Option<bool>,
    fonts_installed: bool,
    focus_request_age: u32,
    now: f64,
}

impl Shell {
    /// A shell over a store that has been `init()`-ed.
    pub fn new(store: Store, dialogs: Box<dyn FileDialogs>) -> Shell {
        Shell {
            store,
            ui: UiState::default(),
            map: MapView::new(),
            dialogs,
            menus: panels::menus::MapMenus::default(),
            revision: 1,
            deferred: IndexMap::new(),
            pending_solve: false,
            status_notice: StatusNotice::default(),
            reasons: Vec::new(),
            save_state: Default::default(),
            run_flash: None,
            map_rect: None,
            map_id: None,
            settings_path: None,
            boot_error: None,
            applied_dark: None,
            fonts_installed: false,
            focus_request_age: 0,
            now: 0.0,
        }
    }

    /// `boot()`: read the catalogue and open the first scenario (or `scenario`).
    pub fn boot(&mut self, scenario: Option<&str>) {
        let result = self.store.init().map(|_| ()).and_then(|_| {
            let first = scenario
                .map(str::to_string)
                .or_else(|| self.store.scenarios.first().and_then(|s| s.get("id")).and_then(|v| v.as_str()).map(str::to_string));
            match first {
                Some(id) => self.store.use_scenario(&id),
                None => Ok(()),
            }
        });
        if let Err(error) = result {
            self.boot_error = Some(error.to_string());
            self.store.set_status_message(StatusState::Error, error.to_string());
            self.store.emit("status");
        }
        self.sync_inspector_visibility();
    }

    /// The palette of the theme in force.
    pub fn palette(&self) -> theme::Palette {
        theme::Palette::of(self.store.view.dark)
    }

    /// The store as the map widget sees it.
    pub fn host(&mut self) -> StoreHost<'_> {
        StoreHost::new(&mut self.store)
    }

    /// egui's clock (seconds), as of this frame.
    pub fn now(&self) -> f64 {
        self.now
    }

    // Frame

    /// One frame of the whole window.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.now = ctx.input(|i| i.time);
        let keyboard_in_field = ctx.text_edit_focused();
        if !self.fonts_installed {
            // Fonts given to egui take effect on the next pass, and the
            // interface asks for the bold family: draw from the next frame.
            crate::fonts::install(&ctx);
            self.fonts_installed = true;
            ctx.request_repaint();
            return;
        }
        self.setup(&ctx);
        self.before_frame(&ctx);

        panels::topbar::show(ui, self);
        panels::left::show(ui, self);
        panels::right::show(ui, self);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            panels::toolbar::show(ui, self);
            self.canvas_column(ui);
        });
        panels::menus::show(&ctx, self);

        self.keys(&ctx, keyboard_in_field);
        // Files dropped on the window open as what they are (as the command line).
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            self.open_paths(&dropped);
        }
        self.after_frame(&ctx);
    }

    /// The map, the workspace page over it, and the results dock.
    fn canvas_column(&mut self, ui: &mut egui::Ui) {
        if let Some(ws) = self.ui.workspace {
            panels::workspace::show(ui, self, ws);
            return;
        }
        if self.ui.results_maximized {
            panels::results::show(ui, self, true);
            return;
        }
        panels::results::show(ui, self, false);
        egui::CentralPanel::no_frame().show(ui, |ui| self.map_area(ui));
    }

    /// `#stage`: the map widget, and what it asks of the panels.
    pub fn map_area(&mut self, ui: &mut egui::Ui) {
        let revision = self.revision;
        let mut host = StoreHost::new(&mut self.store);
        let out = self.map.show(ui, &mut host, revision);
        self.map_rect = Some(out.response.rect);
        self.map_id = Some(out.response.id);
        for event in out.events {
            self.on_canvas_event(event);
        }
    }

    /// What the stage asks of the rest of the application.
    pub fn on_canvas_event(&mut self, event: CanvasEvent) {
        match event {
            CanvasEvent::OpenMenu { menu, at } => self.menus.open(&self.store, menu, at),
            CanvasEvent::CloseMenu(menu) => self.menus.close(menu),
            CanvasEvent::StatusHint(message) => {
                self.store.set_status_message(StatusState::Ok, message);
                self.store.emit("status");
            }
            CanvasEvent::FocusLowerBound => {
                self.ui.focus_lower_bound = true;
                self.reveal_inspector();
            }
            CanvasEvent::ToolChanged(_) => {}
            CanvasEvent::Action(action) => match action {
                "map-full" => self.map_full(),
                "map-core-overview" => self.map_core_overview(),
                _ => {}
            },
            CanvasEvent::TextEditorOpened { .. } | CanvasEvent::TextEditorClosed => {}
        }
    }

    /// Fonts once, and the theme whenever it changes.
    fn setup(&mut self, ctx: &egui::Context) {
        let dark = self.store.view.dark;
        if self.applied_dark != Some(dark) {
            theme::apply(ctx, dark);
            self.applied_dark = Some(dark);
        }
    }

    /// The store's side of a frame: background work, what changed, the
    /// debounced calls that are due, the coalesced solve.
    fn before_frame(&mut self, ctx: &egui::Context) {
        for outcome in self.store.poll_jobs() {
            self.on_job_finished(outcome);
        }
        if self.pending_solve {
            self.pending_solve = false;
            self.store.solve();
        }
        let due: Vec<&'static str> = self.deferred.iter().filter(|(_, (at, _))| *at <= self.now).map(|(k, _)| *k).collect();
        for key in due {
            if let Some((_, call)) = self.deferred.shift_remove(key) {
                self.run_deferred(call);
            }
        }
        self.take_reasons();
        if let Some(when) = self.deferred.values().map(|(at, _)| *at).reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((when - self.now).max(0.0)));
        }
    }

    /// `store.subscribe(…)`: read what the store announced and decide what
    /// to redraw. Here everything is drawn each frame; only the map's
    /// drawing is cached, so a reason other than `status` moves its revision.
    pub fn take_reasons(&mut self) {
        let reasons = self.store.take_events();
        if reasons.is_empty() {
            return;
        }
        if reasons.iter().any(|r| r != "status" && r != "variability") {
            self.revision += 1;
        }
        // The badge compares the work with what was saved, which serialises
        // the scenario; it is brought up to date once the changes pause.
        self.debounce("save-state", 0.15, Deferred::SaveState);
        // `paintInspector()` (every reason but `status`) ends in
        // `syncInspectorVisibility()`.
        if reasons.iter().any(|r| r != "status") {
            self.sync_inspector_visibility();
        }
        // Importing another map or scenario replaces the palette.
        if self.ui.flux_color_source.as_ref() != Some(&self.store.view.flux_color_stops) {
            self.ui.flux_color_source = Some(self.store.view.flux_color_stops.clone());
            if !self.ui.flux_colors_committing {
                self.ui.flux_color_draft = None;
            }
        }
        self.reasons = reasons;
    }

    /// `debounce(key, fn, ms)`: run `call` once `key` has been quiet for `seconds`.
    pub fn debounce(&mut self, key: &'static str, seconds: f64, call: Deferred) {
        self.deferred.insert(key, (self.now + seconds, call));
    }

    /// Run every debounced call now (tests; the end of a gesture).
    pub fn flush_deferred(&mut self) {
        for (_, (_, call)) in std::mem::take(&mut self.deferred) {
            self.run_deferred(call);
        }
        if self.pending_solve {
            self.pending_solve = false;
            self.store.solve();
        }
        self.take_reasons();
    }

    fn run_deferred(&mut self, call: Deferred) {
        match call {
            Deferred::Abundance { species, value } => {
                let _ = self.store.set_abundance(&species, value.as_str());
            }
            Deferred::Medium { base, value } => {
                let _ = self.store.set_medium_limit(&base, value.as_str());
            }
            Deferred::PfbaPercent(value) => {
                let _ = self.store.set_pfba_biomass_percent(value.as_str());
            }
            Deferred::SaveState => self.save_state = self.store.unsaved_work(),
        }
    }

    /// `scheduleSolve()`: coalesce slider drags into one solve per frame.
    pub fn schedule_solve(&mut self) {
        self.pending_solve = true;
    }

    /// A background job finished (the `await` of a JS call returned).
    fn on_job_finished(&mut self, outcome: coxeter_state::JobOutcome) {
        if outcome.kind == JobKind::SteadyCom {
            self.ui.steady_com_pending = false;
        }
        let Err(error) = outcome.result else { return };
        let message = error.to_string();
        let cancelled = error.is_abort() || message.to_lowercase().contains("cancel") || message.to_lowercase().contains("superseded");
        if cancelled {
            return;
        }
        match outcome.kind {
            JobKind::SteadyCom => {
                // `runSteadyCom`'s catch: under the controls, and in the header.
                panels::scenario::show_steady_com_error(self, &message, None);
                self.status_error(format!("SteadyCom: {message}"));
            }
            JobKind::Solve => self.status_error(message),
            _ => {
                if self.store.status().state != StatusState::Error {
                    self.status_error(message);
                }
            }
        }
    }

    /// `store.status = { ...store.status, state: 'error', message }; store.emit('status')`.
    pub fn status_error(&mut self, message: impl Into<String>) {
        self.store.set_status_message(StatusState::Error, message);
        self.store.emit("status");
    }

    /// The same with `ok`.
    pub fn status_ok(&mut self, message: impl Into<String>) {
        self.store.set_status_message(StatusState::Ok, message);
        self.store.emit("status");
    }

    /// The same with `warn`.
    pub fn status_warn(&mut self, message: impl Into<String>) {
        self.store.set_status_message(StatusState::Warn, message);
        self.store.emit("status");
    }

    /// `store.addLog('ok', message); store.emit('status')`.
    pub fn log_ok(&mut self, message: &str) {
        self.store.add_log(StatusState::Ok, message, coxeter_state::LogKind::Message);
        self.store.emit("status");
    }

    /// `statusLine(store, Date.now() - statusNotice.since)`.
    pub fn status_line(&mut self, ctx: &egui::Context) -> coxeter_state::status_line::StatusLine {
        let status = self.store.status().clone();
        if self.status_notice.status.as_ref() != Some(&status) {
            self.status_notice.status = Some(status);
            self.status_notice.since = Some(std::time::Instant::now());
        }
        let age = self.status_notice.since.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1e3);
        let line = coxeter_state::status_line::status_line(&self.store, age);
        if line.expires_in > 0.0 {
            // A notice gives way to the summary on its own.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(line.expires_in / 1e3 + 0.02));
        }
        line
    }

    /// `syncInspectorVisibility()`: give the inspector's column back to the
    /// map when there is no selection, unless the user has set it by hand.
    pub fn sync_inspector_visibility(&mut self) {
        if self.ui.right_tab == crate::ui_state::RightTab::Layout {
            self.ui.inspector_collapsed = false;
            return;
        }
        if self.ui.inspector_pinned {
            return;
        }
        let has = self.store.selected_reaction.is_some() || self.store.selected_entity.is_some();
        self.ui.inspector_collapsed = !has;
    }

    /// Open the inspector (a double click on a reaction asks for its bounds).
    pub fn reveal_inspector(&mut self) {
        self.ui.right_tab = crate::ui_state::RightTab::Inspector;
        self.ui.inspector_collapsed = false;
        self.ui.focus_mode = false;
    }

    // Keys

    /// The window's `keydown` (app.js): Escape closes what is open, Ctrl+F
    /// finds a reaction, and the map's keys (Ctrl+Z / Ctrl+Y, Delete,
    /// Escape, 0 + −, arrows) reach the map when the map itself did not
    /// already take them (the widget answers keys while the pointer is on it
    /// or it was clicked last).
    fn keys(&mut self, ctx: &egui::Context, keyboard_in_field: bool) {
        let events = ctx.input(|i| i.events.clone());
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let focused = ctx.memory(|m| m.focused());
        let canvas_has_keys = self.ui.workspace.is_none()
            && !self.ui.results_maximized
            && (hover.is_some_and(|p| self.map_rect.is_some_and(|r| r.contains(p)))
                || (focused.is_some() && focused == self.map_id)
                || self.map.canvas.gestures.active());
        for event in events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
            if key == egui::Key::Escape {
                if self.ui.workspace.is_some() {
                    self.close_workspace();
                    continue;
                }
                if self.ui.task_overlay {
                    self.ui.task_overlay = false;
                    continue;
                }
                if egui::Popup::is_any_open(ctx) {
                    continue;
                }
                if self.ui.results_maximized {
                    self.ui.results_maximized = false;
                    continue;
                }
                if self.menus.is_open() {
                    self.menus.close_all();
                    continue;
                }
            }
            if keyboard_in_field || ctx.text_edit_focused() {
                continue;
            }
            if modifiers.command && key == egui::Key::F {
                self.open_workspace_named("library");
                self.ui.focus_request = Some(panels::map_options::reaction_search_id());
                continue;
            }
            if canvas_has_keys {
                continue;
            }
            let Some(k) = canvas_key(key) else { continue };
            let mods = KeyMods { command: modifiers.command, shift: modifiers.shift };
            if mods.command && matches!(k, CanvasKey::Z | CanvasKey::Y) {
                self.menus.close_all();
            }
            let mut host = StoreHost::new(&mut self.store);
            self.map.canvas.key_down(&mut host, k, mods);
            for e in self.map.canvas.take_events() {
                self.on_canvas_event(e);
            }
        }
    }

    /// The end of a frame: what changed while drawing reaches the next one.
    fn after_frame(&mut self, ctx: &egui::Context) {
        if !self.store.pending_events().is_empty() {
            self.take_reasons();
            ctx.request_repaint();
        }
        if self.store.has_running_jobs() {
            // Background work reports through `poll_jobs`, not an event.
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
        // A focus request is taken by the control that has its id; one no
        // control took within a few frames is dropped.
        if self.ui.focus_request.is_some() {
            self.focus_request_age += 1;
            if self.focus_request_age > 3 {
                self.ui.focus_request = None;
            }
        } else {
            self.focus_request_age = 0;
        }
    }
}

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

/// The window: eframe drives the [`Shell`].
impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}

impl Shell {
    /// Give the keyboard to `response` if it is the control a focus request
    /// names (Ctrl+F's reaction search, a double click's lower bound).
    pub fn take_focus_request(&mut self, response: &egui::Response) {
        if self.ui.focus_request == Some(response.id) {
            response.request_focus();
            self.ui.focus_request = None;
        }
    }
}
