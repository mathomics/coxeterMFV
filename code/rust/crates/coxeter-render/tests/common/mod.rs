//! Reading the render fixtures written by `rust/tools/fixtures-render.mjs`.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use coxeter_core::community::{CrossFeeding, CrossFeedingLink};
use coxeter_core::js;
use coxeter_core::mapdata::{MapData, MapGraph, Point};
use coxeter_core::testkit::fixture_path;
use coxeter_render::reaction_colors::{normalize_pathway_ribbon_mode, normalize_reaction_color_mode};
use coxeter_render::theme::normalize_flux_color_stops_value;
use coxeter_render::{
    CrossFeedingEvent, CrossFeedingExplorerView, FluxScale, FluxScaleOptions, FvaRange, GeometrySelection, LabelMode,
    MultiSelection, ReactionWidthMode, RenderState,
};
use serde_json::Value;

// References recorded before d7b2c0e

// The render references (SVGs, browser.json) were recorded before d7b2c0e,
// 7b5aaa0 and the line hops of 2a6a8ed, and are not regenerated (lean
// testing); only those of the Zuñiga map were recorded again when the map
// itself changed. What those commits changed on purpose is taken out of both
// sides before comparing:
//
// - every compartment outline writes `ry` beside `rx` (corners may be
//   elliptical): an `ry` equal to its `rx` is dropped;
// - the geometry editor has side handles and shape diamonds and is drawn
//   after the name plates: its layer is left out (tests/js_compartment_shape.rs
//   checks it);
// - a reaction line writes `d` last, once its crossings are known: it is put
//   back straight after `class`;
// - map.css has the cursor rules of those handles and the grey label of a
//   reaction without flux (NEW_RULES).

/// Rules of map.css added by d7b2c0e and 7b5aaa0, and the ring a reactions
/// table puts on the map under its hovered row.
pub const NEW_RULES: &[&str] = &[
    ".emc-geometry-n, .emc-geometry-s",
    ".emc-geometry-e, .emc-geometry-w",
    ".emc-geometry-corner",
    ".emc-geometry-oval",
    ".emc-label-rxn.is-off",
    "emc-table-hover",
    ".is-table-hover",
];

/// Whether a written rule is one the references do not know yet.
pub fn is_new_rule(rule: &str) -> bool {
    NEW_RULES.iter().any(|r| rule.contains(r))
}

/// A stylesheet written a rule a line, without the new rules, and without an
/// at-rule block that held only new ones.
pub fn without_new_rules(css: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut open: Option<(usize, bool)> = None;
    for line in css.lines() {
        if line.starts_with('@') && line.trim_end().ends_with('{') {
            open = Some((out.len(), false));
            out.push(line);
        } else if line == "}" && open.is_some() {
            let (at, kept) = open.take().unwrap_or_default();
            if kept { out.push(line) } else { out.truncate(at) }
        } else if !is_new_rule(line) {
            if let Some((_, kept)) = open.as_mut() {
                *kept = true;
            }
            out.push(line);
        }
    }
    out
}

/// An SVG as it compares with the references recorded before d7b2c0e.
pub fn before_shapes(svg: &str) -> String {
    let ry = regex::Regex::new(r#" rx="([^"]*)" ry="([^"]*)""#).unwrap();
    let svg = ry.replace_all(svg, |c: &regex::Captures| {
        if c[1] == c[2] { format!(r#" rx="{}""#, &c[1]) } else { c[0].to_string() }
    });
    let editor = regex::Regex::new(r#"(?s)<g class="emc-layer emc-layer-geometry-editor">.*?</g>"#).unwrap();
    let svg = editor.replace_all(&svg, "");
    let d_last = regex::Regex::new(r#"<path (class="emc-seg[^"]*")((?: [a-z-]+="[^"]*")*?) d="([^"]*)"(/?>)"#).unwrap();
    let svg = d_last.replace_all(&svg, r#"<path $1 d="$3"$2$4"#);
    // An exported file carries the stylesheet, a rule a line.
    let mut svg = svg.into_owned();
    for rule in NEW_RULES {
        let re = regex::Regex::new(&format!(r"\n[^\n{{}}<>]*{}[^\n{{}}]*\{{[^}}]*\}}", regex::escape(rule))).unwrap();
        svg = re.replace_all(&svg, "").into_owned();
    }
    // An at-rule block that held only new rules.
    let empty = regex::Regex::new(r"\n@[^\n{}<>]*\{\n\}").unwrap();
    empty.replace_all(&svg, "").into_owned()
}

/// `rust/fixtures/render` or `rust/fixtures/large/render`.
pub fn dir(large: bool) -> PathBuf {
    fixture_path(if large { "large/render" } else { "render" })
}

pub fn read_json(path: &PathBuf) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// A map saved by `EditableMap#toJSON()`, as the renderer reads it.
pub fn load_map(path: &PathBuf) -> MapGraph {
    let raw = read_json(path).unwrap_or_else(|| panic!("missing {}", path.display()));
    let raw = raw.get("map").cloned().filter(|m| m.get("nodes").is_some()).unwrap_or(raw);
    let data: MapData = serde_json::from_value(raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    MapGraph::from_data(data)
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Null => f64::NAN,
        other => js::to_number(other),
    }
}

fn f64_map(v: &Value) -> HashMap<String, f64> {
    v.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), num(v))).collect()).unwrap_or_default()
}

fn str_map(v: &Value) -> HashMap<String, String> {
    v.as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default()
}

fn str_set(v: &Value) -> HashSet<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn opt_str(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

fn event(v: &Value) -> CrossFeedingEvent {
    CrossFeedingEvent {
        from: opt_str(v.get("from")).unwrap_or_default(),
        to: opt_str(v.get("to")).unwrap_or_default(),
        base: opt_str(v.get("base")).unwrap_or_default(),
        flux: v.get("flux").map_or(0.0, num),
        producer_transfer: opt_str(v.get("producerTransfer")),
        consumer_transfer: opt_str(v.get("consumerTransfer")),
    }
}

pub fn link(v: &Value) -> CrossFeedingLink {
    CrossFeedingLink {
        base: opt_str(v.get("base")).unwrap_or_default(),
        from: opt_str(v.get("from")),
        to: opt_str(v.get("to")),
        flux: v.get("flux").map_or(0.0, num),
    }
}

/// The flux scale a state names: built from `values` (or the state's fluxes).
pub fn scale_from_spec(spec: &Value, fluxes: &HashMap<String, f64>) -> FluxScale {
    let values: Vec<f64> = match spec.get("values") {
        Some(Value::String(s)) if s == "fluxes" => fluxes.values().copied().collect(),
        Some(Value::Array(a)) => a.iter().map(num).collect(),
        _ => vec![],
    };
    let color_stops = normalize_flux_color_stops_value(spec.get("colorStops").unwrap_or(&Value::Null)).expect("valid stops");
    let mut scale = FluxScale::new(
        values,
        FluxScaleOptions { dark: spec.get("dark").and_then(Value::as_bool).unwrap_or(false), color_stops, ..Default::default() },
    )
    .expect("valid scale");
    if let Some(r) = spec.get("reference") {
        scale.reference = num(r);
    }
    scale
}

/// Rebuild a `RenderState` from the JSON `toState()` reads in the fixture script.
pub fn state_from_spec(spec: &Value) -> RenderState {
    let mut s = RenderState::default();
    let get = |k: &str| spec.get(k);
    let flag = |k: &str| get(k).map(js::truthy);
    if let Some(v) = get("fluxes") {
        s.fluxes = f64_map(v);
    }
    if let Some(v) = get("solvedFluxes") {
        s.solved_fluxes = Some(f64_map(v));
    }
    if let Some(v) = get("scale") {
        s.scale = Arc::new(scale_from_spec(v, &s.fluxes));
    }
    if let Some(v) = flag("dark") {
        s.dark = v;
    }
    if let Some(v) = get("reactionWidthMode").and_then(Value::as_str) {
        s.reaction_width_mode = if v == "flux" { ReactionWidthMode::Flux } else { ReactionWidthMode::Constant };
    }
    if let Some(v) = get("disabledReactionIds") {
        s.disabled_reaction_ids = str_set(v);
    }
    if let Some(v) = get("inactiveSpeciesIds") {
        s.inactive_species_ids = str_set(v);
    }
    if let Some(v) = get("inactiveSpeciesLabel").and_then(Value::as_str) {
        s.inactive_species_label = v.to_string();
    }
    if let Some(v) = get("emphasizedReactionIds") {
        s.emphasized_reaction_ids = str_set(v);
    }
    if let Some(v) = get("reactionColorMode").and_then(Value::as_str) {
        s.reaction_color_mode = normalize_reaction_color_mode(v);
    }
    if let Some(v) = get("reactionColors") {
        s.reaction_colors = str_map(v);
    }
    if let Some(v) = get("pathwayRibbonMode").and_then(Value::as_str) {
        s.pathway_ribbon_mode = normalize_pathway_ribbon_mode(v);
    }
    if let Some(v) = get("pathwayRibbonColors") {
        s.pathway_ribbon_colors = str_map(v);
    }
    if let Some(v) = get("pathwayRibbonGroups") {
        s.pathway_ribbon_groups = str_map(v);
    }
    if let Some(v) = get("pathwayRibbonWidth") {
        s.pathway_ribbon_width = num(v);
    }
    if let Some(v) = get("pathwayRibbonOpacity") {
        s.pathway_ribbon_opacity = num(v);
    }
    s.selected_reaction = opt_str(get("selectedReaction"));
    if let Some(v) = get("selectedGeometry").filter(|v| v.is_object()) {
        s.selected_geometry = Some(GeometrySelection {
            cell_id: opt_str(v.get("cellId")),
            compartment_id: opt_str(v.get("compartmentId")).unwrap_or_default(),
        });
    }
    if let Some(v) = flag("geometryEdit") {
        s.geometry_edit = v;
    }
    if let Some(v) = flag("reactionEdit") {
        s.reaction_edit = v;
    }
    if let Some(v) = get("multiSelection").filter(|v| v.is_object()) {
        s.multi_selection = Some(MultiSelection {
            nodes: v.get("nodes").map(str_set).unwrap_or_default(),
            reactions: v.get("reactions").map(str_set).unwrap_or_default(),
            texts: v.get("texts").map(str_set).unwrap_or_default(),
            ..Default::default()
        });
    }
    if let Some(v) = flag("fvaEnabled") {
        s.fva_enabled = v;
    }
    if let Some(v) = get("fvaRanges").and_then(Value::as_object) {
        s.fva_ranges = v
            .iter()
            .map(|(k, r)| (k.clone(), FvaRange { min: r.get("min").map_or(f64::NAN, num), max: r.get("max").map_or(f64::NAN, num) }))
            .collect();
    }
    if let Some(v) = get("labels").and_then(Value::as_str) {
        s.labels = LabelMode::parse(v);
    }
    if let Some(v) = flag("showCurrency") {
        s.show_currency = v;
    }
    if let Some(v) = flag("currencyLabels") {
        s.currency_labels = v;
    }
    if let Some(v) = get("metaboliteLabels") {
        s.metabolite_labels = str_map(v);
    }
    if let Some(v) = get("reactionLabels") {
        s.reaction_labels = str_map(v);
    }
    if let Some(v) = get("growth") {
        s.growth = f64_map(v);
    }
    if let Some(v) = get("crossFeeding").filter(|v| v.is_object()) {
        s.cross_feeding = Some(CrossFeeding {
            links: v.get("links").and_then(Value::as_array).map(|a| a.iter().map(link).collect()).unwrap_or_default(),
            per_metabolite: vec![],
        });
    }
    if let Some(v) = flag("showCrossFeeding") {
        s.show_cross_feeding = v;
    }
    if let Some(v) = get("crossFeedingExplorer").filter(|v| v.is_object()) {
        s.cross_feeding_explorer = Some(CrossFeedingExplorerView {
            enabled: v.get("enabled").is_some_and(js::truthy),
            visible_events: v.get("visibleEvents").and_then(Value::as_array).map(|a| a.iter().map(event).collect()).unwrap_or_default(),
            selected_event: v.get("selectedEvent").filter(|e| e.is_object()).map(event),
            selected_at: v
                .get("selectedAt")
                .filter(|p| p.is_object())
                .map(|p| Point::new(p.get("x").map_or(f64::NAN, num), p.get("y").map_or(f64::NAN, num))),
        });
    }
    if let Some(v) = get("crossFeedingBases") {
        s.cross_feeding_bases = v.as_array().map(|_| str_set(v));
    }
    if let Some(v) = get("crossFeedingWidth").filter(|v| v.is_number()) {
        s.cross_feeding_width = num(v);
    }
    if let Some(v) = flag("omicsEnabled") {
        s.omics_enabled = v;
    }
    if let Some(v) = get("omicsColors") {
        s.omics_colors = str_map(v);
    }
    s.omics_fallback_color = opt_str(get("omicsFallbackColor"));
    if let Some(v) = get("minHitRadius") {
        s.min_hit_radius = num(v);
    }
    if let Some(v) = flag("forExport") {
        s.for_export = v;
    }
    s
}

/// Panic with where two SVG strings first differ, and what is around it.
#[track_caller]
pub fn assert_same_svg(actual: &str, expected: &str, what: &str) {
    if actual == expected {
        return;
    }
    let a = actual.as_bytes();
    let e = expected.as_bytes();
    let at = a.iter().zip(e).position(|(x, y)| x != y).unwrap_or(a.len().min(e.len()));
    let around = |s: &str| {
        let mut lo = at.saturating_sub(300);
        while !s.is_char_boundary(lo) {
            lo -= 1;
        }
        let mut hi = (at + 300).min(s.len());
        while !s.is_char_boundary(hi) {
            hi += 1;
        }
        s[lo..hi].to_string()
    };
    panic!(
        "{what}: the SVG differs from the JavaScript output at byte {at} (lengths {} vs {})\n--- rust ---\n{}\n--- js ---\n{}",
        actual.len(),
        expected.len(),
        around(actual),
        around(expected)
    );
}

/// Every render case of one fixture folder: (name, map, state, expected svg).
pub fn cases(large: bool) -> Option<Vec<(String, PathBuf, Value, PathBuf)>> {
    let d = dir(large);
    let index = read_json(&d.join("cases.json"))?;
    Some(
        index
            .as_array()
            .expect("a list of cases")
            .iter()
            .map(|c| {
                (
                    c["name"].as_str().unwrap().to_string(),
                    d.join(c["map"].as_str().unwrap()),
                    c["state"].clone(),
                    d.join(c["svg"].as_str().unwrap()),
                )
            })
            .collect(),
    )
}
