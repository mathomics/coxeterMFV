//! Hit-testing against the browser: what Firefox's `elementFromPoint` finds
//! under a grid of stage points (11 px apart, three zooms, `tests/fixtures/
//! hits.json`) against what [`Stage::target_at`] finds in the same drawing.
//!
//! Exact agreement everywhere is not expected: along an edge the browser
//! decides by its own rasterised geometry (flattened curves, stroke joins and
//! caps), the native side by exact shapes. The test requires agreement on
//! nearly every point and reports what differs.
//!
//! One difference is Firefox's own: the map is zoomed by a CSS transform on
//! its `<svg>`, and Firefox measures a `vector-effect: non-scaling-stroke`
//! width (the cell outline's 3 px) before that transform, so the outline is
//! 0.8 px wide at 27 % there. The stylesheet means constant screen pixels
//! ("the same few pixels at every zoom"), which is what Chromium and the
//! native painter and hit-test do. Those points are counted apart.

mod common;

use std::collections::BTreeMap;

use coxeter_canvas::camera::{Camera, StageRect};
use coxeter_canvas::gestures::Stage;
use coxeter_canvas::host::MapHost;
use coxeter_canvas::{MapCanvas, Tool};
use coxeter_core::mapdata::Point;
use serde_json::{Value, json};

use common::*;

fn hits() -> Option<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hits.json");
    Some(serde_json::from_str(&std::fs::read_to_string(path).ok()?).expect("hits.json"))
}

fn label(v: &Value) -> String {
    if v.is_null() {
        return "nothing".into();
    }
    let cls = v["cls"].as_str().unwrap_or("").split_ascii_whitespace().next().unwrap_or("").to_string();
    format!("{}.{}", v["tag"].as_str().unwrap_or("?"), cls)
}

#[test]
fn hit_testing_matches_the_browser() {
    let (Some(doc), Some(grids)) = (gestures(), hits()) else {
        eprintln!("skipped: tests/fixtures/{{gestures,hits}}.json not written (tools/fixtures-canvas.mjs)");
        return;
    };
    let mut total = 0usize;
    let mut agree = 0usize;
    for grid in grids["hits"].as_array().unwrap() {
        let mut host = host_for(&doc, &json!({ "start": "initial" }));
        let mut canvas = MapCanvas::new();
        let st = &grid["stage"];
        canvas.set_rect(StageRect::new(
            st["left"].as_f64().unwrap(),
            st["top"].as_f64().unwrap(),
            st["width"].as_f64().unwrap(),
            st["height"].as_f64().unwrap(),
        ));
        let cam = &grid["camera"];
        canvas.camera = Camera { x: cam["x"].as_f64().unwrap(), y: cam["y"].as_f64().unwrap(), scale: cam["scale"].as_f64().unwrap(), fitted: true };
        canvas.activate_tool(Tool::parse(grid["tool"].as_str().unwrap()).unwrap(), &mut host);
        if let Some(r) = grid["selected"].as_str() {
            host.select(Some(r));
        }
        canvas.sync(&host, host.emitted.len() as u64, 0.0);
        let targets = grid["targets"].as_array().unwrap();
        let filter = canvas.gestures.hit_filter();
        let mut camera = canvas.camera;
        let stage = Stage { camera: &mut camera, rect: canvas.rect, scene: canvas.scene.as_ref() };
        let mut confusions: BTreeMap<(String, String), usize> = BTreeMap::new();
        let (mut n, mut ok, mut firefox) = (0, 0, 0);
        let t0 = std::time::Instant::now();
        for p in grid["points"].as_array().unwrap() {
            let at = Point::new(p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
            let js = &targets[p[2].as_u64().unwrap() as usize];
            let rust = describe(&stage.target_at(at, filter));
            n += 1;
            if same_target(&rust, js) {
                ok += 1;
            } else if rust["cls"].as_str().is_some_and(|c| c.contains("emc-cell-boundary")) && js["cls"].as_str().is_some_and(|c| c.contains("emc-boundary-hit")) && js["data"]["data-boundary"] == rust["data"]["data-boundary"] {
                firefox += 1;
            } else {
                *confusions.entry((label(js), label(&rust))).or_default() += 1;
                if std::env::var("HIT_DETAIL").is_ok() && js["tag"] == "text" {
                    eprintln!("      ({}, {}): browser {} / Rust {}", at.x, at.y, js, rust);
                }
            }
        }
        let per_point = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        eprintln!(
            "scale {} ({}): {ok} of {n} points agree ({:.2}%), {firefox} on a non-scaling outline, {per_point:.1} µs a point",
            cam["scale"],
            grid["tool"],
            100.0 * ok as f64 / n as f64
        );
        n -= firefox;
        let mut worst: Vec<_> = confusions.into_iter().collect();
        worst.sort_by_key(|w| std::cmp::Reverse(w.1));
        for ((js, rust), count) in worst.iter().take(8) {
            eprintln!("    browser {js} / Rust {rust}: {count}");
        }
        total += n;
        agree += ok;
    }
    let rate = agree as f64 / total as f64;
    eprintln!("overall {agree} of {total} ({:.2}%)", 100.0 * rate);
    // Along every edge a point can fall either way: a few points in a
    // thousand lie within a pixel of one on these maps. 99.5 % is that margin;
    // anything systematic (a missing target, a wrong order, a label's box)
    // costs more than it.
    assert!(rate >= 0.995, "only {:.2}% of the points agree with the browser", 100.0 * rate);
}
