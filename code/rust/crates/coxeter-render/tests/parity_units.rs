//! Theme, reaction colours, reaction paths, raster sizes and cross-feeding
//! routes, compared with what the JavaScript modules returned
//! (rust/fixtures/render/units.json, rust/fixtures/large/render/units-routes.json).

mod common;

use coxeter_core::js;
use coxeter_core::mapdata::{Node, Point};
use coxeter_core::testkit::assert_json_eq;
use coxeter_render::crossfeed_routes::{cross_feeding_line_width, medium_endpoints, route_anchor, route_cross_feeding, route_grid, rounded_path};
use coxeter_render::reaction_colors::{build_reaction_coloring, category_color, normalize_reaction_color_mode, reaction_category};
use coxeter_render::reaction_path::{endpoint_node_radius, reaction_endpoint_geometry};
use coxeter_render::theme::{
    FluxScale, FluxScaleOptions, compartment_style, flux_ramp, format_flux, normalize_flux_color_stops_value, sample_ramp,
};
use coxeter_render::{frame_label_scale, raster_scales, rotation_arm};
use serde_json::{Value, json};

fn units() -> Value {
    common::read_json(&common::dir(false).join("units.json")).expect("run rust/tools/fixtures-render.mjs")
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => match s.as_str() {
            "NaN" => f64::NAN,
            other => js::string_to_number(other),
        },
        Value::Null => f64::NAN,
        other => js::to_number(other),
    }
}

/// A JSON number as JSON.stringify writes it (NaN and ±Infinity are null).
fn jnum(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

#[test]
fn flux_scales_match() {
    let u = units();
    let value_sets: Vec<(&str, Vec<f64>)> = vec![
        ("empty", vec![]),
        ("one", vec![5.0]),
        ("mixed", vec![-20.0, 0.0, 5.0, 100.0, 0.0001, -0.003, 12.5, 7.0, 7.0, 1e-10, 3.3333]),
        ("ramp", (0..60).map(|i| (if i % 2 == 1 { -1.0 } else { 1.0 }) * js::pow(1.31, i as f64) * 0.01).collect()),
    ];
    let stop_sets = json!({
        "none": null,
        "signed": [{"value": -20, "color": "#ff0000"}, {"value": 0, "color": "#ffffff"}, {"value": 5, "color": "#0000ff"}],
        "single": [{"value": -3, "color": "#123456"}],
        "wide": [{"value": -1e308, "color": "#000000"}, {"value": 1e308, "color": "#ffffff"}],
        "unsorted": [{"value": 10, "color": "#FFFFFF"}, {"value": -100, "color": " #000000 "}, {"value": 0, "color": "#ff0000"}],
    });
    let mut checked = 0;
    for expected in u["scales"].as_array().unwrap() {
        let values = &value_sets.iter().find(|(n, _)| *n == expected["values"]).unwrap().1;
        let stops = normalize_flux_color_stops_value(&stop_sets[expected["stops"].as_str().unwrap()]).unwrap();
        let dark = expected["dark"].as_bool().unwrap();
        let s = FluxScale::new(values.clone(), FluxScaleOptions { dark, color_stops: stops, ..Default::default() }).unwrap();
        let samples: Vec<Value> = expected["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                let v = num(&x["v"]);
                json!({"v": x["v"], "position": jnum(s.position(v)), "width": jnum(s.width(v)), "color": s.color(v)})
            })
            .collect();
        let legend: Vec<Value> = s
            .legend_stops(6)
            .iter()
            .map(|l| json!({"t": jnum(l.t), "value": jnum(l.value), "color": l.color, "width": jnum(l.width)}))
            .collect();
        let actual = json!({
            "values": expected["values"], "stops": expected["stops"], "dark": dark,
            "count": s.count, "min": s.min, "max": s.max, "reference": s.reference,
            "samples": samples, "legend": legend,
        });
        assert_json_eq(&actual, expected, 0.0, "FluxScale");
        checked += 1;
    }
    assert_eq!(checked, 40);
}

#[test]
fn colour_stop_errors_match() {
    for case in units()["stopErrors"].as_array().unwrap() {
        let got = normalize_flux_color_stops_value(&case["input"]);
        match (&got, case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.to_string(), msg.as_str().unwrap()),
            (Ok(Some(stops)), None) => {
                let as_json: Vec<Value> = stops.iter().map(|s| json!({"value": s.value, "color": s.color})).collect();
                assert_json_eq(&json!(as_json), &case["ok"], 0.0, "normalized stops");
            }
            other => panic!("{}: {other:?} vs {}", case["input"], case),
        }
    }
}

#[test]
fn ramps_formats_and_compartment_styles_match() {
    let u = units();
    for (name, dark) in [("light", false), ("dark", true)] {
        let got: Vec<String> = (-5..=105).map(|i| sample_ramp(flux_ramp(dark), i as f64 / 100.0)).collect();
        assert_json_eq(&json!(got), &u["ramps"][name], 0.0, "sampleRamp");
    }
    for case in u["fluxFormats"].as_array().unwrap() {
        let v = if case["v"].is_null() { None } else { Some(num(&case["v"])) };
        let digits = case["digits"].as_u64().unwrap() as usize;
        assert_eq!(format_flux(v, digits), case["text"].as_str().unwrap(), "formatFlux({}, {digits})", case["v"]);
    }
    for case in u["compartmentStyles"].as_array().unwrap() {
        let s = compartment_style(case["id"].as_str().unwrap(), case["dark"].as_bool().unwrap());
        assert_json_eq(&json!({"fill": s.fill, "stroke": s.stroke, "text": s.text}), &case["style"], 0.0, "compartmentStyle");
    }
}

#[test]
fn category_colours_and_colourings_match() {
    let u = units();
    for case in u["categories"].as_array().unwrap() {
        assert_eq!(
            category_color(case["label"].as_str().unwrap(), case["dark"].as_bool().unwrap()),
            case["color"].as_str().unwrap(),
            "categoryColor({})",
            case["label"]
        );
    }
    let fixtures = coxeter_core::testkit::fixture_path("core");
    for case in u["colorings"].as_array().unwrap() {
        let name = case["model"].as_str().unwrap();
        let raw = common::read_json(&fixtures.join(format!("model-{name}.json"))).expect("core fixture");
        let model = coxeter_core::Model::from_value(&raw["json"]).unwrap();
        let mode = normalize_reaction_color_mode(case["mode"].as_str().unwrap());
        let dark = case["dark"].as_bool().unwrap();
        let ids: std::collections::HashSet<String> =
            model.reactions.keys().enumerate().filter(|(i, _)| i % 3 == 0).map(|(_, k)| k.clone()).collect();
        let coloring = build_reaction_coloring(Some(&model), mode, if case["limited"].as_bool().unwrap() { Some(&ids) } else { None }, dark);
        let legend: Vec<Value> =
            coloring.legend.iter().map(|l| json!({"label": l.label, "count": l.count, "color": l.color})).collect();
        let categories: serde_json::Map<String, Value> = model
            .reactions
            .iter()
            .take(20)
            .map(|(id, r)| (id.clone(), json!(reaction_category(r, mode))))
            .collect();
        let actual = json!({
            "model": name, "mode": case["mode"], "dark": dark, "limited": case["limited"],
            "colors": coloring.colors, "groups": coloring.groups, "legend": legend, "categories": categories,
        });
        assert_json_eq(&actual, case, 0.0, &format!("buildReactionColoring {name}"));
    }
}

#[test]
fn reaction_endpoint_geometry_matches() {
    for case in units()["paths"].as_array().unwrap() {
        let points: Vec<Point> = case["points"].as_array().unwrap().iter().map(|p| Point::new(num(&p["x"]), num(&p["y"]))).collect();
        let radius = case["node"].as_object().map(|n| {
            let mut node = json!({"id": "n", "x": 0, "y": 0});
            for (k, v) in n {
                node[k] = v.clone();
            }
            endpoint_node_radius(&serde_json::from_value::<Node>(node).unwrap())
        });
        let g = reaction_endpoint_geometry(&points, radius, case["arrow"].as_bool().unwrap(), num(&case["width"]));
        let head = g.arrow.map(|a| {
            let p = |p: Point| json!({"x": p.x, "y": p.y});
            json!({"tip": p(a.tip), "base": p(a.base), "points": a.points.iter().map(|q| p(*q)).collect::<Vec<_>>()})
        });
        assert_eq!(g.d, case["d"].as_str().unwrap(), "path {case}");
        assert_json_eq(&json!(head), &case["arrow_head"], 0.0, "arrowhead");
    }
}

#[test]
fn raster_scales_and_zoom_helpers_match() {
    let u = units();
    for case in u["rasters"].as_array().unwrap() {
        let got = raster_scales(num(&case["w"]), num(&case["h"]), 2.0);
        assert_json_eq(&json!(got), &case["scales"], 0.0, "rasterScales");
    }
    for case in u["misc"]["frameLabelScale"].as_array().unwrap() {
        assert_eq!(frame_label_scale(num(&case["m"])), num(&case["v"]));
    }
    for case in u["misc"]["rotationArm"].as_array().unwrap() {
        assert_eq!(rotation_arm(num(&case["m"])), num(&case["v"]));
    }
}

#[test]
fn rounded_paths_and_anchors_match() {
    for case in units()["rounded"].as_array().unwrap() {
        let points: Vec<Point> = case["points"].as_array().unwrap().iter().map(|p| Point::new(num(&p["x"]), num(&p["y"]))).collect();
        assert_eq!(rounded_path(&points, num(&case["radius"])), case["d"].as_str().unwrap());
        let at = case["at"].as_object().map(|p| Point::new(num(&p["x"]), num(&p["y"])));
        let a = route_anchor(&points, at);
        assert_json_eq(&json!({"x": a.x, "y": a.y}), &case["anchor"], 0.0, "routeAnchor");
    }
}

#[test]
fn cross_feeding_routes_match_on_the_gut9_map() {
    let dir = common::dir(true);
    let Some(u) = common::read_json(&dir.join("units-routes.json")) else {
        eprintln!("no large fixtures; skipped");
        return;
    };
    let map = common::load_map(&dir.join(u["map"].as_str().unwrap()));
    let endpoint = |e: &coxeter_render::crossfeed_routes::Endpoint| {
        let mut v = json!({"x": e.x, "y": e.y, "nx": e.nx, "ny": e.ny, "gap": e.gap, "r": e.r});
        if let Some(n) = &e.name {
            v["name"] = json!(n);
        }
        v
    };
    let ends: serde_json::Map<String, Value> = medium_endpoints(&map).iter().map(|(k, e)| (k.clone(), endpoint(e))).collect();
    // JS endpoints carry `name: null` where the node has none; compare without the nulls.
    let strip = |v: &Value| -> Value {
        let mut v = v.clone();
        for e in v.as_object_mut().unwrap().values_mut() {
            if e["name"].is_null() {
                e.as_object_mut().unwrap().remove("name");
            }
        }
        v
    };
    assert_json_eq(&Value::Object(ends), &strip(&u["endpoints"]), 0.0, "mediumEndpoints");
    for set in u["routeSets"].as_array().unwrap() {
        assert_eq!(route_grid(&map), num(&set["grid"]));
        let links: Vec<_> = set["links"].as_array().unwrap().iter().map(common::link).collect();
        let routes = route_cross_feeding(&map, &links, cross_feeding_line_width(&map, 1.0));
        let mut actual = serde_json::Map::new();
        for (key, r) in routes.iter() {
            let pts: Vec<Value> = r.points.iter().map(|p| json!({"x": p.x, "y": p.y})).collect();
            let mut start = endpoint(&r.start);
            let mut end = endpoint(&r.end);
            for e in [&mut start, &mut end] {
                if e.get("name").is_none() && e["r"] != json!(0.0) {
                    e["name"] = Value::Null;
                }
            }
            actual.insert(key.clone(), json!({"points": pts, "start": start, "end": end}));
        }
        let mut expected = set["routes"].clone();
        for r in expected.as_object_mut().unwrap().values_mut() {
            for k in ["start", "end"] {
                if r[k]["name"].is_null() {
                    r[k].as_object_mut().unwrap().remove("name");
                }
            }
        }
        for r in actual.values_mut() {
            for k in ["start", "end"] {
                if r[k]["name"].is_null() {
                    r[k].as_object_mut().unwrap().remove("name");
                }
            }
        }
        assert_json_eq(&Value::Object(actual), &expected, 0.0, "routeCrossFeeding");
    }
}
