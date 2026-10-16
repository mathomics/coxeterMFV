//! The pathway manager and reaction colouring (the "pathway manager"
//! section of `store.js`).

use std::collections::HashSet;

use coxeter_core::colors::normalize_hex_color;
use coxeter_core::js;
use coxeter_render::reaction_colors::{
    PathwayRibbonMode, ReactionColorMode, ReactionColoring, build_reaction_coloring, category_color, is_missing_group,
};
use coxeter_render::theme::FluxColorStop;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};

use crate::error::{Result, StoreError};
use crate::store::Store;
use crate::types::*;
use crate::util::{js_trim, num_json, unique_pathway_id};

/// `fluxColorStops` as a file holds them (`null`: automatic).
pub fn flux_color_stops_json(stops: Option<&[FluxColorStop]>) -> Value {
    match stops {
        None => Value::Null,
        Some(list) => Value::Array(list.iter().map(|s| json!({"value": num_json(s.value), "color": s.color})).collect()),
    }
}

/// `fluxColorStops` from a file: the stops of a well-formed list, else none.
/// Validation is `normalizeFluxColorStops`, which the caller applies.
pub fn flux_color_stops_from_json(v: &Value) -> Option<Vec<FluxColorStop>> {
    let list = v.as_array()?;
    Some(
        list.iter()
            .map(|s| FluxColorStop {
                value: s.get("value").and_then(Value::as_f64).unwrap_or(f64::NAN),
                color: s.get("color").and_then(Value::as_str).unwrap_or("").to_string(),
            })
            .collect(),
    )
}

fn coloring(c: ReactionColoring) -> Coloring {
    Coloring {
        mode: c.mode.as_str().to_string(),
        colors: c.colors,
        groups: c.groups,
        legend: c.legend.into_iter().map(|l| LegendItem { label: l.label, count: l.count, color: l.color.to_string() }).collect(),
    }
}

/// `legendFromManagedGroups(groups, colors, reactionIds)`.
fn legend_from_managed_groups(
    groups: &IndexMap<String, String>,
    colors: &IndexMap<String, String>,
    reaction_ids: Option<&HashSet<String>>,
) -> Vec<LegendItem> {
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    let mut swatches: IndexMap<String, String> = IndexMap::new();
    for (rid, group) in groups {
        if reaction_ids.is_some_and(|ids| !ids.contains(rid)) {
            continue;
        }
        if group.is_empty() || group.eq_ignore_ascii_case("Sin pathway") {
            continue;
        }
        *counts.entry(group.clone()).or_insert(0) += 1;
        if !swatches.contains_key(group) {
            if let Some(c) = colors.get(rid) {
                swatches.insert(group.clone(), c.clone());
            }
        }
    }
    let mut legend: Vec<LegendItem> = counts
        .into_iter()
        .map(|(label, count)| LegendItem { color: swatches.get(&label).cloned().unwrap_or_else(|| "#64748b".into()), label, count })
        .collect();
    legend.sort_by(|a, b| js::locale_compare(&a.label, &b.label));
    legend
}

/// A pathway patch (`updateManualPathway(id, patch)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathwayPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub visible: Option<bool>,
}

impl Store {
    /// `pathwayManagerState()`: what the map and scenario files keep.
    pub fn pathway_manager_state(&self) -> Value {
        let manual: Vec<Value> = self
            .manual_pathways
            .values()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "color": p.color, "visible": p.visible,
                    "reactionIds": p.reaction_ids.iter().collect::<Vec<_>>(),
                })
            })
            .collect();
        let settings: Vec<Value> = self
            .pathway_settings
            .iter()
            .map(|(name, value)| {
                let mut o = Map::new();
                o.insert("name".into(), json!(name));
                for (k, v) in &value.0 {
                    o.insert(k.clone(), v.clone());
                }
                Value::Object(o)
            })
            .collect();
        json!({ "manual": manual, "settings": settings })
    }

    /// `restorePathwayManagerState(state)`.
    pub fn restore_pathway_manager_state(&mut self, state: Option<&Value>) {
        self.manual_pathways = IndexMap::new();
        self.pathway_settings = IndexMap::new();
        let Some(state) = state.filter(|s| s.is_object()) else { return };
        let text = |v: Option<&Value>| match v {
            None | Some(Value::Null) => String::new(),
            Some(v) => js_trim(&js::to_string(v)).to_string(),
        };
        let color_of = |v: Option<&Value>| v.and_then(Value::as_str).and_then(normalize_hex_color);
        for raw in state.get("manual").and_then(Value::as_array).cloned().unwrap_or_default() {
            let id = text(raw.get("id"));
            let name = text(raw.get("name"));
            if id.is_empty() || name.is_empty() {
                continue;
            }
            let reaction_ids: IndexSet<String> = raw
                .get("reactionIds")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .filter(|r| self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(*r)))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if reaction_ids.is_empty() {
                continue;
            }
            let color = color_of(raw.get("color")).unwrap_or_else(|| category_color(&name, self.view.dark).to_string());
            let visible = raw.get("visible") != Some(&Value::Bool(false));
            self.manual_pathways.insert(id.clone(), ManualPathway { id, name, color, visible, reaction_ids });
        }
        for raw in state.get("settings").and_then(Value::as_array).cloned().unwrap_or_default() {
            let name = text(raw.get("name"));
            if name.is_empty() {
                continue;
            }
            let mut setting = Map::new();
            setting.insert("visible".into(), json!(raw.get("visible") != Some(&Value::Bool(false))));
            if let Some(c) = color_of(raw.get("color")) {
                setting.insert("color".into(), json!(c));
            }
            self.pathway_settings.insert(name, PathwaySetting(setting));
        }
    }

    /// `createManualPathway({ name, color, reactionIds })`: a pathway drawn by
    /// hand, from the given reactions or the selected ones.
    pub fn create_manual_pathway(&mut self, name: &str, color: Option<&str>, reaction_ids: Option<&[String]>) -> Result<ManualPathway> {
        let label = js_trim(name).to_string();
        if label.is_empty() {
            return Err(StoreError::msg("Enter a pathway name."));
        }
        let selected: Vec<String> = match reaction_ids {
            Some(ids) => ids.to_vec(),
            None => self.multi_selection.reactions.iter().cloned().collect(),
        };
        let valid: IndexSet<String> = selected
            .into_iter()
            .filter(|r| self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(r)))
            .collect();
        if valid.is_empty() {
            return Err(StoreError::msg("Select at least one reaction."));
        }
        // A manual annotation is exclusive: assigning a reaction to a new
        // manual pathway removes it from the previous manual pathway, while
        // the original model pathway metadata remains untouched underneath.
        for pathway in self.manual_pathways.values_mut() {
            for rid in &valid {
                pathway.reaction_ids.shift_remove(rid);
            }
        }
        self.manual_pathways.retain(|_, p| !p.reaction_ids.is_empty());
        let id = unique_pathway_id(&label, |id| self.manual_pathways.contains_key(id));
        let color = color.and_then(normalize_hex_color).unwrap_or_else(|| category_color(&label, self.view.dark).to_string());
        let pathway = ManualPathway { id: id.clone(), name: label, color, visible: true, reaction_ids: valid };
        self.manual_pathways.insert(id, pathway.clone());
        self.view.pathway_ribbon_mode = PathwayRibbonMode::Pathway;
        self.view.reaction_color_mode = ReactionColorMode::Flux;
        self.emit("pathways");
        Ok(pathway)
    }

    /// `updateManualPathway(id, patch)`.
    pub fn update_manual_pathway(&mut self, id: &str, patch: PathwayPatch) -> Result<Option<ManualPathway>> {
        let Some(pathway) = self.manual_pathways.get(id) else { return Ok(None) };
        let name = match &patch.name {
            None => pathway.name.clone(),
            Some(n) => js_trim(n).to_string(),
        };
        if name.is_empty() {
            return Err(StoreError::msg("The pathway name cannot be empty."));
        }
        let pathway = self.manual_pathways.get_mut(id).expect("checked");
        pathway.name = name;
        if let Some(c) = patch.color.as_deref().and_then(normalize_hex_color) {
            pathway.color = c;
        }
        if let Some(v) = patch.visible {
            pathway.visible = v;
        }
        let out = pathway.clone();
        self.emit("pathways");
        Ok(Some(out))
    }

    /// `deleteManualPathway(id)`.
    pub fn delete_manual_pathway(&mut self, id: &str) -> bool {
        if self.manual_pathways.shift_remove(id).is_none() {
            return false;
        }
        self.emit("pathways");
        true
    }

    fn model_pathway_coloring(&self, reaction_ids: Option<&HashSet<String>>) -> ReactionColoring {
        build_reaction_coloring(self.community.as_ref().map(|c| &c.merged), ReactionColorMode::Pathway, reaction_ids, self.view.dark)
    }

    /// `pathwayReactionIds(key)`: the drawn reactions of `manual:<id>` or
    /// `model:<name>`.
    pub fn pathway_reaction_ids(&self, key: &str) -> Vec<String> {
        let drawn = |rid: &str| self.map.as_ref().is_some_and(|m| m.has_reaction(rid));
        if let Some(id) = key.strip_prefix("manual:") {
            return self
                .manual_pathways
                .get(id)
                .map(|p| p.reaction_ids.iter().filter(|r| drawn(r)).cloned().collect())
                .unwrap_or_default();
        }
        if let Some(name) = key.strip_prefix("model:") {
            let base = self.model_pathway_coloring(None);
            return base.groups.iter().filter(|(rid, g)| g.as_str() == name && drawn(rid)).map(|(rid, _)| rid.clone()).collect();
        }
        Vec::new()
    }

    /// `selectPathway(key)`: select every drawn reaction of a pathway.
    pub fn select_pathway(&mut self, key: &str) -> Vec<String> {
        let reactions = self.pathway_reaction_ids(key);
        self.multi_selection = coxeter_map::multi_selection::empty_selection();
        self.multi_selection.reactions.extend(reactions.iter().cloned());
        self.selected_reaction = None;
        self.variability = None;
        self.view.multi_select = true;
        self.emit("multi-selection");
        reactions
    }

    fn set_model_pathway_setting(&mut self, name: &str, key: &str, value: Value) -> PathwaySetting {
        let entry = self.pathway_settings.entry(name.to_string()).or_default();
        entry.0.insert(key.to_string(), value);
        let out = entry.clone();
        self.emit("pathways");
        out
    }

    /// `setPathwayVisible(key, visible)`.
    pub fn set_pathway_visible(&mut self, key: &str, visible: bool) -> Result<bool> {
        if let Some(id) = key.strip_prefix("manual:") {
            return Ok(self.update_manual_pathway(id, PathwayPatch { visible: Some(visible), ..Default::default() })?.is_some());
        }
        if let Some(name) = key.strip_prefix("model:") {
            self.set_model_pathway_setting(name, "visible", json!(visible));
            return Ok(true);
        }
        Ok(false)
    }

    /// `setPathwayColor(key, color)`.
    pub fn set_pathway_color(&mut self, key: &str, color: &str) -> Result<bool> {
        let Some(next) = normalize_hex_color(color) else { return Ok(false) };
        if let Some(id) = key.strip_prefix("manual:") {
            return Ok(self.update_manual_pathway(id, PathwayPatch { color: Some(next), ..Default::default() })?.is_some());
        }
        if let Some(name) = key.strip_prefix("model:") {
            self.set_model_pathway_setting(name, "color", json!(next));
            return Ok(true);
        }
        Ok(false)
    }

    /// `pathwayManagerEntries()`: the rows of the pathway manager, manual
    /// pathways first.
    pub fn pathway_manager_entries(&self) -> Vec<PathwayEntry> {
        let (Some(_), Some(map)) = (self.community.as_ref(), self.map.as_ref()) else { return Vec::new() };
        let drawn: HashSet<String> = map.reaction_ids().into_iter().collect();
        let base = self.model_pathway_coloring(Some(&drawn));
        let mut entries = Vec::new();
        for item in &base.legend {
            if item.label.eq_ignore_ascii_case("Sin pathway") {
                continue;
            }
            let reaction_ids: Vec<String> =
                base.groups.iter().filter(|(rid, g)| drawn.contains(*rid) && **g == item.label).map(|(rid, _)| rid.clone()).collect();
            if reaction_ids.is_empty() {
                continue;
            }
            let settings = self.pathway_settings.get(&item.label);
            entries.push(PathwayEntry {
                key: format!("model:{}", item.label),
                id: item.label.clone(),
                name: item.label.clone(),
                source: "model",
                color: settings.and_then(|s| s.color()).map_or_else(|| item.color.to_string(), str::to_string),
                visible: settings.is_none_or(|s| s.visible()),
                count: reaction_ids.len(),
                reaction_ids,
            });
        }
        for pathway in self.manual_pathways.values() {
            let reaction_ids: Vec<String> = pathway.reaction_ids.iter().filter(|r| drawn.contains(*r)).cloned().collect();
            entries.push(PathwayEntry {
                key: format!("manual:{}", pathway.id),
                id: pathway.id.clone(),
                name: pathway.name.clone(),
                source: "manual",
                color: pathway.color.clone(),
                visible: pathway.visible,
                count: reaction_ids.len(),
                reaction_ids,
            });
        }
        entries.sort_by(|a, b| {
            if a.source != b.source {
                return if a.source == "manual" { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
            js::locale_compare(&a.name, &b.name)
        });
        entries
    }

    fn drawn_ids(&self, drawn_only: bool) -> Option<HashSet<String>> {
        drawn_only.then(|| self.map.as_ref().map(|m| m.reaction_ids().into_iter().collect()).unwrap_or_default())
    }

    /// `managedPathwayColoring({ drawnOnly })`: model pathways with the
    /// manager's colours and visibility, and the manual pathways over them.
    pub fn managed_pathway_coloring(&self, drawn_only: bool) -> Coloring {
        let reaction_ids = self.drawn_ids(drawn_only);
        let base = self.model_pathway_coloring(reaction_ids.as_ref());
        let mut colors = base.colors.clone();
        let mut groups = base.groups.clone();
        for (rid, group) in &base.groups {
            match self.pathway_settings.get(group) {
                Some(s) if s.hidden() => {
                    groups.shift_remove(rid);
                    colors.shift_remove(rid);
                }
                Some(s) => {
                    if let Some(c) = s.color() {
                        colors.insert(rid.clone(), c.to_string());
                    }
                }
                None => {}
            }
        }
        for pathway in self.manual_pathways.values() {
            for rid in &pathway.reaction_ids {
                if !self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(rid)) {
                    continue;
                }
                if !pathway.visible {
                    groups.shift_remove(rid);
                    colors.shift_remove(rid);
                } else {
                    groups.insert(rid.clone(), pathway.name.clone());
                    colors.insert(rid.clone(), pathway.color.clone());
                }
            }
        }
        let legend = legend_from_managed_groups(&groups, &colors, reaction_ids.as_ref());
        Coloring { mode: "pathway".into(), colors, groups, legend }
    }

    /// `reactionColoring({ drawnOnly })`: the colours the reactions are drawn
    /// in when they are not coloured by flux.
    pub fn reaction_coloring(&self, drawn_only: bool) -> Coloring {
        if self.view.reaction_color_mode == ReactionColorMode::Pathway {
            return self.managed_pathway_coloring(drawn_only);
        }
        let ids = self.drawn_ids(drawn_only);
        coloring(build_reaction_coloring(
            self.community.as_ref().map(|c| &c.merged),
            self.view.reaction_color_mode,
            ids.as_ref(),
            self.view.dark,
        ))
    }

    /// `pathwayRibbonColoring({ drawnOnly })`: the groups and colours of the
    /// ribbons under the reactions.
    pub fn pathway_ribbon_coloring(&self, drawn_only: bool) -> Coloring {
        match self.view.pathway_ribbon_mode {
            PathwayRibbonMode::Off => Coloring { mode: "off".into(), ..Coloring::default() },
            PathwayRibbonMode::Pathway => self.managed_pathway_coloring(drawn_only),
            PathwayRibbonMode::Subsystem => {
                let ids = self.drawn_ids(drawn_only);
                coloring(build_reaction_coloring(
                    self.community.as_ref().map(|c| &c.merged),
                    ReactionColorMode::Subsystem,
                    ids.as_ref(),
                    self.view.dark,
                ))
            }
        }
    }
}

/// Whether a group label is the explicit "no group" (`Sin pathway`).
pub fn is_unannotated(label: &str) -> bool {
    is_missing_group(label)
}
