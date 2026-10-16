//! The render half of `test/line-styles.test.mjs` (7b5aaa0): grey dashes for
//! reactions without flux, width by flux only when asked, a reaction's own
//! colour over its pathway's but not over flux colours, metabolite circle
//! colours map-wide and per node, and hover text. On the E. coli map of the
//! render fixtures; the JS builds its own small model. The line-hop unit tests
//! are in `src/line_hops.rs`; the Store half, hops on a real map included
//! (the fixture maps here have no crossing lines), in
//! `coxeter-state/tests/js_line_styles.rs`.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use coxeter_core::mapdata::MapGraph;
use coxeter_render::reaction_colors::ReactionColorMode;
use coxeter_render::{FluxScale, ReactionWidthMode, RenderState, Tooltips, render_map_svg};
use regex::Regex;

fn gapd() -> MapGraph {
    common::load_map(&common::dir(false).join("maps/gapd.json"))
}

/// GAPD and PGK carry flux; the transporter, the exchange and ACt2rpp do not.
fn solved_state() -> RenderState {
    let fluxes: HashMap<String, f64> = [("eco@GAPD", 8.0), ("eco@PGK", -8.0), ("eco@GLCptspp", 0.0), ("eco@EX_glc__D_e", 0.0), ("eco@ACt2rpp", 0.0)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    RenderState {
        scale: Arc::new(FluxScale::automatic(fluxes.values().copied(), false)),
        solved_fluxes: Some(fluxes.clone()),
        fluxes,
        ..Default::default()
    }
}

/// The `<path>` elements a reaction's lines are drawn with.
fn segments_of(svg: &str, rid: &str) -> Vec<String> {
    Regex::new(r#"<path class="emc-seg[^"]*"[^>]*>"#)
        .unwrap()
        .find_iter(svg)
        .map(|m| m.as_str().to_string())
        .filter(|tag| tag.contains(&format!(r#"data-rxn="{rid}""#)))
        .collect()
}

#[test]
fn with_a_solution_a_reaction_that_carries_no_flux_is_drawn_grey_and_dashed() {
    let map = gapd();
    let svg = render_map_svg(&map, &RenderState { off_style: true, ..solved_state() });
    let dead = segments_of(&svg, "eco@ACt2rpp");
    let live = segments_of(&svg, "eco@GAPD");
    assert!(!dead.is_empty() && !live.is_empty());
    for tag in &dead {
        assert!(tag.contains("is-off") && tag.contains(r##"stroke="#9b9da5""##) && tag.contains("stroke-dasharray=\""), "{tag}");
    }
    for tag in &live {
        assert!(!tag.contains("is-off") && !tag.contains("stroke-dasharray"), "{tag}");
    }
    // Its name, where names are shown, is greyed with it.
    let named = render_map_svg(&map, &RenderState { off_style: true, labels: coxeter_render::LabelMode::All, ..solved_state() });
    assert!(Regex::new(r#"<text class="emc-label emc-label-rxn is-off[^"]*"[^>]*data-rxn="eco@ACt2rpp""#).unwrap().is_match(&named));
    assert!(!Regex::new(r#"<text class="emc-label emc-label-rxn is-off[^"]*"[^>]*data-rxn="eco@GAPD""#).unwrap().is_match(&named));

    let plain = render_map_svg(&map, &solved_state());
    assert!(!plain.contains("is-off"), "switched off, every line keeps its own colour");
}

#[test]
fn line_width_follows_flux_only_when_asked_to() {
    let map = gapd();
    let widths = |svg: &str| {
        Regex::new(r#"<path class="emc-seg[^"]*"[^>]*stroke-width="([\d.]+)""#)
            .unwrap()
            .captures_iter(svg)
            .map(|c| c[1].to_string())
            .collect::<std::collections::HashSet<_>>()
    };
    assert_eq!(widths(&render_map_svg(&map, &solved_state())).len(), 1);
    assert!(widths(&render_map_svg(&map, &RenderState { reaction_width_mode: ReactionWidthMode::Flux, ..solved_state() })).len() > 1);
}

#[test]
fn a_reactions_own_colour_is_used_over_its_pathways_but_not_over_flux_colours() {
    let mut map = gapd();
    map.reactions.get_mut("eco@GAPD").unwrap().extra.insert("color".into(), "#123456".into());
    let by_group = RenderState {
        reaction_color_mode: ReactionColorMode::Subsystem,
        reaction_colors: [("eco@GAPD", "#abcdef"), ("eco@PGK", "#abcdef")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        ..solved_state()
    };
    let svg = render_map_svg(&map, &by_group);
    for tag in segments_of(&svg, "eco@GAPD") {
        assert!(tag.contains(r##"stroke="#123456""##), "{tag}");
    }
    assert!(!segments_of(&svg, "eco@PGK")[0].contains("#123456"));
    let by_flux = render_map_svg(&map, &solved_state());
    for tag in segments_of(&by_flux, "eco@GAPD") {
        assert!(!tag.contains("#123456"), "flux colours win: {tag}");
    }
}

#[test]
fn metabolite_circles_take_the_maps_colours_and_one_metabolite_its_own() {
    let mut map = gapd();
    map.set_node_style(&coxeter_core::mapdata::NodeStyle { fill: Some("#aabbcc".into()), stroke: Some("#112233".into()), ..Default::default() });
    let id = map.nodes.values().find(|n| n.is_metabolite_kind()).unwrap().id.clone();
    map.nodes.get_mut(&id).unwrap().extra.insert("fill".into(), "#ff0000".into());
    let svg = render_map_svg(&map, &solved_state());
    let circles: Vec<String> = Regex::new(r#"<circle class="emc-node[^"]*"[^>]*>"#).unwrap().find_iter(&svg).map(|m| m.as_str().to_string()).collect();
    let own = circles.iter().find(|t| t.contains(&format!(r#"data-node="{id}""#))).unwrap();
    assert!(own.contains(r##"fill="#ff0000""##), "{own}");
    assert!(own.contains(r##"stroke="#112233""##), "what it does not set comes from the map: {own}");
    let metabolites: Vec<&String> = circles.iter().filter(|t| !t.contains(&id) && t.contains("emc-node-primary")).collect();
    assert!(!metabolites.is_empty() && metabolites.iter().all(|t| t.contains(r##"fill="#aabbcc""##)));
}

#[test]
fn hover_text_goes_on_reactions_and_metabolites_when_asked() {
    let map = gapd();
    let id = map.nodes.values().find(|n| n.is_metabolite_kind()).unwrap().id.clone();
    let tips = Tooltips {
        reactions: [("eco@GAPD".to_string(), "GAPD\nglyceraldehyde".to_string())].into_iter().collect(),
        nodes: [(id.clone(), "g3p\nglyceraldehyde 3-phosphate".to_string())].into_iter().collect(),
    };
    let svg = render_map_svg(&map, &RenderState { for_export: true, tooltips: Some(Arc::new(tips)), ..solved_state() });
    assert!(svg.contains("<title>GAPD\nglyceraldehyde</title></path>"), "on the lines");
    assert!(Regex::new(r#"<g class="emc-marker[^"]*" data-rxn="eco@GAPD"[^>]*>\s*<title>GAPD"#).unwrap().is_match(&svg), "on the marker");
    assert!(svg.contains("<title>g3p\nglyceraldehyde 3-phosphate</title></circle>"), "on the circle");
    assert!(!render_map_svg(&map, &solved_state()).contains("<title>GAPD"), "none unless asked");
}
