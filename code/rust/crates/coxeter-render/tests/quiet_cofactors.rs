//! Quiet cofactors (`cofactorStyle: 'quiet'`, test/quiet-cofactors.test.mjs):
//! a map that asks for them draws its secondary metabolites smaller and their
//! branches thinner, and nothing else changes.

mod common;

use coxeter_core::mapdata::MapGraph;
use coxeter_render::{QUIET_COFACTORS, RenderState, is_quiet_cofactors, render_map_svg};
use regex::Regex;
use serde_json::Value;

fn gapd() -> MapGraph {
    common::load_map(&common::dir(false).join("maps/gapd-only.json"))
}

fn circle<'a>(svg: &'a str, node: &str) -> &'a str {
    let re = Regex::new(&format!(r#"<circle class="emc-node[^"]*" data-node="{}"[^>]*>"#, regex::escape(node))).unwrap();
    re.find(svg).unwrap_or_else(|| panic!("no circle for {node}")).as_str()
}

fn branch_widths(svg: &str) -> Vec<(String, f64)> {
    let re = Regex::new(r#"<path class="(emc-seg emc-seg-currency[^"]*)"[^>]*stroke-width="([^"]+)""#).unwrap();
    re.captures_iter(svg).map(|c| (c[1].to_string(), c[2].parse().unwrap())).collect()
}

#[test]
fn a_quiet_map_draws_its_cofactors_small_and_thin_and_nothing_else_differs() {
    let full_map = gapd();
    let mut quiet_map = gapd();
    quiet_map.extra.insert("cofactorStyle".into(), Value::from("quiet"));
    assert!(!is_quiet_cofactors(&full_map) && is_quiet_cofactors(&quiet_map));

    let state = RenderState::default();
    let full = render_map_svg(&full_map, &state);
    let quiet = render_map_svg(&quiet_map, &state);
    assert!(!full.contains("emc-quiet-cofactors"));
    assert!(quiet.contains(r#"class="emc-map emc-quiet-cofactors""#));

    let nad = "cur:eco@GAPD:eco@nad_c";
    assert!(circle(&full, nad).contains(r#"r="7""#));
    assert!(circle(&quiet, nad).contains(&format!(r#"r="{}""#, QUIET_COFACTORS.radius)));
    let g3p = "met:eco@GAPD:eco@g3p_c";
    assert_eq!(circle(&quiet, g3p), circle(&full, g3p));

    let quiet_branches = branch_widths(&quiet);
    assert!(!quiet_branches.is_empty());
    for (class, width) in &quiet_branches {
        assert!(class.contains("is-cofactor"), "{class}");
        assert_eq!(*width, 7.5 * QUIET_COFACTORS.branch_width);
    }
    assert!(branch_widths(&full).iter().all(|(class, width)| *width == 7.5 && !class.contains("is-cofactor")));
}

#[test]
fn an_unknown_style_is_the_default() {
    let mut map = gapd();
    map.extra.insert("cofactorStyle".into(), Value::from("loud"));
    assert!(!is_quiet_cofactors(&map));
    assert_eq!(render_map_svg(&map, &RenderState::default()), render_map_svg(&gapd(), &RenderState::default()));
}
