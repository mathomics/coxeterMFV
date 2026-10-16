//! What the interface knows that the Store does not: the `ui` object of
//! `src/ui/app.js` (tabs, the open workspace, drafts of forms) and the
//! classes it toggles on `.app` (collapsed panels, focus mode, the results
//! dock).

use indexmap::IndexMap;

/// `ui.leftTab` (`data-tab` of `.panel-tabs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum LeftTab {
    /// Edit Map.
    #[default]
    Build,
    /// Edit Model.
    Model,
    /// Simulate.
    Analysis,
}

impl LeftTab {
    pub fn as_str(self) -> &'static str {
        match self {
            LeftTab::Build => "build",
            LeftTab::Model => "model",
            LeftTab::Analysis => "analysis",
        }
    }
    /// `['build', 'model', 'analysis'].includes(tab) ? tab : 'build'`.
    pub fn parse(s: &str) -> LeftTab {
        match s {
            "model" => LeftTab::Model,
            "analysis" => LeftTab::Analysis,
            _ => LeftTab::Build,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LeftTab::Build => "Edit Map",
            LeftTab::Model => "Edit Model",
            LeftTab::Analysis => "Simulate",
        }
    }
}

/// `ui.resultsTab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum ResultsTab {
    #[default]
    Analysis,
    Table,
    Logs,
}

impl ResultsTab {
    pub fn parse(s: &str) -> ResultsTab {
        match s {
            "table" => ResultsTab::Table,
            "logs" => ResultsTab::Logs,
            _ => ResultsTab::Analysis,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ResultsTab::Analysis => "analysis",
            ResultsTab::Table => "table",
            ResultsTab::Logs => "logs",
        }
    }
}

/// `ui.analysisTab` (`data-analysis-tab`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum AnalysisTab {
    #[default]
    Flux,
    Fva,
    Reduced,
    Scaled,
    Shadow,
    Repro,
}

impl AnalysisTab {
    pub const ALL: [AnalysisTab; 6] =
        [AnalysisTab::Flux, AnalysisTab::Fva, AnalysisTab::Reduced, AnalysisTab::Scaled, AnalysisTab::Shadow, AnalysisTab::Repro];
    pub fn parse(s: &str) -> AnalysisTab {
        AnalysisTab::ALL.into_iter().find(|t| t.as_str() == s).unwrap_or_default()
    }
    pub fn as_str(self) -> &'static str {
        match self {
            AnalysisTab::Flux => "flux",
            AnalysisTab::Fva => "fva",
            AnalysisTab::Reduced => "reduced",
            AnalysisTab::Scaled => "scaled",
            AnalysisTab::Shadow => "shadow",
            AnalysisTab::Repro => "repro",
        }
    }
}

/// `ui.rightTab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum RightTab {
    #[default]
    Inspector,
    Layout,
}

/// A workspace page (`data-workspace`; `WORKSPACES` of app.js, plus
/// `settings`, which `renderWorkspace` knows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Workspace {
    #[default]
    Library,
    Structure,
    Map,
    Scenario,
    Community,
    Medium,
    Model,
    Omics,
    Analysis,
    Settings,
    Crossfeeding,
    Overview,
}

impl Workspace {
    pub const ALL: [Workspace; 12] = [
        Workspace::Library,
        Workspace::Structure,
        Workspace::Map,
        Workspace::Scenario,
        Workspace::Community,
        Workspace::Medium,
        Workspace::Model,
        Workspace::Omics,
        Workspace::Analysis,
        Workspace::Settings,
        Workspace::Crossfeeding,
        Workspace::Overview,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Workspace::Library => "library",
            Workspace::Structure => "structure",
            Workspace::Map => "map",
            Workspace::Scenario => "scenario",
            Workspace::Community => "community",
            Workspace::Medium => "medium",
            Workspace::Model => "model",
            Workspace::Omics => "omics",
            Workspace::Analysis => "analysis",
            Workspace::Settings => "settings",
            Workspace::Crossfeeding => "crossfeeding",
            Workspace::Overview => "overview",
        }
    }

    /// The names `openWorkspace` accepts (`WORKSPACES`; `settings` is not one).
    pub fn parse(s: &str) -> Option<Workspace> {
        Workspace::ALL.into_iter().find(|w| w.as_str() == s && *w != Workspace::Settings)
    }

    /// `getWorkspaceTitle(workspace)` (`WORKSPACE_TITLES`).
    pub fn title(self) -> &'static str {
        match self {
            Workspace::Library => "Reaction library",
            Workspace::Structure => "Map structure",
            Workspace::Map => "Map options",
            Workspace::Scenario => "Scenario & objective",
            Workspace::Community => "Species & abundance",
            Workspace::Medium => "Shared medium",
            Workspace::Model => "Biological model",
            Workspace::Omics => "Omics & expression",
            Workspace::Analysis | Workspace::Settings => "Analysis settings",
            Workspace::Crossfeeding => "Cross-feeding explorer",
            Workspace::Overview => "Model overview",
        }
    }
}

/// One row of the flux colour editor as typed (`{ value, color }`).
#[derive(Debug, Clone, PartialEq)]
pub struct FluxColorRow {
    pub value: String,
    pub color: String,
}

/// `ui.fluxColorDraft`: the rows of the flux palette editor as the user left
/// them, with what was wrong with them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FluxColorDraft {
    pub rows: Vec<FluxColorRow>,
    pub error: String,
    pub invalid: Vec<usize>,
}

/// `ui.steadyComDraft`: the growth rate being typed, and the error shown
/// under the SteadyCom controls.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SteadyComDraft {
    pub community_id: Option<String>,
    pub key: Option<String>,
    pub value: String,
    pub error: String,
    /// `mu` or `fiber`: the field marked `aria-invalid`.
    pub invalid_field: Option<String>,
    pub fiber_value: Option<String>,
}

/// The values of a form's named fields (`[name]` inputs), as text; a
/// checkbox holds `"true"` or `"false"`. `ui.drafts` keeps one per form.
pub type FormValues = IndexMap<String, String>;

/// The interface's own state.
#[derive(Debug, Clone)]
pub struct UiState {
    pub left_tab: LeftTab,
    pub results_tab: ResultsTab,
    pub analysis_tab: AnalysisTab,
    /// The open workspace page, if any.
    pub workspace: Option<Workspace>,
    pub last_workspace: Workspace,
    pub right_tab: RightTab,
    /// Set once the user works the right panel by hand.
    pub inspector_pinned: bool,

    // The classes of `.app`
    pub left_collapsed: bool,
    pub focus_mode: bool,
    pub task_overlay: bool,
    pub inspector_collapsed: bool,
    /// `is-results-collapsed` (the page starts with it).
    pub results_collapsed: bool,
    pub results_maximized: bool,
    /// `--results-h`.
    pub results_height: f32,

    // Drafts
    pub flux_color_draft: Option<FluxColorDraft>,
    /// `ui.fluxColorSource`: the palette the draft was made from; another
    /// palette arriving (a map or scenario import) drops the draft.
    pub flux_color_source: Option<Option<Vec<coxeter_render::FluxColorStop>>>,
    pub flux_colors_committing: bool,
    pub steady_com_draft: Option<SteadyComDraft>,
    pub steady_com_pending: bool,
    /// `ui.drafts`: form values by draft key (`model:metabolite`, …).
    pub drafts: IndexMap<String, FormValues>,
    /// The error under the model editor (`.model-editor-error`).
    pub model_editor_error: Option<String>,
    /// The scenario chosen in the list (`select[data-action="scenario"]`),
    /// when it differs from the open one while it loads.
    pub scenario_choice: Option<String>,
    /// Put the keyboard in this control (an `egui::Id`): the panel that
    /// draws a control with this id calls `response.request_focus()` and
    /// clears it (see `Shell::take_focus_request`).
    pub focus_request: Option<egui::Id>,
    /// The inspector's lower-bound field should take the keyboard (a double
    /// click on a reaction).
    pub focus_lower_bound: bool,
    /// `details.canvas-view[open]`: the View popover of the map toolbar.
    pub view_open: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            left_tab: LeftTab::Build,
            results_tab: ResultsTab::Analysis,
            analysis_tab: AnalysisTab::Flux,
            workspace: None,
            last_workspace: Workspace::Library,
            right_tab: RightTab::Inspector,
            inspector_pinned: false,
            left_collapsed: false,
            focus_mode: false,
            task_overlay: false,
            inspector_collapsed: false,
            results_collapsed: true,
            results_maximized: false,
            results_height: 252.0,
            flux_color_draft: None,
            flux_color_source: None,
            flux_colors_committing: false,
            steady_com_draft: None,
            steady_com_pending: false,
            drafts: IndexMap::new(),
            model_editor_error: None,
            scenario_choice: None,
            focus_request: None,
            focus_lower_bound: false,
            view_open: false,
        }
    }
}
