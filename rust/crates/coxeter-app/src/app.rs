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
use std::sync::Arc;

use coxeter_canvas::gestures::{CanvasKey, KeyMods};
use coxeter_canvas::{CanvasEvent, MapView};
use coxeter_state::reactions::{ReactionTablePatch, ReactionTableModel};
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
    /// `store.setReactionTable({ query })`: the Reactions page's search.
    ReactionSearch(String),
    /// `store.setReactionTable({ geneQuery })`: the Genes control's search.
    GeneSearch(String),
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
    /// Counts store changes that can change a reactions table row: the
    /// table model is rebuilt once per revision, not once per frame.
    pub table_revision: u64,
    table_model: Option<(u64, Arc<ReactionTableModel>)>,
    /// Debounced store calls, by key, with the time (egui seconds) they run.
    pub deferred: IndexMap<String, (f64, Deferred)>,
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
    /// Where the theme choice is kept (`localStorage['coxeter-theme']`), and
    /// the solver's; none in tests.
    pub settings_path: Option<PathBuf>,
    /// The solvers of this computer (the Solver block of Simulation settings).
    pub solvers: crate::solvers::Solvers,
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
            table_revision: 1,
            table_model: None,
            deferred: IndexMap::new(),
            pending_solve: false,
            status_notice: StatusNotice::default(),
            reasons: Vec::new(),
            save_state: Default::default(),
            run_flash: None,
            map_rect: None,
            map_id: None,
            settings_path: None,
            solvers: crate::solvers::Solvers::found(IndexMap::new()),
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
        // Set again by a reactions table whose row is under the pointer.
        self.ui.rx.hover = None;
        // Escape that closes a menu, a select's list or the View popover does
        // only that, as app.js returns once it has closed `.canvas-view` /
        // `.top-menu`: the menus see it while they are drawn, and it is taken
        // away before the map and `keys` would clear the selection with it.
        let escape_closes_menu = (egui::Popup::is_any_open(&ctx) || self.ui.view_open) && ctx.input(|i| i.key_pressed(egui::Key::Escape));

        panels::topbar::show(ui, self);
        panels::left::show(ui, self);
        panels::right::show(ui, self);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            panels::toolbar::show(ui, self);
            if escape_closes_menu {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            }
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
        if self.reactions_split() {
            // The Reactions page leaves the top of the map in view, so
            // hovering a row shows the reaction and clicking one on the map
            // finds its row.
            let height = (ui.available_height() * 2.0 / 3.0).max(260.0);
            let frame = egui::Frame::new().fill(self.palette().bg);
            egui::Panel::bottom("reactions-page").exact_size(height).resizable(false).frame(frame).show(ui, |ui| panels::reactions::page(ui, self));
            egui::CentralPanel::no_frame().show(ui, |ui| self.map_area(ui));
            return;
        }
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
        self.map.table_hover = self.ui.rx.hover.clone();
        let mut host = StoreHost::new(&mut self.store);
        let out = self.map.show(ui, &mut host, revision);
        self.map_rect = Some(out.response.rect);
        self.map_id = Some(out.response.id);
        for event in out.events {
            self.on_canvas_event(event);
        }
        // Centred once the map has the box it is drawn in.
        if let Some(rid) = self.ui.rx.center_on.take() {
            self.center_on_reaction(&rid);
            ui.ctx().request_repaint();
        }
    }

    /// Whether the Reactions page is open with the map above it.
    pub fn reactions_split(&self) -> bool {
        self.ui.workspace == Some(crate::ui_state::Workspace::Reactions) && self.ui.rx.split && self.store.map.is_some()
    }

    /// `reactionTableModel()` of app.js: the reactions table for the store as
    /// it is now, rebuilt once per store change, shared by the page and the dock.
    pub fn reaction_table_model(&mut self) -> Arc<ReactionTableModel> {
        if let Some((revision, model)) = &self.table_model {
            if *revision == self.table_revision {
                return model.clone();
            }
        }
        let model = Arc::new(self.store.reaction_table_model());
        self.table_model = Some((self.table_revision, model.clone()));
        model
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
        if self.solvers.poll() {
            self.on_solvers_checked();
        }
        if self.solvers.checking() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.pending_solve {
            self.pending_solve = false;
            self.store.solve();
        }
        let due: Vec<String> = self.deferred.iter().filter(|(_, (at, _))| *at <= self.now).map(|(key, _)| key.clone()).collect();
        for key in due {
            if let Some((_, call)) = self.deferred.shift_remove(&key) {
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
        // How the reactions table lists rows changes nothing on the map.
        if reasons.iter().any(|r| r != "status" && r != "variability" && r != "reaction-table") {
            self.revision += 1;
        }
        if reasons.iter().any(|r| r != "status" && r != "variability" && r != "selection") {
            self.table_revision += 1;
        }
        // A reaction picked on the map is found in the table.
        let table_shown = self.ui.workspace == Some(crate::ui_state::Workspace::Reactions)
            || (self.ui.results_tab == crate::ui_state::ResultsTab::Table && !self.ui.results_collapsed);
        if table_shown && reasons.iter().any(|r| r == "selection") {
            self.ui.rx.reveal = self.store.selected_reaction.clone();
        }
        // The badge compares the work with what was saved, which serialises
        // the scenario; it is brought up to date once the changes pause.
        self.debounce("save-state", 0.15, Deferred::SaveState);
        // `paintInspector()` (every reason but `status`) ends in
        // `syncInspectorVisibility()`.
        if reasons.iter().any(|r| r != "status") {
            self.sync_inspector_visibility();
        }
        // Importing another map or scenario replaces the palette, and
        // switching between signed and absolute colours shows the other one.
        let palette = crate::panels::map_options::active_flux_palette(&self.store);
        if self.ui.flux_color_source.as_ref() != Some(&palette) {
            self.ui.flux_color_source = Some(palette);
            if !self.ui.flux_colors_committing {
                self.ui.flux_color_draft = None;
            }
        }
        self.reasons = reasons;
    }

    /// `debounce(key, fn, ms)`: run `call` once `key` has been quiet for `seconds`.
    pub fn debounce(&mut self, key: &'static str, seconds: f64, call: Deferred) {
        let key = match &call {
            Deferred::Abundance { species, .. } => format!("{key}:{species}"),
            Deferred::Medium { base, .. } => format!("{key}:{base}"),
            _ => key.to_string(),
        };
        self.deferred.insert(key, (self.now + seconds, call));
    }

    /// Commit pending inputs before an action uses or replaces the model.
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
            Deferred::ReactionSearch(query) => {
                if self.store.reaction_table.query != query {
                    self.store.set_reaction_table(ReactionTablePatch { query: Some(query), ..Default::default() });
                }
            }
            Deferred::GeneSearch(query) => {
                if self.store.reaction_table.gene_query != query {
                    self.store.set_reaction_table(ReactionTablePatch { gene_query: Some(query), ..Default::default() });
                }
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
        if outcome.kind == JobKind::Download && outcome.result.is_ok() {
            self.deferred.retain(|_, (_, call)| {
                !matches!(call, Deferred::Abundance { .. } | Deferred::Medium { .. } | Deferred::PfbaPercent(_))
            });
            self.pending_solve = false;
        }
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
        let has = self.store.selected_reaction.is_some()
            || self.store.selected_entity.is_some()
            || crate::panels::inspector::shows_selection_inspector(&self.store);
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
        let canvas_has_keys = (self.ui.workspace.is_none() || self.reactions_split())
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
        if self.store.has_pending_job_results() {
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

#[cfg(test)]
mod deferred_input_tests {
    use super::*;
    use crate::files::ScriptedDialogs;

    fn make_shell() -> Shell {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        Shell::new(store, Box::new(ScriptedDialogs::new()))
    }

    #[test]
    fn rapid_medium_changes_preserve_both_component_limits_and_coalesce_repeated_edits() {
        let mut shell = make_shell();
        shell.store.use_scenario("solo-ecoli").expect("scenario");
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "0".into() });
        shell.debounce("medium", 0.12, Deferred::Medium { base: "o2".into(), value: "0".into() });
        assert_eq!(shell.deferred.values().filter(|(_, call)| matches!(call, Deferred::Medium { .. })).count(), 2);
        shell.flush_deferred();
        assert_eq!(shell.store.medium["glc__D"], 0.0);
        assert_eq!(shell.store.medium["o2"], 0.0);
        assert_eq!(shell.store.solution.as_ref().expect("result").status.as_str(), "infeasible");
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "7".into() });
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "10".into() });
        assert_eq!(shell.deferred.values().filter(|(_, call)| matches!(call, Deferred::Medium { .. })).count(), 1);
        shell.flush_deferred();
        assert_eq!(shell.store.medium["glc__D"], 10.0);
        assert!(shell.store.solution.as_ref().expect("result").is_optimal());
    }

    #[test]
    fn rapid_changes_to_two_species_preserve_both_abundance_values() {
        let mut shell = make_shell();
        shell.store.use_scenario("aerobio-mixto").expect("two-species scenario");
        let first = shell.store.species[0].id.clone();
        let second = shell.store.species[1].id.clone();
        shell.debounce("abundance", 0.14, Deferred::Abundance { species: first.clone(), value: "0.2".into() });
        shell.debounce("abundance", 0.14, Deferred::Abundance { species: second.clone(), value: "0.8".into() });
        assert_eq!(shell.deferred.values().filter(|(_, call)| matches!(call, Deferred::Abundance { .. })).count(), 2);
        shell.flush_deferred();
        assert_eq!(shell.store.species.iter().find(|species| species.id == first).expect("first species").abundance, 0.2);
        assert_eq!(shell.store.species.iter().find(|species| species.id == second).expect("second species").abundance, 0.8);
    }
}

#[cfg(test)]
mod input_order_tests {
    use super::*;
    use crate::files::{ScriptedDialogs, SharedDialogs};

    fn make_shell() -> Shell {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        Shell::new(store, Box::new(ScriptedDialogs::new()))
    }

    #[test]
    fn pending_medium_edits_do_not_land_on_a_replacement_scenario() {
        let mut shell = make_shell();
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "1".into() });
        shell.queue_scenario_selection("sc:aerobio-mixto");
        shell.flush_deferred();
        assert_eq!(shell.store.medium["glc__D"], 10.0);
    }

    #[test]
    fn completing_a_model_download_retires_old_numeric_edits_and_preserves_searches() {
        let mut shell = make_shell();
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "1".into() });
        shell.debounce("pfba-biomass-percent", 0.18, Deferred::PfbaPercent("75".into()));
        shell.debounce("rx-search", 0.14, Deferred::ReactionSearch("biomass".into()));
        shell.store.use_scenario("aerobio-mixto").expect("downloaded context");
        shell.on_job_finished(coxeter_state::JobOutcome { id: 1, kind: JobKind::Download, result: Ok(()) });
        shell.flush_deferred();
        assert_eq!(shell.store.medium["glc__D"], 10.0);
        assert_eq!(shell.store.pfba_biomass_percent, 100.0);
        assert_eq!(shell.store.reaction_table.query, "biomass");
    }

    #[test]
    fn an_immediate_analysis_uses_pending_parameters_and_keeps_its_report() {
        let mut shell = make_shell();
        shell.debounce("pfba-biomass-percent", 0.18, Deferred::PfbaPercent("75".into()));
        shell.run_analysis("pfba");
        assert_eq!(shell.store.pfba_biomass_percent, 75.0);
        shell.flush_deferred();
        assert_eq!(shell.store.pfba_biomass_percent, 75.0);
        assert_eq!(shell.store.last_run.as_ref().expect("completed run").method, coxeter_state::AnalysisMethod::Pfba);
    }

    #[test]
    fn an_immediate_scenario_export_includes_pending_medium_values() {
        let mut shell = make_shell();
        let directory = std::env::temp_dir().join(format!("coxeter-input-order-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary export directory");
        let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(&directory));
        shell.dialogs = Box::new(dialogs.clone());
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "4".into() });
        shell.export_state();
        let path = dialogs.with(|dialogs| dialogs.saved[0].clone());
        let text = std::fs::read_to_string(&path).expect("saved scenario");
        std::fs::remove_file(&path).expect("remove trial export");
        std::fs::remove_dir(&directory).expect("remove trial export directory");
        let scenario: serde_json::Value = serde_json::from_str(&text).expect("scenario JSON");
        assert_eq!(scenario["medium"]["glc__D"], 4.0);
    }

    #[test]
    fn a_scenario_export_waits_for_the_requested_fixed_growth_result() {
        let mut shell = make_shell();
        shell.store.solve_steady_com_at(0.1).expect("initial fixed growth");
        for outcome in shell.store.wait_for_jobs(std::time::Duration::from_secs(10)) {
            outcome.result.expect("initial calculation");
        }
        shell.store.mark_scenario_saved();
        shell.store.solve_steady_com_at(0.2).expect("changed fixed growth");
        let directory = std::env::temp_dir().join(format!("coxeter-pending-growth-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary export directory");
        let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(&directory));
        shell.dialogs = Box::new(dialogs.clone());
        shell.export_state();
        let premature = dialogs.with(|dialogs| dialogs.saved.clone());
        let message = shell.store.status().message.clone();
        for path in &premature {
            std::fs::remove_file(path).expect("remove premature export");
        }
        for outcome in shell.store.wait_for_jobs(std::time::Duration::from_secs(10)) {
            outcome.result.expect("changed calculation");
        }
        shell.export_state();
        let path = dialogs.with(|dialogs| dialogs.saved.last().expect("completed export").clone());
        let text = std::fs::read_to_string(&path).expect("saved scenario");
        std::fs::remove_file(&path).expect("remove trial export");
        std::fs::remove_dir(&directory).expect("remove trial export directory");
        let scenario: serde_json::Value = serde_json::from_str(&text).expect("scenario JSON");
        assert!(premature.is_empty(), "a pending calculation wrote an older result");
        assert!(message.contains("Wait"), "{message}");
        assert_eq!(scenario["steadyCom"]["mu"], 0.2);
    }

    #[test]
    fn exports_do_not_write_the_previous_context_while_a_model_download_is_pending() {
        let mut shell = make_shell();
        let directory = std::env::temp_dir().join(format!("coxeter-pending-download-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary export directory");
        let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(&directory));
        shell.dialogs = Box::new(dialogs.clone());
        shell.store.start_yeast9_scenario(shell.store.yeast9_scenario());
        let exports: [fn(&mut Shell); 10] = [
            Shell::map_save,
            Shell::export_state,
            |shell| {
                shell.export_model(false);
            },
            |shell| {
                shell.export_model(true);
            },
            Shell::export_svg,
            Shell::export_png,
            Shell::export_csv,
            Shell::export_reproducibility,
            Shell::export_methods_provenance,
            Shell::export_state,
        ];
        for export in exports {
            export(&mut shell);
        }
        let asked = dialogs.with(|dialogs| dialogs.asked.clone());
        let paths = dialogs.with(|dialogs| dialogs.saved.clone());
        for path in paths {
            if path.exists() {
                std::fs::remove_file(path).expect("remove premature export");
            }
        }
        let outcomes = shell.store.wait_for_jobs(std::time::Duration::from_secs(10));
        shell.export_state();
        let path = dialogs.with(|dialogs| dialogs.saved.last().expect("recovered export").clone());
        let text = std::fs::read_to_string(&path).expect("saved scenario");
        std::fs::remove_file(path).expect("remove recovered export");
        std::fs::remove_dir(directory).expect("remove trial export directory");
        assert!(asked.is_empty(), "pending download opened export dialogs: {asked:?}");
        assert!(outcomes.iter().any(|outcome| outcome.kind == JobKind::Download && outcome.result.is_err()));
        let scenario: serde_json::Value = serde_json::from_str(&text).expect("scenario JSON");
        assert_eq!(scenario["scenario"], "solo-ecoli");
    }

    #[test]
    fn a_status_message_cannot_allow_reports_during_a_fixed_growth_calculation() {
        let mut shell = make_shell();
        shell.store.solve_steady_com_at(0.2).expect("fixed growth calculation");
        shell.status_warn("A different action changed the notice.");
        let bundle = shell.store.reproducibility_bundle();
        let report = shell.store.methods_provenance_report();
        for outcome in shell.store.wait_for_jobs(std::time::Duration::from_secs(10)) {
            outcome.result.expect("completed calculation");
        }
        assert!(bundle.is_err(), "a pending calculation produced a bundle");
        assert!(report.is_err(), "a pending calculation produced a Methods report");
        assert!(bundle.expect_err("unfinished analysis").to_string().contains("Wait"));
    }

    #[test]
    fn every_file_export_button_rejects_a_pending_context_without_opening_a_dialog() {
        use egui_kittest::kittest::Queryable;

        let mut shell = make_shell();
        let dialogs = SharedDialogs::new(ScriptedDialogs::new());
        shell.dialogs = Box::new(dialogs.clone());
        shell.store.start_yeast9_scenario(shell.store.yeast9_scenario());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .build_ui_state(|ui, shell: &mut Shell| {
                if !shell.fonts_installed {
                    crate::fonts::install(ui.ctx());
                    shell.fonts_installed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                panels::topbar::show(ui, shell);
            }, shell);
        harness.run_steps(4);
        for label in [
            "Export map JSON",
            "Export model JSON",
            "Export model SBML",
            "Export SVG map",
            "Export PNG map",
            "Export flux table (CSV)",
            "Export scenario",
            "Export reproducibility bundle",
            "Export methods / provenance report",
        ] {
            if harness.query_by_label(label).is_none() {
                harness.get_by_label("File menu").click();
                harness.run_steps(4);
            }
            harness.get_by_label(label).click();
            harness.run_steps(4);
            assert!(harness.state().store.status().message.contains("Wait for the current model load"), "{label}");
        }
        assert!(dialogs.with(|dialogs| dialogs.asked.is_empty()));
        harness.state_mut().store.wait_for_jobs(std::time::Duration::from_secs(10));
    }

    #[test]
    fn a_superseded_download_does_not_report_an_error_or_discard_newer_pending_inputs() {
        let mut shell = make_shell();
        let scenario = shell.store.yeast9_scenario();
        let id = scenario["id"].as_str().expect("download scenario id").to_string();
        if !shell.store.scenarios.iter().any(|scenario| scenario["id"] == id) {
            shell.store.scenarios.push(scenario);
        }
        shell.queue_scenario_selection(&format!("sc:{id}"));
        shell.queue_scenario_selection("sc:aerobio-mixto");
        let status = shell.store.status().clone();
        shell.debounce("medium", 0.12, Deferred::Medium { base: "glc__D".into(), value: "3".into() });
        for outcome in shell.store.wait_for_jobs(std::time::Duration::from_secs(10)) {
            shell.on_job_finished(outcome);
        }
        assert_eq!(shell.store.status(), &status);
        assert_eq!(shell.store.scenario_id().as_deref(), Some("aerobio-mixto"));
        shell.flush_deferred();
        assert_eq!(shell.store.medium["glc__D"], 3.0);
        assert!(shell.store.solution.as_ref().expect("newer solution").is_optimal());
    }

    #[test]
    fn native_fixed_growth_buttons_supersede_older_analysis_and_allow_cancel_then_retry() {
        use egui_kittest::kittest::Queryable;

        let mut shell = make_shell();
        shell.store.steady_com_range(false).expect("permitted range request");
        shell.store.wait_steady_com(std::time::Duration::from_secs(20)).expect("range completion").expect("permitted range");
        shell.ui.steady_com_draft = Some(crate::ui_state::SteadyComDraft {
            community_id: Some("solo-ecoli".into()),
            key: Some(shell.store.steady_com_key()),
            value: "0.1".into(),
            ..Default::default()
        });
        shell.store.run_analysis("fba").expect("older FBA");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .build_ui_state(|ui, shell: &mut Shell| {
                if !shell.fonts_installed {
                    crate::fonts::install(ui.ctx());
                    shell.fonts_installed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                panels::scenario::scenario_card(ui, shell);
            }, shell);
        harness.run_steps(4);
        harness.get_by_label("Solve at this µ").click();
        harness.run_steps(4);
        assert!(harness.state().store.steady_com_running());
        assert!(!harness.state().store.job_running(JobKind::Solve));
        assert!(harness.state().store.last_run.is_none());
        harness.get_by_label("Cancel").click();
        harness.run_steps(4);
        assert!(!harness.state().store.steady_com_running());
        assert!(!harness.state().ui.steady_com_pending);
        harness.get_by_label("Solve at this µ").click();
        harness.run_steps(4);
        assert!(harness.state().store.steady_com_running());
        let outcomes = harness.state_mut().store.wait_for_jobs(std::time::Duration::from_secs(20));
        for outcome in outcomes {
            harness.state_mut().on_job_finished(outcome);
        }
        let solution = harness.state().store.solution.as_ref().expect("fixed result");
        assert_eq!(solution.steady_com.as_ref().expect("fixed growth").mu, 0.1);
        assert!(harness.state().store.last_run.is_none());
        assert_eq!(harness.state().store.status().state, StatusState::Ok);
    }

    #[test]
    fn the_native_return_fba_button_retires_a_gimme_worker_and_immediately_allows_export() {
        use egui_kittest::kittest::Queryable;
        use coxeter_state::omics::CommunityOmicsPatch;

        let mut shell = make_shell();
        let mut raw: serde_json::Value = serde_json::from_str(include_str!("../../../../models/ecoli_core_mc.json")).expect("bundled core");
        for reaction in raw["reactions"].as_array_mut().expect("reactions") {
            if reaction["id"] == "GAPD" {
                reaction["gene_reaction_rule"] = serde_json::json!("test_gap");
            }
            if reaction["id"] == "PGI" {
                reaction["gene_reaction_rule"] = serde_json::json!("test_pgi");
            }
        }
        shell.store.models.insert("ecoli_core_mc", coxeter_core::model::Model::from_value(&raw).expect("annotated model"));
        shell.store.use_scenario("solo-ecoli").expect("annotated scenario");
        shell.store.import_metatranscriptomics(
            "MAG,gene,condition,TPM\neco,test_gap,Control,1\neco,test_pgi,Control,8\n", "return-fba-genes.csv",
        ).expect("expression");
        shell.store.run_community_gimme(CommunityOmicsPatch::default()).expect("installed GIMME");
        shell.store.start_community_gimme(CommunityOmicsPatch::default()).expect("pending GIMME");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1100.0, 1000.0))
            .build_ui_state(|ui, shell: &mut Shell| {
                if !shell.fonts_installed {
                    crate::fonts::install(ui.ctx());
                    shell.fonts_installed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                panels::model::community_omics_card(ui, shell);
            }, shell);
        harness.run_steps(4);
        harness.get_by_label("Return FBA").click();
        harness.run_steps(4);
        let export = harness.state().store.reproducibility_bundle();
        let solution = harness.state().store.solution.clone().expect("FBA result");
        let outcomes = harness.state_mut().store.wait_for_jobs(std::time::Duration::from_secs(20));
        for outcome in outcomes {
            harness.state_mut().on_job_finished(outcome);
        }
        assert_eq!(export.expect("immediate FBA export")["analysis"]["method"], "FBA");
        assert!(!harness.state().store.omics.gimme_enabled);
        assert!(std::sync::Arc::ptr_eq(harness.state().store.solution.as_ref().expect("retained result"), &solution));
        assert_eq!(harness.state().store.status().state, StatusState::Ok);
    }
}

#[cfg(test)]
mod retired_worker_tests {
    use super::*;
    use crate::files::ScriptedDialogs;

    #[test]
    fn cancelled_workers_keep_the_idle_poll_timer_until_their_final_messages_are_drained() {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        store.show_full_map_in_background();
        let job = store.running_jobs().into_iter().find(|job| job.kind == JobKind::Layout).expect("layout job");
        store.cancel_job(job.id);
        let mut shell = Shell::new(store, Box::new(ScriptedDialogs::new()));
        let context = egui::Context::default();
        let mut delay = std::time::Duration::MAX;
        for _ in 0..6 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                shell.after_frame(ui.ctx());
            });
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(delay <= std::time::Duration::from_millis(60), "The cancelled worker stopped being polled: {delay:?}");
        shell.store.wait_for_jobs(std::time::Duration::from_secs(20));
        for _ in 0..6 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                shell.after_frame(ui.ctx());
            });
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(delay > std::time::Duration::from_millis(60), "A drained worker kept polling: {delay:?}");
    }
}
