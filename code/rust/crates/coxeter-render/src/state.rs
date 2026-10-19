//! What `renderMap(layout, state)` is told besides the map: every field the
//! JavaScript renderer destructures from its `state`, with the same defaults.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use coxeter_core::community::CrossFeeding;
use coxeter_core::mapdata::Point;

use crate::reaction_colors::{PathwayRibbonMode, ReactionColorMode};
use crate::theme::{FluxScale, Scale};

/// How wide reaction lines are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReactionWidthMode {
    /// Every reaction at the Escher width (7.5).
    #[default]
    Constant,
    /// Scaled by the magnitude of the displayed flux.
    Flux,
}

/// Which labels are written (`labels` in the JS state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelMode {
    /// Reactions that carry a large share of the flux, and every metabolite.
    #[default]
    Auto,
    /// Everything.
    All,
    /// What carries flux: active reactions and the metabolites drawn on them.
    Active,
    /// No label at all.
    None,
}

impl LabelMode {
    /// `auto`, `all`, `active` or `none`; anything else is `all`, as the
    /// renderer treats any value it does not test for.
    pub fn parse(s: &str) -> LabelMode {
        match s {
            "auto" => LabelMode::Auto,
            "active" => LabelMode::Active,
            "none" => LabelMode::None,
            _ => LabelMode::All,
        }
    }
}

/// The compartment (or the shared medium) selected for geometry editing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GeometrySelection {
    /// None for the shared medium.
    pub cell_id: Option<String>,
    pub compartment_id: String,
}

/// Objects selected together.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MultiSelection {
    pub nodes: HashSet<String>,
    pub reactions: HashSet<String>,
    pub texts: HashSet<String>,
    /// Whole compartments, as `(cell id, compartment id)`.
    pub compartments: HashSet<(String, String)>,
}

/// A flux variability range; missing or non-finite ends read "calculando…".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FvaRange {
    pub min: f64,
    pub max: f64,
}

/// One cross-feeding event of the explorer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CrossFeedingEvent {
    pub from: String,
    pub to: String,
    pub base: String,
    pub flux: f64,
    /// The producer's transfer reaction, if known.
    pub producer_transfer: Option<String>,
    /// The consumer's transfer reaction, if known.
    pub consumer_transfer: Option<String>,
}

/// What the cross-feeding explorer contributes to the map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CrossFeedingExplorerView {
    pub enabled: bool,
    pub visible_events: Vec<CrossFeedingEvent>,
    pub selected_event: Option<CrossFeedingEvent>,
    /// Where the picked line was clicked; its name tag is pinned there.
    pub selected_at: Option<Point>,
}

/// Everything `renderMap` reads from its `state`, with the JS defaults.
#[derive(Clone)]
pub struct RenderState {
    /// The fluxes drawn (colour, labels); may be differences between solutions.
    pub fluxes: HashMap<String, f64>,
    /// Actual flux directions when `fluxes` holds differences (`None`: `fluxes`).
    pub solved_fluxes: Option<HashMap<String, f64>>,
    pub reaction_width_mode: ReactionWidthMode,
    /// Reactions kept on the map with both bounds at zero (knockouts).
    pub disabled_reaction_ids: HashSet<String>,
    /// Context cells kept visible outside the current simulation.
    pub inactive_species_ids: HashSet<String>,
    pub inactive_species_label: String,
    /// Reactions labelled in `auto` mode whenever they carry flux.
    pub emphasized_reaction_ids: HashSet<String>,
    pub scale: Arc<dyn Scale + Send + Sync>,
    pub dark: bool,
    pub reaction_color_mode: ReactionColorMode,
    pub reaction_colors: HashMap<String, String>,
    pub pathway_ribbon_mode: PathwayRibbonMode,
    pub pathway_ribbon_colors: HashMap<String, String>,
    pub pathway_ribbon_groups: HashMap<String, String>,
    pub pathway_ribbon_width: f64,
    pub pathway_ribbon_opacity: f64,
    pub selected_reaction: Option<String>,
    pub selected_geometry: Option<GeometrySelection>,
    pub geometry_edit: bool,
    /// Show the control points of every drawn reaction.
    pub reaction_edit: bool,
    pub multi_selection: Option<MultiSelection>,
    pub fva_enabled: bool,
    pub fva_ranges: HashMap<String, FvaRange>,
    pub labels: LabelMode,
    pub show_currency: bool,
    pub currency_labels: bool,
    /// Label text per metabolite node, overriding its base label.
    pub metabolite_labels: HashMap<String, String>,
    /// Label text per reaction, overriding its id.
    pub reaction_labels: HashMap<String, String>,
    /// Species id → growth rate.
    pub growth: HashMap<String, f64>,
    pub cross_feeding: Option<CrossFeeding>,
    pub show_cross_feeding: bool,
    pub cross_feeding_explorer: Option<CrossFeedingExplorerView>,
    /// The exchanged metabolites whose lines are drawn (`None`: all).
    pub cross_feeding_bases: Option<HashSet<String>>,
    pub omics_enabled: bool,
    pub omics_colors: HashMap<String, String>,
    /// `None`: the scheme's grey (`#475569` dark, `#cbd5e1` light).
    pub omics_fallback_color: Option<String>,
    /// The smallest pointer target, in map units (`MIN_HIT_PX / zoom`).
    pub min_hit_radius: f64,
    /// The picture only: no selection, handles, pointer targets or canvas
    /// resize grips.
    pub for_export: bool,
    /// Draw reactions that carry no flux in the solution grey and dashed;
    /// set it only when there is a solution.
    pub off_style: bool,
    /// A line drawn under another is cut where they cross.
    pub line_hops: bool,
    /// Hover text for reactions and metabolites, written into the SVG.
    pub tooltips: Option<Arc<Tooltips>>,
}

/// `state.tooltips`: hover text by reaction id and by node id.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tooltips {
    pub reactions: HashMap<String, String>,
    pub nodes: HashMap<String, String>,
}

impl Default for RenderState {
    fn default() -> Self {
        RenderState {
            fluxes: HashMap::new(),
            solved_fluxes: None,
            reaction_width_mode: ReactionWidthMode::Constant,
            disabled_reaction_ids: HashSet::new(),
            inactive_species_ids: HashSet::new(),
            inactive_species_label: "inactive context".into(),
            emphasized_reaction_ids: HashSet::new(),
            scale: Arc::new(FluxScale::automatic([], false)),
            dark: false,
            reaction_color_mode: ReactionColorMode::Flux,
            reaction_colors: HashMap::new(),
            pathway_ribbon_mode: PathwayRibbonMode::Off,
            pathway_ribbon_colors: HashMap::new(),
            pathway_ribbon_groups: HashMap::new(),
            pathway_ribbon_width: 56.0,
            pathway_ribbon_opacity: 0.18,
            selected_reaction: None,
            selected_geometry: None,
            geometry_edit: false,
            reaction_edit: false,
            multi_selection: None,
            fva_enabled: false,
            fva_ranges: HashMap::new(),
            labels: LabelMode::Auto,
            show_currency: true,
            currency_labels: true,
            metabolite_labels: HashMap::new(),
            reaction_labels: HashMap::new(),
            growth: HashMap::new(),
            cross_feeding: None,
            show_cross_feeding: false,
            cross_feeding_explorer: None,
            cross_feeding_bases: None,
            omics_enabled: false,
            omics_colors: HashMap::new(),
            omics_fallback_color: None,
            min_hit_radius: 0.0,
            for_export: false,
            off_style: false,
            line_hops: false,
            tooltips: None,
        }
    }
}

impl std::fmt::Debug for RenderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderState")
            .field("fluxes", &self.fluxes.len())
            .field("dark", &self.dark)
            .field("labels", &self.labels)
            .field("for_export", &self.for_export)
            .finish_non_exhaustive()
    }
}
