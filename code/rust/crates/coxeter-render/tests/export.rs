//! The exported SVG, the SVG with its styles written out for a renderer
//! without CSS, and the PNG made from it.

mod common;

use coxeter_core::mapdata::{Canvas, MapData, MapGraph, NodeKind};
use coxeter_render::css::Paint;
use coxeter_render::export::{export_css, export_stylesheet};
use coxeter_render::{
    ExportOptions, GeometrySelection, InlineOptions, MultiSelection, RenderState, export_png, exportable_svg, exportable_tree,
    inline_styles, render_map,
};
use resvg::tiny_skia::Pixmap;
use resvg::usvg::roxmltree;

fn gapd() -> (MapGraph, RenderState) {
    let map = common::load_map(&common::dir(false).join("maps/gapd.json"));
    let spec = common::cases(false).unwrap().into_iter().find(|c| c.0 == "gapd-default").unwrap().2;
    (map, common::state_from_spec(&spec))
}

fn editing(mut state: RenderState, map: &MapGraph) -> RenderState {
    state.selected_reaction = Some("eco@GAPD".into());
    state.reaction_edit = true;
    state.min_hit_radius = 12.0;
    state.selected_geometry = Some(GeometrySelection { cell_id: Some("eco".into()), compartment_id: "c".into() });
    state.multi_selection = Some(MultiSelection { nodes: map.nodes.keys().take(3).cloned().collect(), ..Default::default() });
    state
}

#[test]
fn the_exported_svg_is_the_map_alone_with_its_stylesheet_rules_in_front() {
    let (map, state) = gapd();
    let state = editing(state, &map);
    let svg = exportable_svg(&map, &state, &ExportOptions::default());
    let doc = roxmltree::Document::parse(&svg).expect("well-formed XML");
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "svg");
    assert_eq!(root.tag_name().namespace(), Some("http://www.w3.org/2000/svg"));
    assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" class="emc-map" viewBox="0 0 "#));

    // The rules that style the map come first, as the browser's CSSOM writes them.
    let style = root.first_element_child().unwrap();
    assert_eq!(style.tag_name().name(), "style");
    let css = style.text().unwrap();
    assert_eq!(css, export_css());
    assert!(css.contains(r#".emc-map { font-family: "Inter", "Segoe UI", system-ui, -apple-system, sans-serif; --label-scale: 1; --frame-label-scale: 1; }"#));
    assert!(css.contains(".emc-label-met { font-size: calc(12.5px * var(--label-scale)); font-weight: 600; }"));
    assert!(css.contains(".emc-node.is-multi-selected { stroke: var(--accent, #2563eb) !important; stroke-width: 4px !important; filter: url(\"#emc-glow\"); }"));
    assert!(css.contains(".emc-node-metabolite, .emc-node-medium { cursor: grab; }"), "selector lists on one line");
    // An at-rule (`@media`) opens and closes on lines of its own around them.
    assert!(css.lines().all(|rule| rule.contains(".emc-") || rule.starts_with('@') || rule == "}"), "only rules that mention the map's classes");
    assert!(css.contains(".emc-selection-box {"), "app.css rules that mention them come too, as in the app");
    assert!(!css.contains("/*"));

    // The picture, not the editor around it.
    for marker in ["emc-hit ", "data-canvas-resize", "emc-multi-area\"", "is-selected", "emc-geometry-selection\"", "emc-layer-edit-handles"] {
        assert!(!svg[svg.find("</style>").unwrap()..].contains(marker), "the export must not contain {marker}");
    }
    assert!(svg.contains(r#"class="emc-node emc-node-metabolite"#));

    // Written as XMLSerializer writes a parsed document.
    assert!(svg.contains(r#"<rect class="emc-canvas-frame" data-canvas-bg="" x="0.75" y="0.75""#), "one space between attributes, each with a value");
    assert!(svg.contains(r#">Glycolysis &lt;upper&gt; &amp; "lower"</text>"#), "text keeps its quotes");
    assert!(!svg.contains("\n      x="), "no layout whitespace inside tags");

    // Apart from the <style>, it is renderMap(forExport).
    let mut tree = exportable_tree(&map, &state, &ExportOptions::default());
    tree.children.remove(0);
    assert_eq!(tree, render_map(&map, &RenderState { for_export: true, ..state.clone() }));

    let with_meta = exportable_svg(&map, &state, &ExportOptions { metadata: Some(r#"{"id":"zuniga","a":"<b>"}"#.into()), ..Default::default() });
    assert!(with_meta.contains(r#"role="img" aria-label="Multicell community metabolic map"><metadata>{"id":"zuniga","a":"&lt;b&gt;"}</metadata><style>"#));
}

#[test]
fn inlined_styles_leave_nothing_for_css_to_do() {
    let map = common::load_map(&common::dir(false).join("maps/edited.json"));
    let spec = common::cases(false).unwrap().into_iter().find(|c| c.0 == "edited-dark").unwrap().2;
    let state = common::state_from_spec(&spec);
    let tree = exportable_tree(&map, &state, &ExportOptions::default());
    let inlined = inline_styles(&tree, export_stylesheet(), &InlineOptions::default());
    let svg = inlined.to_xml();
    for gone in ["var(", "calc(", " style=", "<style", "min("] {
        assert!(!svg.contains(gone), "{gone} is left in the inlined SVG");
    }
    roxmltree::Document::parse(&svg).expect("well-formed XML");

    let all = inlined.descendants();
    let by_class = |class: &str| all.iter().copied().find(|e| e.has_class(class)).unwrap_or_else(|| panic!("no {class}"));
    // Label sizes come from the stylesheet, at --label-scale: 1.
    let met = by_class("emc-label-primary");
    assert_eq!(met.get("font-size"), Some("12.5"));
    assert_eq!(met.get("font-weight"), Some("600"));
    // A metabolite set to Secondary is written smaller: the later rule of equal weight wins.
    let minor = by_class("emc-label-secondary");
    assert!(minor.has_class("emc-label-met"));
    assert_eq!((minor.get("font-size"), minor.get("font-weight"), minor.get("opacity")), (Some("9.5"), Some("500"), Some("0.85")));
    let currency = by_class("emc-label-currency");
    assert_eq!((currency.get("font-size"), currency.get("opacity")), (Some("9.5"), Some("0.85")), "dark: .emc-map.emc-dark .emc-label-currency");
    // A name plate is scaled as a whole, never past its fit; its text is in capitals.
    let frame = by_class("emc-frame-label");
    let scale: f64 = frame.get("transform").unwrap().strip_prefix("scale(").unwrap().strip_suffix(')').unwrap().parse().unwrap();
    assert!(scale > 0.0 && scale <= 1.0);
    let name = by_class("emc-comp-text");
    assert_eq!(name.text_content(), name.text_content().to_uppercase());
    assert_eq!((name.get("font-size"), name.get("font-weight")), (Some("11.5"), Some("700")));
    // Secondary nodes are lighter, and the important rule holds their stroke width.
    let secondary = by_class("emc-node-secondary");
    assert_eq!((secondary.get("opacity"), secondary.get("stroke-width")), (Some("0.78"), Some("1.8")));
    // What the element says itself is kept.
    assert_eq!(secondary.get("fill"), Some("#f59e0b"));
    assert_eq!(by_class("emc-cell-name").get("filter"), Some("brightness(1.3)"));
    // The root states the font for everything under it.
    assert_eq!(inlined.get("font-family"), Some(r#""Inter", "Segoe UI", system-ui, -apple-system, sans-serif"#));

    // A stroke that must not scale is thinned by the scale of the picture.
    let twice = inline_styles(&tree, export_stylesheet(), &InlineOptions { device_scale: 2.0, ..Default::default() });
    let frame_of = |t: &coxeter_render::Element| t.descendants().into_iter().find(|e| e.has_class("emc-canvas-frame")).unwrap().get("stroke-width").map(str::to_string);
    assert_eq!(frame_of(&inlined).as_deref(), Some("1.5"));
    assert_eq!(frame_of(&twice).as_deref(), Some("0.75"));
}

#[test]
fn the_export_stylesheet_styles_the_export_like_the_page_does() {
    // Resolving against the exported rules gives what map.css alone gives.
    let (map, state) = gapd();
    let tree = render_map(&map, &RenderState { for_export: true, ..state });
    let a = export_stylesheet().resolve(&tree, &Default::default());
    let b = coxeter_render::map_stylesheet().resolve(&tree, &Default::default());
    assert_eq!(a.len(), tree.descendants().len());
    for ((x, y), el) in a.iter().zip(&b).zip(tree.descendants()) {
        for (name, value) in x.iter() {
            assert_eq!(value, y.get(name), "{name} of <{} class={:?}>", el.name, el.get("class"));
        }
    }
    assert!(a.iter().any(|s| s.fill() == Paint::None) && a.iter().any(|s| matches!(s.fill(), Paint::Color(_))));
    assert!(a.iter().any(|s| s.get("filter") == "url(#emc-cell-shadow)"), "a filter reference survives");
}

fn pixel(p: &Pixmap, x: f64, y: f64) -> (u8, u8, u8) {
    let c = p.pixel(x.round() as u32, y.round() as u32).expect("inside the picture").demultiply();
    (c.red(), c.green(), c.blue())
}

#[test]
fn a_png_is_the_export_at_twice_its_size_on_the_canvas_colour() {
    let (map, state) = gapd();
    let state = editing(state, &map);
    let out = export_png(&map, &state).expect("png");
    assert_eq!(out.scale, 2.0);
    assert_eq!((out.width as f64, out.height as f64), (map.width() * 2.0, map.height() * 2.0));
    assert_eq!(&out.png[..8], b"\x89PNG\r\n\x1a\n");
    let image = Pixmap::decode_png(&out.png).unwrap();
    assert_eq!((image.width(), image.height()), (out.width, out.height));

    // A primary metabolite is the orange disc the renderer asks for…
    let primary = map.nodes.values().find(|n| n.kind == NodeKind::Metabolite && n.primary != Some(false)).unwrap();
    assert_eq!(pixel(&image, primary.x * 2.0, primary.y * 2.0), (0xe6, 0x7e, 0x4c));
    // …and a cofactor is the same disc at the stylesheet's 78% opacity, so lighter.
    // (one that no other node is drawn over)
    let alone = |n: &&coxeter_core::mapdata::Node| {
        map.nodes.values().filter(|o| (o.x - n.x).abs() < 20.0 && (o.y - n.y).abs() < 20.0).count() == 1
    };
    let cofactor = map.nodes.values().filter(|n| n.kind == NodeKind::Currency).find(alone).unwrap();
    let (r, g, b) = pixel(&image, cofactor.x * 2.0, cofactor.y * 2.0);
    assert!((0x7e + 15..200).contains(&g) && b > 0x4c + 15 && r >= 0xdc, "{r} {g} {b}: orange at 78% over the compartment");
    // No editor furniture: where the selected reaction's ring would be there is none of its dashes.
    let colours: std::collections::HashSet<(u8, u8, u8)> =
        image.pixels().iter().step_by(7).map(|c| (c.red(), c.green(), c.blue())).collect();
    assert!(colours.len() > 100, "a drawing, not a blank sheet ({} colours)", colours.len());

    // Dark: the same map on the dark canvas.
    let dark = export_png(&map, &RenderState { dark: true, ..state }).expect("png");
    let dark_image = Pixmap::decode_png(&dark.png).unwrap();
    let mean = |p: &Pixmap| p.pixels().iter().step_by(11).map(|c| c.red() as f64 + c.green() as f64 + c.blue() as f64).sum::<f64>();
    assert!(mean(&dark_image) < mean(&image) * 0.5);
    assert_eq!(pixel(&dark_image, primary.x * 2.0, primary.y * 2.0), (0xf5, 0x9e, 0x0b));
}

#[test]
fn text_is_drawn_when_the_system_has_fonts() {
    let (map, state) = gapd();
    let annotation = map.texts.values().find(|t| t.text == "Plain note").unwrap();
    let image = Pixmap::decode_png(&export_png(&map, &state).unwrap().png).unwrap();
    // The note is dark text with a white halo; look for ink in its box.
    let size = annotation.font_size.unwrap_or(22.0);
    let mut dark = 0;
    for y in ((annotation.y - size) * 2.0) as u32..(annotation.y * 2.0) as u32 {
        for x in (annotation.x * 2.0) as u32..((annotation.x + size * 4.0) * 2.0) as u32 {
            let c = image.pixel(x, y).unwrap();
            if (c.red() as u32 + c.green() as u32 + c.blue() as u32) < 150 {
                dark += 1;
            }
        }
    }
    assert!(dark > 50, "only {dark} dark pixels where the note is written; are any fonts installed?");
}

#[test]
fn a_map_too_large_for_twice_its_size_is_made_at_the_largest_scale_that_fits() {
    // 40000 × 200: held by the 32767-pixel side limit, as rasterScales says.
    let data: MapData = serde_json::from_value(serde_json::json!({
        "version": 1, "canvas": {"width": 40000, "height": 200},
        "cells": [], "nodes": [], "reactions": [], "segments": [], "texts": []
    }))
    .unwrap();
    let map = MapGraph::from_data(data);
    assert_eq!(map.canvas, Canvas { width: 40000.0, height: 200.0 });
    let out = export_png(&map, &RenderState::default()).expect("png");
    assert_eq!(out.scale, 0.81);
    assert_eq!((out.width, out.height), (32400, 162));
}

/// Not a test: writes the SVG, the inlined SVG and the PNG of one fixture case
/// to `$COXETER_RENDER_DUMP/` for looking at.
///
///   COXETER_RENDER_DUMP=/tmp/out COXETER_RENDER_CASE=zuniga-default \
///     cargo test -p coxeter-render --test export -- --ignored write_a_case
#[test]
#[ignore]
fn write_a_case_for_looking_at() {
    let Ok(dir) = std::env::var("COXETER_RENDER_DUMP") else { return };
    let name = std::env::var("COXETER_RENDER_CASE").unwrap_or_else(|_| "gapd-default".into());
    let large = std::env::var("COXETER_RENDER_LARGE").is_ok();
    let (_, map_path, spec, _) = common::cases(large).unwrap().into_iter().find(|c| c.0 == name).expect("a fixture case");
    let map = common::load_map(&map_path);
    let state = common::state_from_spec(&spec);
    let dir = std::path::PathBuf::from(dir);
    std::fs::write(dir.join(format!("{name}.svg")), exportable_svg(&map, &state, &ExportOptions::default())).unwrap();
    let tree = exportable_tree(&map, &state, &ExportOptions::default());
    let inlined = inline_styles(&tree, export_stylesheet(), &InlineOptions { device_scale: 2.0, ..Default::default() });
    std::fs::write(dir.join(format!("{name}.inlined.svg")), inlined.to_xml()).unwrap();
    let started = std::time::Instant::now();
    let out = export_png(&map, &state).unwrap();
    eprintln!("{name}: {} × {} px at {}× in {:.0} ms", out.width, out.height, out.scale, started.elapsed().as_secs_f64() * 1e3);
    std::fs::write(dir.join(format!("{name}.png")), out.png).unwrap();
}
