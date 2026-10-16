//! The SVG of every render case equals, byte for byte, what the JavaScript
//! `renderMap` wrote for the same map and state (rust/tools/fixtures-render.mjs),
//! but for what d7b2c0e changed on purpose (`common::before_shapes`).

mod common;

use std::collections::HashMap;
use std::time::Instant;

use coxeter_render::render_map_svg;

fn run_cases(large: bool) -> usize {
    let Some(cases) = common::cases(large) else {
        eprintln!("no {} render fixtures; run rust/tools/fixtures-render.mjs", if large { "large" } else { "small" });
        return 0;
    };
    let mut maps = HashMap::new();
    let mut failures = Vec::new();
    for (name, map_path, spec, svg_path) in &cases {
        let map = maps.entry(map_path.clone()).or_insert_with(|| common::load_map(map_path));
        let state = common::state_from_spec(spec);
        let started = Instant::now();
        let svg = render_map_svg(map, &state);
        let took = started.elapsed();
        let expected = std::fs::read_to_string(svg_path).expect("expected svg");
        if large {
            eprintln!("{name}: {} bytes in {:.1} ms", svg.len(), took.as_secs_f64() * 1e3);
        }
        let (svg, expected) = (common::before_shapes(&svg), common::before_shapes(&expected));
        if let Err(e) = std::panic::catch_unwind(|| common::assert_same_svg(&svg, &expected, name)) {
            let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
            failures.push(msg);
        }
    }
    if !failures.is_empty() {
        panic!("{} of {} cases differ:\n\n{}", failures.len(), cases.len(), failures.join("\n\n"));
    }
    cases.len()
}

#[test]
fn every_small_render_case_matches_the_javascript_svg() {
    let n = run_cases(false);
    assert!(n >= 60, "only {n} small cases");
}

#[test]
fn every_large_render_case_matches_the_javascript_svg() {
    run_cases(true);
}

/// Not a test: how long the largest map takes to draw.
///
///   cargo test --release -p coxeter-render --test parity_render -- --ignored --nocapture timing
#[test]
#[ignore]
fn timing_on_the_largest_map() {
    let Some(cases) = common::cases(true) else {
        eprintln!("no large fixtures");
        return;
    };
    for wanted in ["gut9-default", "gut9-dark-crossfeed", "gut9-edit", "todas-default"] {
        let (name, map_path, spec, _) = cases.iter().find(|c| c.0 == wanted).expect("a large case");
        let map = common::load_map(map_path);
        let state = common::state_from_spec(spec);
        let best = |f: &mut dyn FnMut()| {
            (0..15)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .fold(f64::INFINITY, f64::min)
        };
        let mut tree = coxeter_render::render_map(&map, &state);
        let mut svg = String::new();
        let build = best(&mut || tree = coxeter_render::render_map(&map, &state));
        let write = best(&mut || svg = tree.to_markup());
        let both = best(&mut || svg = render_map_svg(&map, &state));
        let mut styled = 0;
        let style = best(&mut || styled = coxeter_render::map_stylesheet().resolve(&tree, &Default::default()).len());
        eprintln!(
            "{name}: {} reactions, {} nodes, {} segments -> {} elements, {:.1} MB of SVG; tree {build:.1} ms, markup {write:.1} ms, \
             render_map_svg {both:.1} ms, computed styles of {styled} elements {style:.1} ms (best of 15)",
            map.reactions.len(),
            map.nodes.len(),
            map.segments.len(),
            tree.descendants().len(),
            svg.len() as f64 / 1e6
        );
    }
}
