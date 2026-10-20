//! What the renderer is told besides the map (`mapRenderState()` of
//! `src/ui/app.js`, which reads only store state).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use coxeter_render::state::{
    CrossFeedingEvent, CrossFeedingExplorerView, FvaRange as RenderFvaRange, LabelMode, MultiSelection, ReactionWidthMode, RenderState,
};
use coxeter_render::theme::FluxScale;

use crate::cross_feeding::ExplorerEvent;
use crate::store::Store;

/// The two things `mapRenderState` reads from the interface rather than the
/// store.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderOptions {
    /// `ui.tool === 'edit'`: show the control points of every drawn reaction.
    pub reaction_edit: bool,
    /// `MIN_HIT_PX / camera.scale`: the smallest pointer target in map units.
    pub min_hit_radius: f64,
}

fn event(e: &ExplorerEvent) -> CrossFeedingEvent {
    CrossFeedingEvent {
        from: e.from.clone(),
        to: e.to.clone(),
        base: e.base.clone(),
        flux: e.flux,
        producer_transfer: e.producer_transfer.clone(),
        consumer_transfer: e.consumer_transfer.clone(),
    }
}

impl Store {
    /// `mapRenderState()`: everything the renderer needs to draw the map as
    /// it stands.
    pub fn map_render_state(&self, opts: RenderOptions) -> RenderState {
        let reaction_coloring = self.reaction_coloring(false);
        let ribbon_coloring = self.pathway_ribbon_coloring(false);
        let omics_overlay = self.omics_overlay();
        let community_omics = self.omics.enabled && self.omics.dataset.is_some();
        // Both imports retain their own settings. Reaction overlays take
        // precedence for matching reactions; community expression fills the
        // remaining colours.
        let mut omics_colors: HashMap<String, String> =
            if community_omics { self.omics.colors.iter().map(|(k, v)| (k.clone(), v.clone())).collect() } else { HashMap::new() };
        if omics_overlay.enabled {
            omics_colors.extend(omics_overlay.colors.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let zuniga = self.is_zuniga_study();
        let mut inactive: HashSet<String> = HashSet::new();
        if zuniga && self.zuniga.scenario_id.as_deref() != Some("mutant-coculture") {
            inactive.insert("Se".into());
        }
        inactive.extend(self.disabled_drawn_species_ids());
        let explorer = &self.cross_feeding_explorer;
        let selected_event = explorer
            .index
            .as_ref()
            .and_then(|i| i.events.iter().find(|e| Some(&e.id) == explorer.selected_id.as_ref()))
            .map(event);
        let scale = self.scale.clone().unwrap_or_else(|| FluxScale::automatic([], self.view.dark));
        let to_map = |m: &indexmap::IndexMap<String, String>| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<HashMap<_, _>>();
        // Bounds changed from the model's: knocked-out reactions take the
        // disabled style, the others a badge.
        let changes = self.constraint_changes();
        let mut disabled: HashSet<String> = if zuniga { self.zuniga_knockouts().into_iter().collect() } else { HashSet::new() };
        disabled.extend(changes.iter().filter(|c| c.knocked_out).map(|c| c.id.clone()));
        let modified: HashSet<String> = changes.iter().filter(|c| !c.knocked_out).map(|c| c.id.clone()).collect();
        RenderState {
            fluxes: self.display_fluxes().into_iter().collect(),
            solved_fluxes: Some(
                self.solution.as_ref().filter(|s| s.is_optimal()).map(|s| s.fluxes.clone().into_iter().collect()).unwrap_or_default(),
            ),
            reaction_width_mode: if zuniga || self.view.width_by_flux { ReactionWidthMode::Flux } else { ReactionWidthMode::Constant },
            disabled_reaction_ids: disabled,
            modified_reaction_ids: modified,
            inactive_species_ids: inactive,
            inactive_species_label: if zuniga { "inactive context" } else { "switched off" }.into(),
            emphasized_reaction_ids: if zuniga {
                ["Se@PHELtex", "Ec@PHEtex", "Ec@PHEt2rpp"].iter().map(|s| s.to_string()).collect()
            } else {
                HashSet::new()
            },
            scale: Arc::new(scale),
            dark: self.view.dark,
            reaction_color_mode: self.view.reaction_color_mode,
            reaction_colors: to_map(&reaction_coloring.colors),
            pathway_ribbon_mode: self.view.pathway_ribbon_mode,
            pathway_ribbon_colors: to_map(&ribbon_coloring.colors),
            pathway_ribbon_groups: to_map(&ribbon_coloring.groups),
            pathway_ribbon_width: self.view.pathway_ribbon_width,
            pathway_ribbon_opacity: self.view.pathway_ribbon_opacity,
            selected_reaction: self.selected_reaction.clone(),
            selected_geometry: self.selected_geometry.clone(),
            geometry_edit: self.view.geometry_edit,
            reaction_edit: opts.reaction_edit,
            multi_selection: Some(MultiSelection {
                nodes: self.multi_selection.nodes.iter().cloned().collect(),
                reactions: self.multi_selection.reactions.iter().cloned().collect(),
                texts: self.multi_selection.texts.iter().cloned().collect(),
                compartments: self.multi_selection.compartments.iter().cloned().collect(),
            }),
            fva_enabled: self.fva_enabled,
            fva_ranges: self.fva_ranges.iter().map(|(k, r)| (k.clone(), RenderFvaRange { min: r.min, max: r.max })).collect(),
            labels: LabelMode::parse(&self.view.labels),
            show_currency: self.view.show_currency,
            currency_labels: self.view.currency_labels || self.map_mode == "core-overview",
            metabolite_labels: self.map_labels("metabolite").into_iter().collect(),
            reaction_labels: self.map_labels("reaction").into_iter().collect(),
            growth: self.growth.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            cross_feeding: Some(self.cross_feeding.clone()),
            show_cross_feeding: self.view.show_cross_feeding,
            cross_feeding_explorer: Some(CrossFeedingExplorerView {
                enabled: explorer.enabled,
                visible_events: self.cross_feeding_explorer_events().iter().map(event).collect(),
                selected_event,
                selected_at: explorer.selected_at,
            }),
            cross_feeding_bases: self.view.show_cross_feeding.then(|| self.visible_bases_set()),
            cross_feeding_width: self.view.cross_feeding_width,
            omics_enabled: omics_overlay.enabled || community_omics,
            omics_colors,
            omics_fallback_color: Some(omics_overlay.fallback_color),
            min_hit_radius: opts.min_hit_radius,
            for_export: false,
            // Grey and dashed only against a solution: with none, nothing carries flux.
            off_style: self.view.dash_no_flux && self.solution.as_ref().is_some_and(|s| s.is_optimal()),
            line_hops: self.view.line_hops,
            tooltips: None,
        }
    }
}
