//! The actions of `src/ui/app.js` that are not one panel's: workspaces
//! (`openWorkspace`, `closeWorkspace`), tools (`activateTool`), the theme
//! (`setTheme`), the camera (`zoomBy`, `centerOnReaction`,
//! `centerOnPathway`), the dock and panel toggles of `onSidebarClick`,
//! scenario selection (`queueScenarioSelection`), and every import and
//! export (the file inputs' `change` handlers, `exportSVG`, `exportPNG`,
//! `exportReproducibility`, `exportMethodsProvenance`, `downloadFindings`,
//! `downloadMemberJSON`, `downloadMemberSBML`, `downloadCommunityCSV`).
//!
//! `download(name, text, type)` becomes a save dialog suggesting `name`; a
//! cancelled dialog writes nothing and records nothing as saved.

use std::path::{Path, PathBuf};

use coxeter_canvas::Tool;
use coxeter_io::file_names::{self, export_suffix};
use coxeter_io::model_import::UploadFile;
use coxeter_state::StatusState;
use serde_json::Value;

use crate::app::Shell;
use crate::files::{self, Filter, filter};
use crate::host::StoreHost;
use crate::ui_state::{LeftTab, ResultsTab, RightTab, Workspace};

impl Shell {
    // Workspaces

    /// `openWorkspace(name)` by its `data-workspace` name.
    pub fn open_workspace_named(&mut self, name: &str) {
        if let Some(ws) = Workspace::parse(name) {
            self.open_workspace(ws);
        }
    }

    /// `openWorkspace(name)`: one editor page over the map.
    pub fn open_workspace(&mut self, ws: Workspace) {
        self.ui.workspace = Some(ws);
        self.ui.last_workspace = ws;
        self.close_map_menus();
        self.ui.task_overlay = false;
        self.ui.results_maximized = false;
        self.ui.view_open = false;
        // The library and the Reactions page open with the keyboard in their search field.
        if ws == Workspace::Library {
            self.ui.focus_request = Some(crate::panels::map_options::reaction_search_id());
        }
        if ws == Workspace::Reactions {
            self.ui.focus_request = Some(crate::panels::reactions::search_id());
        }
    }

    // Reactions page

    /// `revealInReactions(rid)`: open the Reactions page on one reaction,
    /// clearing a search or filter that hides it.
    pub fn reveal_in_reactions(&mut self, rid: &str) {
        if self.ui.workspace != Some(Workspace::Reactions) {
            self.open_workspace(Workspace::Reactions);
            self.ui.focus_request = None;
        }
        let model = self.reaction_table_model();
        let listed = model.items.iter().any(|i| matches!(i, coxeter_state::reactions_explorer::TableItem::Row(r) if r.id() == rid));
        if !listed {
            self.store.reaction_table.collapsed.clear();
            let filtered = !model.visible.iter().any(|r| r.id() == rid);
            let patch = if filtered {
                coxeter_state::reactions::ReactionTablePatch { query: Some(String::new()), filters: Some(Vec::new()), ..Default::default() }
            } else {
                Default::default()
            };
            self.store.set_reaction_table(patch);
        }
        if self.store.selected_reaction.as_deref() != Some(rid) {
            self.store.select(Some(rid));
        }
        self.ui.rx.reveal = Some(rid.to_string());
    }

    /// `locateReaction(rid)`: select a reaction and centre the map on it,
    /// bringing the map into view above the page if need be.
    pub fn locate_reaction(&mut self, rid: &str) {
        if !self.store.map.as_ref().is_some_and(|m| m.has_reaction(rid)) {
            // Still selected, so the inspector shows it: only the map has nothing to show.
            self.store.select(Some(rid));
            let title = self.store.reaction_title(rid);
            self.status_warn(format!("{title} is not drawn on the map. Add it from the Reaction library."));
            return;
        }
        if self.ui.workspace == Some(Workspace::Reactions) {
            self.ui.rx.split = true;
        } else {
            self.close_workspace_quietly();
        }
        self.store.select(Some(rid));
        self.ui.rx.center_on = Some(rid.to_string());
    }

    /// `exportBoundsCSV()`: every reaction's bounds in force.
    pub fn export_bounds_csv(&mut self) {
        if self.store.community.is_none() {
            return;
        }
        let name = self.store.file_name_for(export_suffix::BOUNDS, "csv");
        let csv = self.store.bounds_csv(None);
        if let Some(path) = self.save_file("Export reaction bounds", &name, &[filter::CSV], csv.as_bytes()) {
            self.log_ok(&format!("Bounds exported as {}", files::display_name(&path)));
        }
    }

    /// The bounds file input's `change`: apply a bounds CSV.
    pub fn import_bounds_csv(&mut self) {
        let Some((name, text)) = self.pick_texts("Import reaction bounds", &[filter::CSV, filter::TABLE], false).into_iter().next() else {
            return;
        };
        let result = text.and_then(|t| self.store.import_bounds_csv(&t, &name).map(|_| ()).map_err(|e| e.to_string()));
        if let Err(e) = result {
            self.status_error(format!("Could not import bounds: {e}"));
        }
    }

    /// `closeWorkspace()`. Leaving Scenario & objective commits the choice
    /// in its list (the JS awaits a pending load; here loads are applied as
    /// they are chosen).
    pub fn close_workspace(&mut self) {
        let returning_from_scenario = self.ui.workspace == Some(Workspace::Scenario);
        if returning_from_scenario {
            if let Some(choice) = self.ui.scenario_choice.take() {
                if choice != self.active_scenario_value() {
                    self.queue_scenario_selection(&choice);
                }
            }
        }
        self.ui.workspace = None;
        if returning_from_scenario {
            self.refit();
        }
    }

    /// The value of the scenario list for what is open (`pub:<id>` or `sc:<id>`).
    pub fn active_scenario_value(&self) -> String {
        match (self.store.scenario_published(), self.store.scenario_id()) {
            (Some(p), _) => format!("pub:{p}"),
            (None, Some(id)) => format!("sc:{id}"),
            _ => String::new(),
        }
    }

    /// `queueScenarioSelection(value)`: `pub:<id>` opens a published
    /// community, `sc:<id>` a teaching scenario.
    pub fn queue_scenario_selection(&mut self, value: &str) {
        self.refit();
        let (kind, id) = value.split_once(':').unwrap_or(("", ""));
        if id.is_empty() {
            return;
        }
        let result = if kind == "pub" {
            self.store.use_published_community(id)
        } else {
            let yeast9 = self
                .store
                .scenarios
                .iter()
                .find(|s| s.get("id").and_then(Value::as_str) == Some(id))
                .filter(|s| s.get("loader").and_then(Value::as_str) == Some("yeast9"))
                .cloned();
            match yeast9 {
                // The download runs in the background; `poll_jobs` installs it.
                Some(scenario) => {
                    self.store.start_yeast9_scenario(scenario);
                    Ok(())
                }
                None => self.store.use_scenario(id),
            }
        };
        if let Err(error) = result {
            self.status_error(format!("Could not load community: {error}"));
        }
        self.ui.scenario_choice = None;
    }

    // Tools

    /// `activateTool(tool)`, with what the tool buttons of `onSidebarClick` add:
    /// the workspace closes, and the drawing tools go back to Edit Map.
    pub fn activate_tool(&mut self, tool: Tool) {
        self.close_workspace_quietly();
        self.close_map_menus();
        let mut host = StoreHost::new(&mut self.store);
        self.map.activate_tool(tool, &mut host);
        self.drain_canvas_events();
        if matches!(tool, Tool::Reaction | Tool::Metabolite | Tool::Compartment | Tool::Cell) {
            self.ui.left_tab = LeftTab::Build;
        }
        self.ui.task_overlay = false;
    }

    /// `closeWorkspace({ restoreFocus: false })`.
    pub fn close_workspace_quietly(&mut self) {
        if self.ui.workspace.is_some() {
            self.close_workspace();
        }
    }

    /// `edit-selected-cell-boundary` / `edit-selected-compartment-boundary`:
    /// the geometry editor on the selected membrane.
    pub fn edit_selected_boundary(&mut self, cell_id: Option<&str>) {
        self.activate_tool(Tool::Select);
        let _ = self.store.set_view(coxeter_state::ViewPatch {
            text_tool: Some(false),
            geometry_edit: Some(true),
            multi_select: Some(false),
            ..Default::default()
        });
        if let Some(cell) = cell_id {
            self.store.select_cell_geometry(cell);
        }
    }

    /// Events the canvas queued outside its own frame (a tool activated
    /// from the panels closes its menus).
    pub fn drain_canvas_events(&mut self) {
        for event in self.map.canvas.take_events() {
            self.on_canvas_event(event);
        }
    }

    /// `closeMapMenus()`.
    pub fn close_map_menus(&mut self) {
        self.menus.close_all();
    }

    // Theme

    /// `setTheme(dark)`: the scheme, remembered for the next start.
    pub fn set_theme(&mut self, dark: bool) {
        let _ = self.store.set_view(coxeter_state::ViewPatch { dark: Some(dark), ..Default::default() });
        if let Some(path) = &self.settings_path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, serde_json::json!({ "theme": if dark { "dark" } else { "light" } }).to_string());
        }
    }

    /// The theme remembered by [`Shell::set_theme`], if any.
    pub fn saved_theme(path: &Path) -> Option<bool> {
        let text = std::fs::read_to_string(path).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        match v.get("theme")?.as_str()? {
            "dark" => Some(true),
            "light" => Some(false),
            _ => None,
        }
    }

    // Camera

    /// `camera.fitted = false`: the next frame fits the map to the stage.
    pub fn refit(&mut self) {
        self.map.canvas.camera.fitted = false;
    }

    /// `zoomBy(factor)`.
    pub fn zoom_by(&mut self, factor: f64) {
        self.map.zoom_by(factor);
    }

    /// `zoom-fit`.
    pub fn zoom_fit(&mut self) {
        let host = StoreHost::new(&mut self.store);
        self.map.fit(&host);
    }

    /// `centerOnReaction(reactionId)`.
    pub fn center_on_reaction(&mut self, reaction_id: &str) -> bool {
        let host = StoreHost::new(&mut self.store);
        self.map.center_on_reaction(&host, reaction_id)
    }

    /// `centerOnPathway(pathwayKey)`.
    pub fn center_on_pathway(&mut self, key: &str) -> bool {
        let ids = self.store.pathway_reaction_ids(key);
        let host = StoreHost::new(&mut self.store);
        self.map.center_on_reactions(&host, &ids)
    }

    /// `select-result-reaction`, `search-select-reaction`: select and show it.
    pub fn select_and_center(&mut self, reaction_id: &str) {
        self.store.select(Some(reaction_id));
        self.center_on_reaction(reaction_id);
    }

    // Panels and dock

    /// `left-tab`.
    pub fn set_left_tab(&mut self, tab: LeftTab) {
        // Map-editing tools are scoped to Edit Map. Leaving that workspace
        // must cancel any pending draw/edit mode so clicks in Edit Model or
        // Simulate cannot accidentally mutate the map when the user comes back.
        if tab != LeftTab::Build {
            let mut host = StoreHost::new(&mut self.store);
            self.map.activate_tool(Tool::Select, &mut host);
            self.drain_canvas_events();
        }
        self.ui.left_tab = tab;
    }

    /// `right-tab`.
    pub fn set_right_tab(&mut self, tab: RightTab) {
        self.ui.right_tab = tab;
        self.ui.inspector_pinned = true;
        self.ui.inspector_collapsed = false;
        self.ui.focus_mode = false;
    }

    /// `toggle-left`. A narrow window or focus mode lays the panel over the map.
    pub fn toggle_left(&mut self, narrow: bool) {
        if narrow || self.ui.focus_mode {
            self.ui.task_overlay = !self.ui.task_overlay;
            self.ui.left_collapsed = false;
        } else {
            self.ui.left_collapsed = !self.ui.left_collapsed;
        }
    }

    /// `toggle-inspector`: explicit beats automatic.
    pub fn toggle_inspector(&mut self) {
        self.ui.inspector_pinned = true;
        self.ui.focus_mode = false;
        self.ui.inspector_collapsed = !self.ui.inspector_collapsed;
    }

    /// `toggle-results`.
    pub fn toggle_results(&mut self) {
        self.ui.results_maximized = false;
        self.ui.results_collapsed = !self.ui.results_collapsed;
    }

    /// `maximize-results`.
    pub fn maximize_results(&mut self) {
        self.close_workspace_quietly();
        self.ui.results_collapsed = false;
        self.ui.focus_mode = false;
        self.ui.results_maximized = !self.ui.results_maximized;
    }

    /// `show-results`.
    pub fn show_results(&mut self, stage_height: f32) {
        self.close_workspace_quietly();
        self.ui.results_collapsed = false;
        self.ui.task_overlay = false;
        self.ui.focus_mode = false;
        if stage_height < 320.0 {
            self.ui.results_maximized = true;
        }
    }

    /// `results-tab`.
    pub fn set_results_tab(&mut self, tab: ResultsTab) {
        self.close_workspace_quietly();
        self.ui.results_collapsed = false;
        self.ui.focus_mode = false;
        self.ui.results_tab = tab;
    }

    /// `toggle-focus`.
    pub fn toggle_focus(&mut self) {
        self.ui.focus_mode = !self.ui.focus_mode;
        self.ui.task_overlay = false;
    }

    /// `layout-preset`.
    pub fn layout_preset(&mut self, name: &str) {
        self.refit();
        self.ui.right_tab = RightTab::Layout;
        self.ui.inspector_pinned = true;
        self.ui.inspector_collapsed = false;
        self.ui.focus_mode = false;
        self.store.set_layout_preset(name);
    }

    // Analyses

    /// `run-fba`, `run-pfba`, `run-fva`, `run-loopless`.
    pub fn run_analysis(&mut self, method: &str) {
        if let Err(error) = self.store.run_analysis(method) {
            self.status_error(error.to_string());
        }
    }

    // Map

    /// `map-full`: on a genome-scale model this replaces the drawing after a
    /// long layout, so it asks first.
    pub fn map_full(&mut self) {
        if let Some(warning) = self.store.full_map_warning() {
            if !self.dialogs.confirm("Draw the full map", &warning) {
                return;
            }
        }
        self.close_workspace_quietly();
        self.close_map_menus();
        self.refit();
        if self.store.too_large_to_auto_draw() {
            // The layout of a genome-scale model takes minutes: off the
            // interface thread, installed by `poll_jobs`.
            self.store.show_full_map_in_background();
        } else {
            self.store.show_full_map();
        }
    }

    /// `map-empty`.
    pub fn map_empty(&mut self) {
        self.close_workspace_quietly();
        self.close_map_menus();
        self.refit();
        let cleared = self.store.map.as_ref().map_or(0, |m| m.reactions().len());
        self.store.show_empty_map();
        self.status_ok(if cleared > 0 {
            format!("Map cleared — {cleared} drawn reactions removed. The model is unchanged; Ctrl+Z undoes this.")
        } else {
            "Map cleared. The model is unchanged.".to_string()
        });
    }

    /// `map-core-overview`.
    pub fn map_core_overview(&mut self) {
        self.close_workspace_quietly();
        self.close_map_menus();
        self.refit();
        self.store.show_core_metabolism_overview(true);
    }

    /// `map-undo` / `map-redo`.
    pub fn undo_redo(&mut self, redo: bool) {
        let mut host = StoreHost::new(&mut self.store);
        self.map.canvas.gestures.finish_metabolite_label_drag_for_history(&mut host);
        self.close_map_menus();
        if redo {
            self.store.redo_map();
        } else {
            self.store.undo_map();
        }
    }

    // Files

    /// `download(name, bytes)`: a save dialog suggesting `name`. Returns the
    /// file written, `None` when the dialog was cancelled or writing failed
    /// (which the status line then says).
    pub fn save_file(&mut self, title: &str, name: &str, filters: &[Filter], bytes: &[u8]) -> Option<PathBuf> {
        let path = self.dialogs.save(title, name, filters)?;
        match std::fs::write(&path, bytes) {
            Ok(()) => Some(path),
            Err(error) => {
                self.status_error(format!("Could not write {}: {error}", files::display_name(&path)));
                None
            }
        }
    }

    /// `input.click()` and `file.text()`: the files picked, read.
    fn pick_texts(&mut self, title: &str, filters: &[Filter], multiple: bool) -> Vec<(String, Result<String, String>)> {
        self.dialogs.open(title, filters, multiple).into_iter().map(|p| (files::display_name(&p), files::read_text(&p))).collect()
    }

    /// `map-save`.
    pub fn map_save(&mut self) {
        // Before a model has loaded there is no map; say so rather than throw.
        if self.store.map.is_none() {
            self.status_warn("There is no map to save yet.");
            return;
        }
        let name = self.store.file_name_for(export_suffix::MAP, "json");
        let text = match self.store.map_json() {
            Ok(t) => t,
            Err(error) => return self.status_error(error.to_string()),
        };
        if let Some(path) = self.save_file("Save map", &name, &[filter::JSON], text.as_bytes()) {
            self.store.mark_map_saved();
            self.log_ok(&format!("Map saved as {}", files::display_name(&path)));
        }
    }

    /// `map-load` (the map file input's `change`).
    pub fn map_load(&mut self) {
        let Some((name, text)) = self.pick_texts("Open map", &[filter::JSON], false).into_iter().next() else { return };
        let text = match text {
            Ok(t) => t,
            Err(e) => return self.status_error(format!("Could not load the map: {e}")),
        };
        self.load_map_named(&text, Some(&name));
    }

    /// `discard-waiting-map`: forget the map that waits for its model.
    pub fn discard_waiting_map(&mut self) {
        self.store.discard_waiting_map();
        self.status_ok("The waiting map was forgotten.");
    }

    /// Open a map document's text (the file input, or a file on the command line).
    pub fn load_map_text(&mut self, text: &str) {
        self.load_map_named(text, None);
    }

    /// The map file input's `change`: open a map document's text, from the
    /// file `name`.
    pub fn load_map_named(&mut self, text: &str, name: Option<&str>) {
        self.close_map_menus();
        self.refit();
        let was = self.store.scenario_id();
        match self.store.load_map_document_named(text, name) {
            Ok(()) if self.store.waiting_map.is_some() => {
                let waiting = self.store.waiting_map.clone().expect("checked");
                let file = name.map_or_else(|| "The map".to_string(), |n| format!("“{n}”"));
                self.store.set_status_message(
                    StatusState::Warn,
                    format!(
                        "{file} does not fit what is open: {} The map is kept and will be drawn as soon as {} is loaded (File ▸ Model). The current drawing is unchanged.",
                        waiting.reason, waiting.needs
                    ),
                );
                self.store.emit("status");
            }
            Ok(()) => {
                let moved = self.store.scenario_id() != was;
                let (reactions, nodes) = self.store.map.as_ref().map_or((0, 0), |m| (m.reactions().len(), m.nodes().len()));
                let mut message = format!("Map loaded: {reactions} reactions, {nodes} metabolites.");
                if moved {
                    let name = self.store.scenario_str("name").or_else(|| self.store.scenario_id()).unwrap_or_default();
                    message.push_str(&format!(" Switched to “{name}”, which this map was drawn over."));
                }
                message.push_str(&self.store.map_load_note);
                // Anything the load had to change to make the file fit is
                // said where it will be read.
                let state = if self.store.map_load_note.is_empty() { StatusState::Ok } else { StatusState::Warn };
                self.store.set_status_message(state, message);
                self.store.emit("status");
            }
            Err(error) => self.status_error(format!("Could not load the map: {error}")),
        }
    }

    /// `import-models` (the model file input's `change`).
    pub fn import_models(&mut self) {
        // MATLAB and Excel files are binary, so every model is read as bytes.
        let picked: Vec<(String, Result<Vec<u8>, String>)> = self
            .dialogs
            .open("Load model(s)", &[filter::MODEL], true)
            .into_iter()
            .map(|p| (files::display_name(&p), files::read_bytes(&p)))
            .collect();
        if picked.is_empty() {
            return;
        }
        self.import_model_bytes(picked);
    }

    /// Read model files (COBRA JSON, SBML, COBRA Toolbox .mat or .xlsx) into a new community.
    pub fn import_model_bytes(&mut self, picked: Vec<(String, Result<Vec<u8>, String>)>) {
        self.close_map_menus();
        self.refit();
        self.store.set_status_message(StatusState::Busy, format!("Reading {} model file(s)…", picked.len()));
        self.store.emit("status");
        let mut files = Vec::new();
        for (name, bytes) in picked {
            match bytes {
                Ok(bytes) => files.push(UploadFile { name, bytes }),
                Err(e) => return self.status_error(format!("Could not load the model: {e}")),
            }
        }
        if let Err(error) = self.store.import_model_uploads(&files) {
            self.status_error(format!("Could not load the model: {error}"));
        }
    }

    /// `load-yeast9` (`loadYeast9Model()`): the download runs in the background.
    pub fn load_yeast9(&mut self) {
        self.close_map_menus();
        self.refit();
        let scenario = self.store.yeast9_scenario();
        self.store.start_yeast9_scenario(scenario);
    }

    /// `import-state` (the scenario file input's `change`).
    pub fn import_state(&mut self) {
        let Some((name, text)) = self.pick_texts("Open scenario", &[filter::JSON], false).into_iter().next() else { return };
        match text.map_err(|e| e.to_string()).and_then(|t| self.store.load_state(&t).map_err(|e| e.to_string())) {
            Ok(()) => self.log_ok(&format!("Scenario loaded from {name}")),
            Err(e) => self.status_error(format!("Could not load: {e}")),
        }
    }

    /// `import-omics` (the omics file input's `change`).
    pub fn import_omics(&mut self) {
        let Some((name, text)) = self.pick_texts("Load omics CSV", &[filter::CSV, filter::TABLE], false).into_iter().next() else {
            return;
        };
        self.import_omics_text(&name, text);
    }

    /// Import the omics table `name`, already read (`text`, or why it could
    /// not be), and say how it went.
    pub fn import_omics_text(&mut self, name: &str, text: Result<String, String>) {
        let data_type = self.store.omics_settings.data_type.clone();
        let result = text.and_then(|t| self.store.import_omics_csv(&t, name, &data_type).map(|_| ()).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                let (rows, conditions) = self.store.omics_dataset.as_ref().map_or((0, 0), |d| (d.row_count, d.conditions.len()));
                self.status_ok(format!("Omics loaded: {name} · {rows} IDs · {conditions} condition(s)."));
            }
            Err(e) => self.status_error(format!("Could not load omics: {e}")),
        }
    }

    /// `import-metatranscriptomics`.
    pub fn import_metatranscriptomics(&mut self) {
        let Some((name, text)) = self.pick_texts("Load metatranscriptomics", &[filter::TABLE], false).into_iter().next() else {
            return;
        };
        self.import_metatranscriptomics_text(&name, text);
    }

    /// Import the metatranscriptomics table `name`, already read; an error
    /// goes to the status line.
    pub fn import_metatranscriptomics_text(&mut self, name: &str, text: Result<String, String>) {
        let result = text.and_then(|t| self.store.import_metatranscriptomics(&t, name).map(|_| ()).map_err(|e| e.to_string()));
        if let Err(e) = result {
            self.status_error(format!("Could not load metatranscriptomics: {e}"));
        }
    }

    /// `import-reproducibility`.
    pub fn import_reproducibility(&mut self) {
        let Some((_, text)) = self.pick_texts("Open reproducibility bundle", &[filter::JSON], false).into_iter().next() else {
            return;
        };
        self.import_reproducibility_text(text);
    }

    /// Replay a reproducibility bundle already read; an error goes to the
    /// status line.
    pub fn import_reproducibility_text(&mut self, text: Result<String, String>) {
        let result = text
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
            .and_then(|bundle| self.store.import_reproducibility_bundle(&bundle).map(|_| ()).map_err(|e| e.to_string()));
        if let Err(e) = result {
            self.status_error(format!("Could not replay the bundle: {e}"));
        }
    }

    /// Open files given on the command line, each as what it looks like.
    pub fn open_paths(&mut self, paths: &[PathBuf]) {
        let mut models = Vec::new();
        let mut rest = Vec::new();
        for path in paths {
            let name = files::display_name(path);
            let bytes = match files::read_bytes(path) {
                Ok(b) => b,
                Err(e) => {
                    self.status_error(format!("Could not open {e}"));
                    continue;
                }
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            match files::classify(path, &text) {
                Some(files::FileKind::Model) => models.push((name, Ok(bytes))),
                Some(kind) => rest.push((kind, name, text)),
                None => self.status_error(format!("Could not tell what {name} is: open it from the File menu.")),
            }
        }
        // Models first: a map or scenario opened with them is drawn over them.
        if !models.is_empty() {
            self.import_model_bytes(models);
        }
        for (kind, name, text) in rest {
            match kind {
                files::FileKind::Map => self.load_map_text(&text),
                files::FileKind::Scenario => match self.store.load_state(&text) {
                    Ok(()) => self.log_ok(&format!("Scenario loaded from {name}")),
                    Err(e) => self.status_error(format!("Could not load: {e}")),
                },
                files::FileKind::Bundle => self.import_reproducibility_text(Ok(text)),
                files::FileKind::Metatranscriptomics => self.import_metatranscriptomics_text(&name, Ok(text)),
                files::FileKind::Omics => self.import_omics_text(&name, Ok(text)),
                files::FileKind::Model => {}
            }
        }
    }

    /// `exportSVG()`.
    pub fn export_svg(&mut self) {
        let Some(svg) = self.exportable_svg() else { return };
        let name = self.store.file_name_for(export_suffix::MAP, "svg");
        if let Some(path) = self.save_file("Export SVG map", &name, &[filter::SVG], svg.as_bytes()) {
            self.log_ok(&format!("Map exported as {}", files::display_name(&path)));
        }
    }

    /// `exportableSVG()`: the map drawn afresh, without the editor.
    pub fn exportable_svg(&self) -> Option<String> {
        let map = self.store.map.as_ref()?;
        self.store.scale.as_ref()?;
        let mut state = self.store.map_render_state(Default::default());
        // Hover text goes into the file: a reaction or metabolite says what it
        // is and what flux it carries wherever the SVG is opened.
        state.tooltips = Some(std::sync::Arc::new(self.store.export_tooltips()));
        let metadata = self.zuniga_svg_metadata();
        let about = self.store.export_metadata();
        let title = about["scenario"]["name"].as_str().map(str::to_string).unwrap_or_else(|| self.store.export_base_name());
        let opts = coxeter_render::ExportOptions { metadata, about: Some(coxeter_core::js::stringify(&about, None)), title: Some(title) };
        Some(coxeter_render::exportable_svg(map.graph(), &state, &opts))
    }

    /// What an SVG of the Zuñiga case carries in its `<metadata>`.
    fn zuniga_svg_metadata(&self) -> Option<String> {
        if !self.store.is_zuniga_study() {
            return None;
        }
        let mut meta = self.store.zuniga_state().as_object().cloned().unwrap_or_default();
        meta.insert("method".into(), "Stationary source-model FBA".into());
        meta.insert("units".into(), "Source-model flux units; growth h^-1".into());
        meta.insert("solutionStatus".into(), self.store.solution.as_ref().map_or(Value::Null, |s| s.status.as_str().into()));
        let growth: serde_json::Map<String, Value> =
            self.store.growth.iter().map(|(k, v)| (k.clone(), coxeter_state::util::num_json(*v))).collect();
        meta.insert("growth".into(), Value::Object(growth));
        meta.insert("adaptation".into(), "Not a dynamic or experimental trajectory reproduction.".into());
        Some(coxeter_core::js::stringify(&Value::Object(meta), None))
    }

    /// `exportPNG()`.
    pub fn export_png(&mut self) {
        if self.store.map.is_none() {
            return;
        }
        if self.store.scale.is_none() {
            return;
        }
        // Drawn once the file has a place to go (`saveFile` with a function):
        // a large map takes a while to draw, and Cancel draws nothing.
        let name = self.store.file_name_for(export_suffix::MAP, "png");
        let Some(path) = self.dialogs.save("Export PNG map", &name, &[filter::PNG]) else { return };
        let Some(map) = self.store.map.as_ref() else { return };
        let state = self.store.map_render_state(Default::default());
        let png = match coxeter_render::export_png(map.graph(), &state) {
            Ok(p) => p,
            Err(error) => return self.status_error(format!("Could not export PNG: {error}")),
        };
        if let Err(error) = std::fs::write(&path, &png.png) {
            return self.status_error(format!("Could not write {}: {error}", files::display_name(&path)));
        }
        if png.scale >= coxeter_render::PREFERRED_SCALE {
            self.log_ok(&format!("Map exported as {} ({} × {} px)", files::display_name(&path), png.width, png.height));
        } else {
            self.status_warn(format!(
                "PNG exported at {}× ({} × {} px): a {}× image of this map exceeds the canvas limit. The SVG keeps full resolution.",
                coxeter_core::js::number_to_string(png.scale),
                png.width,
                png.height,
                coxeter_core::js::number_to_string(coxeter_render::PREFERRED_SCALE)
            ));
        }
    }

    /// `export-csv`.
    pub fn export_csv(&mut self) {
        let name = self.store.file_name_for(export_suffix::FLUXES, "csv");
        let csv = self.store.flux_csv();
        if let Some(path) = self.save_file("Export flux table", &name, &[filter::CSV], csv.as_bytes()) {
            self.log_ok(&format!("Flux table exported as {}", files::display_name(&path)));
        }
    }

    /// `export-state`.
    pub fn export_state(&mut self) {
        let name = self.store.file_name_for(export_suffix::SCENARIO, "json");
        let text = self.store.state_json();
        if let Some(path) = self.save_file("Save scenario", &name, &[filter::JSON], text.as_bytes()) {
            self.store.mark_scenario_saved();
            self.log_ok(&format!("Scenario saved as {}", files::display_name(&path)));
        }
    }

    /// `export-model-json` / `export-model-sbml`: the model of the model editor.
    pub fn export_model(&mut self, sbml: bool) {
        let Some(sp) = self.store.model_editor_species().cloned() else { return };
        let Some(model_id) = self.store.model_editor_model().map(|m| if m.id.is_empty() { sp.id.clone() } else { m.id.clone() }) else {
            return;
        };
        let (ext, filters, text) = if sbml {
            ("xml", [filter::SBML], self.store.model_sbml(Some(&sp.id)))
        } else {
            ("json", [filter::JSON], self.store.model_json(Some(&sp.id)))
        };
        let text = match text {
            Ok(t) => t,
            Err(error) => return self.status_error(format!("Could not export the model: {error}")),
        };
        let name = file_names::export_file_name(&model_id, export_suffix::MODEL, ext);
        if let Some(path) = self.save_file("Export model", &name, &filters, text.as_bytes()) {
            self.store.mark_model_saved(Some(&sp.file));
            self.log_ok(&format!("Model {} saved as {}", sp.id, files::display_name(&path)));
        }
    }

    /// `exportReproducibility()`.
    pub fn export_reproducibility(&mut self) {
        let bundle = match self.store.reproducibility_bundle() {
            Ok(b) => b,
            Err(error) => return self.status_error(format!("Could not export provenance: {error}")),
        };
        let name = file_names::timestamped_file_name_now(&self.store.export_base_name(), export_suffix::REPRO, "json");
        let text = coxeter_core::js::stringify(&bundle, Some(2));
        if self.save_file("Export reproducibility bundle", &name, &[filter::JSON], text.as_bytes()).is_some() {
            let hash = bundle.get("integrity").and_then(|i| i.get("payloadHash")).and_then(Value::as_str).unwrap_or("");
            let short: String = hash.chars().take(12).collect();
            self.status_ok(format!("Reproducibility bundle exported · SHA-256 {short}…"));
        }
    }

    /// `exportMethodsProvenance()`.
    pub fn export_methods_provenance(&mut self) {
        let (bundle, text) = match self.store.methods_provenance_report() {
            Ok(r) => r,
            Err(error) => return self.status_error(format!("Could not export the Methods report: {error}")),
        };
        let name = file_names::timestamped_file_name_now(&self.store.export_base_name(), export_suffix::METHODS, "md");
        if self.save_file("Export Methods / provenance report", &name, &[filter::MARKDOWN], text.as_bytes()).is_some() {
            let commit = bundle
                .pointer("/metadata/software/gitCommit")
                .and_then(Value::as_str)
                .map(|c| format!("commit {}", c.chars().take(12).collect::<String>()))
                .unwrap_or_else(|| "commit not injected".into());
            self.status_ok(format!("Methods / Provenance report exported · {commit}"));
        }
    }

    /// `downloadFindings(communityId)`: findings and adaptations together, as Markdown.
    pub fn download_findings(&mut self, community_id: &str) {
        let Some(c) = self.store.published_communities.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(community_id)).cloned()
        else {
            return;
        };
        let text = findings_markdown(&c);
        let name = file_names::export_file_name(community_id, export_suffix::FINDINGS, "md");
        self.save_file("Download findings", &name, &[filter::MARKDOWN], text.as_bytes());
    }

    /// `downloadMemberJSON(communityId, memberId)`.
    pub fn download_member_json(&mut self, community_id: &str, member_id: &str) {
        if !self.store.published_communities.iter().any(|c| c.get("id").and_then(Value::as_str) == Some(community_id)) {
            return;
        }
        let result = (|| -> Result<String, String> {
            if self.store.is_zuniga_study() && self.store.scenario_published().as_deref() == Some(community_id) {
                let model = self
                    .store
                    .models
                    .get(&format!("published/{community_id}/model-{member_id}"))
                    .ok_or("The privately loaded model is unavailable.")?;
                return Ok(coxeter_core::js::stringify(&model.to_json(), Some(2)));
            }
            let manifest = self.store.published_manifest(community_id);
            let file = manifest
                .as_ref()
                .and_then(|m| m.get("members"))
                .and_then(Value::as_array)
                .and_then(|ms| ms.iter().find(|m| m.get("id").and_then(Value::as_str) == Some(member_id)))
                .and_then(|m| m.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("{member_id} has no model file in this build."))?;
            let key = format!("published/{community_id}/{}", file.strip_suffix(".json").unwrap_or(&file));
            let model = self.store.load_model(&key).map_err(|e| e.to_string())?;
            Ok(coxeter_core::js::stringify(&model.to_json(), Some(2)))
        })();
        match result {
            Ok(text) => {
                let name = file_names::member_model_file_name(community_id, member_id, "json");
                self.save_file("Download member model", &name, &[filter::JSON], text.as_bytes());
            }
            Err(e) => self.status_error(format!("Could not export JSON: {e}")),
        }
    }

    /// `downloadMemberSBML(communityId, memberId)`.
    pub fn download_member_sbml(&mut self, community_id: &str, member_id: &str) {
        if community_id.is_empty() || member_id.is_empty() {
            return;
        }
        match self.store.published_member_sbml(community_id, member_id) {
            Ok((xml, _)) => {
                let name = file_names::member_model_file_name(community_id, member_id, "xml");
                self.save_file("Download member model (SBML)", &name, &[filter::SBML], xml.as_bytes());
            }
            Err(e) => self.status_error(format!("Could not export SBML: {e}")),
        }
    }

    /// `downloadCommunityCSV(communityId, file)`: only the manifest's medium
    /// and bounds files.
    pub fn download_community_csv(&mut self, community_id: &str, file: &str) {
        if !self.store.published_communities.iter().any(|c| c.get("id").and_then(Value::as_str) == Some(community_id)) {
            return;
        }
        let manifest = self.store.published_manifest(community_id);
        let allowed = ["media", "bounds"]
            .iter()
            .filter_map(|k| manifest.as_ref().and_then(|m| m.get(*k)).and_then(Value::as_str))
            .any(|f| f == file);
        if file.is_empty() || !allowed {
            return self.status_error("Could not export CSV: This community has no such CSV file.");
        }
        match self.store.assets.read(&format!("models/published/{community_id}/{file}")) {
            Ok(bytes) => {
                // `media.csv` → `steadycom-gut-nine_media.csv`.
                let name = file_names::community_data_file_name(community_id, file);
                self.save_file("Download CSV", &name, &[filter::CSV], &bytes);
            }
            Err(e) => self.status_error(format!("Could not export CSV: {e}")),
        }
    }
}

/// The Markdown of `downloadFindings`.
pub fn findings_markdown(c: &Value) -> String {
    let s = |v: Option<&Value>| v.map(coxeter_core::js::to_string).unwrap_or_default();
    let paper = c.get("paper");
    let p = |k: &str| s(paper.and_then(|p| p.get(k)).filter(|v| !v.is_null()));
    let rep = c.get("reproduction");
    let list = |k: &str| rep.and_then(|r| r.get(k)).and_then(Value::as_array).cloned().unwrap_or_default();
    let bullets = |items: &[Value]| {
        if items.is_empty() { "_none recorded_".to_string() } else { items.iter().map(|t| format!("- {}", s(Some(t)))).collect::<Vec<_>>().join("\n") }
    };
    let citation = paper.and_then(|p| p.get("citation")).filter(|v| !v.is_null()).map_or_else(|| p("journal"), |v| s(Some(v)));
    let verified = paper.and_then(|p| p.get("doiVerifiedFrom")).filter(|v| coxeter_core::js::truthy(v));
    let mut lines = vec![
        format!("# {}", s(c.get("name"))),
        String::new(),
        format!("**{}**", p("title")),
        format!("{} ({}), {}", p("authors"), p("year"), citation),
        format!("doi:{}{}", p("doi"), verified.map_or_else(|| " — citation unverified".to_string(), |v| format!(" — verified against {}", s(Some(v))))),
        String::new(),
        format!("Framework: {}  ·  Status: {}", p("framework"), s(rep.and_then(|r| r.get("status")))),
        String::new(),
        s(c.get("description")),
        String::new(),
        "## Reproduced paper findings".into(),
        String::new(),
        bullets(&list("reproduces")),
        String::new(),
        "## Adaptations and assumptions".into(),
        String::new(),
        bullets(&list("adaptations")),
    ];
    let needs = list("needs");
    if !needs.is_empty() {
        lines.extend([String::new(), "## Still required to reproduce".into(), String::new(), bullets(&needs)]);
    }
    if let Some(computed) = c.get("computed").filter(|v| coxeter_core::js::truthy(v)) {
        lines.extend([
            String::new(),
            "## Recorded result".into(),
            String::new(),
            "```json".into(),
            coxeter_core::js::stringify(computed, Some(2)),
            "```".into(),
        ]);
    }
    lines.push(String::new());
    lines.join("\n")
}
