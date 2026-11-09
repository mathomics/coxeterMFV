//! Presentation: the fluxes shown, the flux colour scale and `setView`.
//! Nothing here changes the model or solves an LP.

use coxeter_render::reaction_colors::{normalize_pathway_ribbon_mode, normalize_reaction_color_mode};
use coxeter_render::theme::{
    FluxColorStop, FluxScale, FluxScaleOptions, normalize_absolute_flux_color_stops, normalize_absolute_flux_colors_value,
    normalize_flux_color_stops,
};
use serde_json::{Value, json};
use indexmap::IndexMap;

use crate::error::Result;
use crate::store::Store;
use crate::types::ViewPatch;
use crate::util::clamp_number;

impl Store {
    /// `displayFluxes()`: the fluxes the map shows. They are the solution's,
    /// or in the Zuñiga case's difference mode, the difference from the WT
    /// monoculture.
    pub fn display_fluxes(&self) -> IndexMap<String, f64> {
        let actual = match self.solution.as_ref().filter(|s| s.is_optimal()) {
            Some(s) => s.fluxes.clone(),
            None => IndexMap::new(),
        };
        if !self.is_zuniga_study() || self.zuniga.comparison != "difference" {
            return actual;
        }
        let baseline = self.zuniga.solutions.get("wt-monoculture").map(|s| s.fluxes.clone()).unwrap_or_default();
        let mut ids: indexmap::IndexSet<String> = actual.keys().cloned().collect();
        ids.extend(baseline.keys().cloned());
        ids.into_iter()
            .map(|id| {
                let v = actual.get(&id).copied().unwrap_or(0.0) - baseline.get(&id).copied().unwrap_or(0.0);
                (id, v)
            })
            .collect()
    }

    /// `refreshFluxScale()`: refresh presentation only.
    pub fn refresh_flux_scale(&mut self) -> &FluxScale {
        let fluxes = self.display_fluxes();
        let reference = self.zuniga_state().get("fluxReference").and_then(serde_json::Value::as_f64);
        let difference = self.is_zuniga_study() && self.zuniga.comparison == "difference";
        let stops = match (difference, reference) {
            (true, Some(r)) => Some(vec![
                FluxColorStop { value: -r, color: "#2166ac".into() },
                FluxColorStop { value: 0.0, color: "#bdbdbd".into() },
                FluxColorStop { value: r, color: "#b2182b".into() },
            ]),
            _ => self.view.flux_color_stops.clone(),
        };
        let dark = self.view.dark;
        let options = FluxScaleOptions {
            color_stops: stops,
            absolute_stops: self.view.absolute_flux_color_stops.clone(),
            absolute_log: self.view.absolute_flux_log,
            dark,
            ..Default::default()
        };
        let mut scale = FluxScale::new(fluxes.values().copied(), options)
            .unwrap_or_else(|_| FluxScale::automatic(fluxes.values().copied(), dark));
        if self.is_zuniga_study() {
            if let Some(r) = reference {
                scale.reference = r;
            }
        }
        self.scale = Some(scale);
        self.scale.as_ref().expect("just set")
    }

    /// `setFluxColorStops(stops)`: a custom palette (`None`: automatic).
    pub fn set_flux_color_stops(&mut self, stops: Option<Vec<FluxColorStop>>) -> Result<Option<Vec<FluxColorStop>>> {
        self.set_view(ViewPatch { flux_color_stops: Some(stops), ..Default::default() })?;
        Ok(self.view.flux_color_stops.clone())
    }

    /// `absoluteFluxColors()`: the Absolute flux palette as files keep it.
    pub fn absolute_flux_colors_json(&self) -> Value {
        json!({
            "stops": crate::pathways::flux_color_stops_json(self.view.absolute_flux_color_stops.as_deref()),
            "log": self.view.absolute_flux_log,
        })
    }

    /// `restoreAbsoluteFluxColors(value)`: the Absolute flux palette from a
    /// file; one without it gets the default.
    pub fn restore_absolute_flux_colors(&mut self, value: &Value) -> Result<()> {
        let (stops, log) = normalize_absolute_flux_colors_value(value)?;
        self.view.absolute_flux_color_stops = stops;
        self.view.absolute_flux_log = log;
        Ok(())
    }

    /// `setView(patch)`. An invalid flux palette is refused before anything
    /// changes.
    pub fn set_view(&mut self, patch: ViewPatch) -> Result<()> {
        let stops = match &patch.flux_color_stops {
            Some(Some(list)) => Some(Some(normalize_flux_color_stops(list)?)),
            Some(None) => Some(None),
            None => None,
        };
        let absolute_stops = match &patch.absolute_flux_color_stops {
            Some(Some(list)) => Some(Some(normalize_absolute_flux_color_stops(list)?)),
            Some(None) => Some(None),
            None => None,
        };
        let v = &mut self.view;
        if let Some(x) = patch.labels {
            v.labels = x;
        }
        if let Some(x) = patch.scroll_input {
            v.scroll_input = if ["auto", "mouse", "touchpad"].contains(&x.as_str()) { x } else { "auto".into() };
        }
        if let Some(x) = stops.clone() {
            v.flux_color_stops = x;
        }
        if let Some(x) = absolute_stops.clone() {
            v.absolute_flux_color_stops = x;
        }
        if let Some(x) = patch.absolute_flux_log {
            v.absolute_flux_log = x;
        }
        if let Some(x) = &patch.reaction_color_mode {
            v.reaction_color_mode = normalize_reaction_color_mode(x);
        }
        if let Some(x) = patch.width_by_flux {
            v.width_by_flux = x;
        }
        if let Some(x) = patch.dash_no_flux {
            v.dash_no_flux = x;
        }
        if let Some(x) = patch.line_hops {
            v.line_hops = x;
        }
        if let Some(x) = &patch.line_color_group {
            v.line_color_group = x.clone();
        }
        if let Some(x) = &patch.pathway_ribbon_mode {
            v.pathway_ribbon_mode = normalize_pathway_ribbon_mode(x);
        }
        if let Some(x) = patch.pathway_ribbon_width {
            v.pathway_ribbon_width = clamp_number(x, 20.0, 120.0, 56.0);
        }
        if let Some(x) = patch.pathway_ribbon_opacity {
            v.pathway_ribbon_opacity = clamp_number(x, 0.04, 0.45, 0.18);
        }
        if let Some(x) = patch.show_currency {
            v.show_currency = x;
        }
        if let Some(x) = patch.currency_labels {
            v.currency_labels = x;
        }
        if let Some(x) = patch.show_cross_feeding {
            v.show_cross_feeding = x;
        }
        if let Some(x) = patch.cross_feeding_top {
            v.cross_feeding_top = x;
        }
        if let Some(x) = patch.cross_feeding_shown {
            v.cross_feeding_shown = x;
        }
        if let Some(x) = patch.cross_feeding_width {
            v.cross_feeding_width = cross_feeding_width_value(x, v.cross_feeding_width);
        }
        if let Some(x) = patch.dark {
            v.dark = x;
        }
        if let Some(x) = patch.geometry_edit {
            v.geometry_edit = x;
        }
        if let Some(x) = patch.multi_select {
            v.multi_select = x;
        }
        if let Some(x) = patch.text_tool {
            v.text_tool = x;
        }
        match patch.geometry_edit {
            Some(false) => self.selected_geometry = None,
            Some(true) => {
                self.selected_reaction = None;
                self.selected_entity = None;
                self.variability = None;
            }
            None => {}
        }
        if patch.dark.is_some() || stops.is_some() || absolute_stops.is_some() || patch.absolute_flux_log.is_some() {
            self.refresh_flux_scale();
        }
        self.emit("view");
        Ok(())
    }
}

/// `crossFeedingWidthValue(value, fallback)`: a cross-feeding line width
/// multiple inside the slider's range, or `fallback` for one that is not a number.
pub(crate) fn cross_feeding_width_value(value: f64, fallback: f64) -> f64 {
    let range = &coxeter_render::crossfeed_routes::CROSSFEED_WIDTH_RANGE;
    crate::util::clamp_number(value, range.min, range.max, fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blue() -> Vec<FluxColorStop> {
        vec![FluxColorStop { value: 0.0, color: "#ffffff".into() }, FluxColorStop { value: 10.0, color: "#0000ff".into() }]
    }

    #[test]
    fn scenario_and_map_files_carry_the_absolute_palette() {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled files");
        store.use_scenario("solo-ecoli").expect("scenario");
        let patch = ViewPatch {
            reaction_color_mode: Some("absflux".into()),
            absolute_flux_color_stops: Some(Some(blue())),
            absolute_flux_log: Some(true),
            ..Default::default()
        };
        store.set_view(patch).expect("a valid palette");
        let scale = store.scale.as_ref().expect("a scale");
        assert!(scale.absolute_log);
        assert_eq!(scale.absolute_color(10.0), "#0000ff");
        let negative = vec![FluxColorStop { value: -1.0, color: "#ffffff".into() }];
        assert!(store.set_view(ViewPatch { absolute_flux_color_stops: Some(Some(negative)), ..Default::default() }).is_err());
        assert_eq!(store.view.absolute_flux_color_stops, Some(blue()), "a refused edit keeps the last palette");

        let mut scenario = store.state_value();
        assert_eq!(normalize_absolute_flux_colors_value(&scenario["absoluteFluxColors"]), Ok((Some(blue()), true)));
        let map: Value = serde_json::from_str(&store.map_json().expect("a map")).expect("json");
        assert_eq!(normalize_absolute_flux_colors_value(&map["absoluteFluxColors"]), Ok((Some(blue()), true)));

        let mut restored = Store::with_dir(coxeter_core::testkit::repo_root());
        restored.init().expect("bundled files");
        restored.use_scenario("solo-ecoli").expect("scenario");
        restored.load_state(&scenario.to_string()).expect("scenario file");
        assert_eq!((restored.view.absolute_flux_color_stops.clone(), restored.view.absolute_flux_log), (Some(blue()), true));
        scenario.as_object_mut().expect("object").remove("absoluteFluxColors");
        restored.load_state(&scenario.to_string()).expect("older scenario file");
        assert_eq!((restored.view.absolute_flux_color_stops.clone(), restored.view.absolute_flux_log), (None, false));

        restored.load_map_json(&map.to_string()).expect("map file");
        assert_eq!((restored.view.absolute_flux_color_stops.clone(), restored.view.absolute_flux_log), (Some(blue()), true));
        assert!(restored.scale.as_ref().is_some_and(|s| s.absolute_log));
    }
}
