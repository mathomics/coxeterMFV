//! Cross-feeding on the map and the cross-feeding explorer (the
//! `crossFeeding*` methods of `store.js`).
//!
//! The explorer's event index and the ranking of exchanged metabolites are
//! `src/analysis/cross-feeding-explorer.js`, ported in `coxeter-analysis`;
//! this module holds the explorer's state and the store methods around it.

use std::collections::HashSet;

use coxeter_analysis::{CrossFeedingFilter, CrossFeedingOptions, build_cross_feeding_index};
use coxeter_core::mapdata::Point;
use coxeter_render::crossfeed_routes::is_drawn_cross_feeding_base;

pub use coxeter_analysis::{CROSS_FEEDING_GROUPS, CrossFeedingIndex, ExchangedMetabolite, exchanged_metabolites};

use crate::store::Store;
use crate::types::ViewPatch;

/// One event of the explorer: `from` feeds `to` with `base`.
pub type ExplorerEvent = coxeter_analysis::CrossFeedingEvent;

/// `store.crossFeedingExplorer`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CrossFeedingExplorer {
    pub enabled: bool,
    pub index: Option<CrossFeedingIndex>,
    pub query: String,
    pub metabolite: String,
    pub producer: String,
    pub consumer: String,
    pub min_flux: f64,
    pub selected_id: Option<String>,
    pub selected_at: Option<Point>,
}

/// A `setCrossFeedingExplorer(options)`: fields left `None` are left alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExplorerPatch {
    pub enabled: Option<bool>,
    pub query: Option<String>,
    pub metabolite: Option<String>,
    pub producer: Option<String>,
    pub consumer: Option<String>,
    pub min_flux: Option<f64>,
}

/// `filterCrossFeedingIndex(index, filters)` with the explorer's own filters.
pub fn filter_cross_feeding_index(explorer: &CrossFeedingExplorer) -> Vec<ExplorerEvent> {
    let filter = CrossFeedingFilter {
        query: explorer.query.clone(),
        metabolite: explorer.metabolite.clone(),
        producer: explorer.producer.clone(),
        consumer: explorer.consumer.clone(),
        min_flux: explorer.min_flux,
    };
    coxeter_analysis::filter_cross_feeding_index(explorer.index.as_ref(), &filter).into_iter().cloned().collect()
}

impl Store {
    /// `refreshCrossFeedingExplorer()`: the event index of the current
    /// solution. A SteadyCom or joint solution hands over the cross-feeding
    /// it computed itself (`raw`).
    pub fn refresh_cross_feeding_explorer(&mut self) {
        let Some(solution) = self.solution.clone().filter(|s| s.is_optimal()) else {
            self.cross_feeding_explorer.index = None;
            return;
        };
        let raw = (solution.steady_com.is_some() || self.joint_projection_spec.is_some()).then(|| self.cross_feeding.clone());
        let options = CrossFeedingOptions { threshold: 1e-6, raw };
        let Some(community) = self.community_view() else {
            self.cross_feeding_explorer.index = None;
            return;
        };
        let index = build_cross_feeding_index(&community, &solution.fluxes, &options);
        self.cross_feeding_explorer.index = Some(index);
        let visible = self.cross_feeding_explorer_events();
        if let Some(id) = &self.cross_feeding_explorer.selected_id {
            if !visible.iter().any(|e| &e.id == id) {
                self.cross_feeding_explorer.selected_id = None;
            }
        }
    }

    /// `crossFeedingExplorerEvents()`: the events the explorer's filters let
    /// through.
    pub fn cross_feeding_explorer_events(&self) -> Vec<ExplorerEvent> {
        filter_cross_feeding_index(&self.cross_feeding_explorer)
    }

    /// `setCrossFeedingExplorer(options)`.
    pub fn set_cross_feeding_explorer(&mut self, patch: ExplorerPatch) {
        let e = &mut self.cross_feeding_explorer;
        if let Some(v) = patch.enabled {
            e.enabled = v;
        }
        if let Some(v) = patch.query {
            e.query = v;
        }
        if let Some(v) = patch.metabolite {
            e.metabolite = v;
        }
        if let Some(v) = patch.producer {
            e.producer = v;
        }
        if let Some(v) = patch.consumer {
            e.consumer = v;
        }
        if let Some(v) = patch.min_flux {
            e.min_flux = v;
        }
        if patch.enabled == Some(true) || self.cross_feeding_explorer.index.is_none() {
            self.refresh_cross_feeding_explorer();
        }
        self.view.show_cross_feeding = true;
        self.emit("cross-feeding-explorer");
    }

    /// `toggleCrossFeedingLink({ from, base, to }, at)`: pick the event a
    /// cross-feeding line on the map stands for, with the map point that was
    /// clicked (the name tag is pinned there). Picking the picked line again
    /// unpicks it.
    pub fn toggle_cross_feeding_link(&mut self, from: &str, base: &str, to: &str, at: Option<Point>) -> Option<ExplorerEvent> {
        if self.cross_feeding_explorer.index.is_none() {
            self.refresh_cross_feeding_explorer();
        }
        let event = self
            .cross_feeding_explorer
            .index
            .as_ref()?
            .events
            .iter()
            .find(|e| e.from == from && e.base == base && e.to == to)?
            .clone();
        if self.cross_feeding_explorer.selected_id.as_deref() == Some(event.id.as_str()) {
            self.clear_cross_feeding_pick();
            return None;
        }
        self.cross_feeding_explorer.selected_at = at;
        self.select_cross_feeding_event(&event.id)
    }

    /// `crossFeedingMetabolites()`: the metabolites species pass to each
    /// other, most important first (protons and water left out).
    pub fn cross_feeding_metabolites(&self) -> Vec<ExchangedMetabolite> {
        exchanged_metabolites(&self.cross_feeding.links, |base| !is_drawn_cross_feeding_base(base))
    }

    /// `crossFeedingVisibleBases()`: the bases whose cross-feeding lines are
    /// drawn.
    pub fn cross_feeding_visible_bases(&self) -> indexmap::IndexSet<String> {
        if let Some(shown) = &self.view.cross_feeding_shown {
            return shown.iter().cloned().collect();
        }
        let all = self.cross_feeding_metabolites();
        let top = self.view.cross_feeding_top;
        let take = if top > 0.0 { (top as usize).min(all.len()) } else { all.len() };
        all.into_iter().take(take).map(|e| e.base).collect()
    }

    /// `setCrossFeedingTop(n)`: draw the `n` most important exchanged
    /// metabolites (0: all of them).
    pub fn set_cross_feeding_top(&mut self, n: f64) {
        let top = coxeter_core::js::max(0.0, coxeter_core::js::round(if n.is_nan() { 0.0 } else { n }));
        let _ = self.set_view(ViewPatch { cross_feeding_top: Some(top), cross_feeding_shown: Some(None), ..Default::default() });
    }

    /// `toggleCrossFeedingMetabolite(base, on)`: tick or untick one
    /// metabolite; from then on the list is the user's own.
    pub fn toggle_cross_feeding_metabolite(&mut self, base: &str, on: bool) {
        let mut shown = self.cross_feeding_visible_bases();
        if on {
            shown.insert(base.to_string());
        } else {
            shown.shift_remove(base);
        }
        let _ = self.set_view(ViewPatch { cross_feeding_shown: Some(Some(shown.into_iter().collect())), ..Default::default() });
    }

    /// `clearCrossFeedingPick()`: unpick the picked event, leaving the
    /// explorer as it is.
    pub fn clear_cross_feeding_pick(&mut self) {
        if self.cross_feeding_explorer.selected_id.is_none() {
            return;
        }
        self.cross_feeding_explorer.selected_id = None;
        self.cross_feeding_explorer.selected_at = None;
        self.emit("cross-feeding-explorer");
    }

    /// `selectCrossFeedingEvent(id)`.
    pub fn select_cross_feeding_event(&mut self, id: &str) -> Option<ExplorerEvent> {
        self.cross_feeding_explorer.enabled = true;
        self.cross_feeding_explorer.selected_id = Some(id.to_string());
        let event = self.cross_feeding_explorer.index.as_ref().and_then(|i| i.events.iter().find(|e| e.id == id)).cloned();
        if let Some(t) = event.as_ref().and_then(|e| e.producer_transfer.clone()) {
            self.selected_reaction = Some(t);
        }
        self.emit("cross-feeding-explorer");
        event
    }

    /// The bases drawn, as the renderer wants them.
    pub(crate) fn visible_bases_set(&self) -> HashSet<String> {
        self.cross_feeding_visible_bases().into_iter().collect()
    }
}
