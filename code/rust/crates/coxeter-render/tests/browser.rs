//! The parts with no JavaScript module to compare with, checked against a real
//! browser (rust/tools/fixtures-render-browser.py, headless Firefox):
//! the stylesheet resolver against `getComputedStyle`, the XML form of the
//! tree against `XMLSerializer`, and the written rules against `cssText`.

mod common;

use coxeter_render::css::{ComputedStyle, Paint, ResolveOptions, parse_paint, parse_transform};
use coxeter_render::export::export_css;
use coxeter_render::{ExportOptions, RenderState, exportable_svg, map_stylesheet, render_map};
use serde_json::Value;

fn reference() -> Option<Value> {
    let r = common::read_json(&common::dir(false).join("browser.json"));
    if r.is_none() {
        eprintln!("no browser reference; run rust/tools/fixtures-render-browser.py");
    }
    r
}

fn tree_of(case: &str) -> coxeter_render::Element {
    let (_, map_path, spec, _) = common::cases(false).unwrap().into_iter().find(|c| c.0 == case).unwrap_or_else(|| panic!("no case {case}"));
    render_map(&common::load_map(&map_path), &common::state_from_spec(&spec))
}

#[test]
fn the_xml_form_is_what_xmlserializer_writes() {
    let Some(reference) = reference() else { return };
    let cases = reference["xml"].as_array().unwrap();
    assert!(cases.len() >= 5);
    for case in cases {
        let name = case["case"].as_str().unwrap();
        common::assert_same_svg(&common::before_shapes(&tree_of(name).to_xml()), &common::before_shapes(case["xml"].as_str().unwrap()), name);
    }
}

#[test]
fn rules_are_written_as_the_browser_writes_csstext() {
    let Some(reference) = reference() else { return };
    let expected: Vec<&str> = reference["cssText"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    // Rules added since the reference was recorded are not in it (common::NEW_RULES).
    let actual: Vec<String> = map_stylesheet().rules_text_containing("").into_iter().filter(|r| !common::is_new_rule(r)).collect();
    assert_eq!(actual.len(), expected.len(), "the same number of rules");
    let mut different = Vec::new();
    for (a, e) in actual.iter().zip(&expected) {
        if a != e {
            different.push(format!("  rust: {a}\n  browser: {e}"));
        }
    }
    assert!(different.is_empty(), "{} of {} rules are written differently:\n{}", different.len(), expected.len(), different.join("\n\n"));
}

#[test]
fn the_exported_svg_is_what_exportablesvg_returns_in_the_browser() {
    let Some(reference) = reference() else { return };
    // The <style> text: the rules of app.css and map.css that mention `.emc-`.
    let expected = reference["exportCss"].as_str().unwrap();
    let actual: String = export_css().lines().filter(|r| !common::is_new_rule(r)).collect::<Vec<_>>().join("\n");
    for (a, e) in actual.lines().zip(expected.lines()) {
        assert_eq!(a, e, "a rule of the exported <style>");
    }
    assert_eq!(actual.lines().count(), expected.lines().count());

    // And the whole file, for cases drawn for export.
    let cases = reference["exportable"].as_array().unwrap();
    assert!(cases.len() >= 3);
    for case in cases {
        let name = case["case"].as_str().unwrap();
        let (_, map_path, spec, _) = common::cases(false).unwrap().into_iter().find(|c| c.0 == name).unwrap();
        let state = common::state_from_spec(&spec);
        assert!(state.for_export, "{name} is an export case");
        let svg = exportable_svg(&common::load_map(&map_path), &state, &ExportOptions::default());
        common::assert_same_svg(&common::before_shapes(&svg), &common::before_shapes(case["svg"].as_str().unwrap()), name);
        // Asking for the export is enough: the state need not say so itself.
        let asked = exportable_svg(&common::load_map(&map_path), &RenderState { for_export: false, ..state }, &ExportOptions::default());
        assert_eq!(asked, svg);
    }
}

fn px(v: &str) -> Option<f64> {
    v.trim().trim_end_matches("px").parse().ok()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * a.abs().max(b.abs()).max(1.0)
}

/// Is `ours` the same computed value as the browser's `theirs`?
fn same(property: &str, ours: &str, theirs: &str) -> bool {
    match property {
        "color" | "fill" | "stroke" => match (parse_paint(ours), parse_paint(theirs)) {
            (Paint::Color(a), Paint::Color(b)) => (a.r, a.g, a.b) == (b.r, b.g, b.b) && (a.a - b.a).abs() < 0.005,
            (a, b) => a == b,
        },
        "fill-opacity" | "stroke-opacity" | "opacity" => match (ours.parse::<f64>(), theirs.parse::<f64>()) {
            (Ok(a), Ok(b)) => close(a, b, 1e-4),
            _ => false,
        },
        // Firefox keeps font sizes to 1/60 px and prints six digits.
        "font-size" | "stroke-width" | "letter-spacing" => match (px(ours), px(theirs)) {
            (Some(a), Some(b)) => close(a, b, 2e-3),
            _ => ours == theirs,
        },
        "stroke-dasharray" => {
            let list = |v: &str| -> Option<Vec<f64>> { v.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).map(px).collect() };
            match (ours, theirs) {
                ("none", "none") => true,
                (a, b) => list(a).is_some() && list(a) == list(b),
            }
        }
        "display" => (ours == "none") == (theirs == "none"),
        "filter" => ours.replace('"', "") == theirs.replace('"', ""),
        "transform" => match (parse_transform(ours), parse_transform(theirs)) {
            (Some(a), Some(b)) => a.iter().zip(&b).all(|(x, y)| close(*x, *y, 2e-5) || (x - y).abs() < 2e-5),
            _ => false,
        },
        _ => ours == theirs,
    }
}

#[test]
fn computed_styles_are_what_the_browser_computes() {
    let Some(reference) = reference() else { return };
    let properties: Vec<&str> = reference["properties"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    let mut compared = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for case in reference["styles"].as_array().unwrap() {
        let name = case["case"].as_str().unwrap();
        let tree = tree_of(name);
        // A selected compartment has more handles than the reference knows
        // (see common::before_shapes), so its elements do not line up.
        if tree.to_markup().contains("emc-layer-geometry-editor") {
            eprintln!("{name}: stale since d7b2c0e (geometry handles), not compared");
            continue;
        }
        let o = &case["options"];
        let opts = ResolveOptions {
            label_scale: o["labelScale"].as_f64(),
            frame_label_scale: o["frameLabelScale"].as_f64(),
            is_far: o["isFar"].as_bool().unwrap_or(false),
            ..Default::default()
        };
        let ours: Vec<ComputedStyle> = map_stylesheet().resolve(&tree, &opts);
        let elements = tree.descendants();
        assert_eq!(ours.len(), case["count"].as_u64().unwrap() as usize, "{name}: as many elements as the browser sees");
        let styles = case["styles"].as_array().unwrap();
        for (k, index) in case["elements"].as_array().unwrap().iter().enumerate() {
            let theirs = styles[index.as_u64().unwrap() as usize].as_array().unwrap();
            for (property, expected) in properties.iter().zip(theirs) {
                let expected = expected.as_str().unwrap();
                let actual = ours[k].get(property);
                compared += 1;
                if !same(property, actual, expected) && failures.len() < 40 {
                    failures.push(format!(
                        "{name} {o}: <{} class={:?}> {property}: rust {actual:?}, browser {expected:?}",
                        elements[k].name,
                        elements[k].get("class").unwrap_or("")
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} differences (first shown) in {compared} values:\n{}", failures.len(), failures.join("\n"));
    assert!(compared > 80_000, "{compared} values compared");
}
