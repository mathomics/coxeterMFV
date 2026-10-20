//! The render assertions of the JavaScript test suite (`../test/*.test.mjs`),
//! on the same maps (written by rust/tools/fixtures-render.mjs, since the maps
//! are built by the editing code of another crate).

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use coxeter_core::community::CrossFeedingLink;
use coxeter_core::mapdata::{MapData, MapGraph, Node, NodeKind, Point};
use coxeter_core::testkit::repo_root;
use coxeter_render::crossfeed_routes::{is_drawn_cross_feeding_base, medium_endpoints, route_cross_feeding};
use coxeter_render::reaction_colors::{ReactionColorMode, build_reaction_coloring, reaction_category};
use coxeter_render::reaction_path::{endpoint_node_radius, reaction_endpoint_geometry};
use coxeter_render::theme::{FluxColorStop, FluxScale, FluxScaleOptions, normalize_flux_color_stops, normalize_flux_color_stops_value};
use coxeter_render::{
    CrossFeedingEvent, CrossFeedingExplorerView, GeometrySelection, LabelMode, MultiSelection, PathwayRibbonMode,
    ReactionWidthMode, RenderState, Scale, frame_label_scale, render_map_svg,
};
use regex::Regex;
use serde_json::{Value, json};

/// The routes cache is one for the process; tests that route take turns.
static ROUTING: Mutex<()> = Mutex::new(());

fn map(name: &str) -> MapGraph {
    common::load_map(&common::dir(false).join(format!("maps/{name}.json")))
}

/// The state a fixture case was rendered with.
fn case_state(name: &str) -> Value {
    common::cases(false).unwrap().into_iter().find(|c| c.0 == name).unwrap_or_else(|| panic!("no case {name}")).2
}

/// A flux scale given as functions, as the JS tests write theirs.
struct TestScale {
    reference: f64,
    width: fn(f64) -> f64,
    color: fn(f64) -> String,
}

impl Scale for TestScale {
    fn reference(&self) -> f64 {
        self.reference
    }
    fn width(&self, v: f64) -> f64 {
        (self.width)(v)
    }
    fn color(&self, v: f64) -> String {
        (self.color)(v)
    }
}

fn fluxes(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn set(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn all(re: &str, svg: &str) -> Vec<Vec<String>> {
    Regex::new(re)
        .unwrap()
        .captures_iter(svg)
        .map(|c| c.iter().map(|m| m.map_or(String::new(), |m| m.as_str().to_string())).collect())
        .collect()
}

// escher-style-render

fn escher_scale() -> Arc<TestScale> {
    Arc::new(TestScale {
        reference: 1.0,
        width: |v| if v.abs() > 1.0 { 14.0 } else { 1.1 },
        color: |v| if v.abs() > 1.0 { "#be123c".into() } else { "#cbd5e1".into() },
    })
}

fn escher_render(m: &MapGraph, gapd: f64, extra: impl FnOnce(&mut RenderState)) -> String {
    let mut s = RenderState {
        fluxes: fluxes(&[("eco@GAPD", gapd)]),
        scale: escher_scale(),
        show_currency: true,
        currency_labels: true,
        ..Default::default()
    };
    extra(&mut s);
    render_map_svg(m, &s)
}

#[test]
fn reaction_flux_changes_colour_but_never_changes_stroke_width() {
    let m = map("gapd-only");
    let a = escher_render(&m, 0.1, |_| {});
    let b = escher_render(&m, 100.0, |_| {});
    let widths = |svg: &str| -> Vec<f64> {
        all(r#"class="emc-seg[^"]*"[^>]*stroke-width="([^"]+)""#, svg).iter().map(|c| c[1].parse().unwrap()).collect()
    };
    let low = widths(&a);
    assert!(!low.is_empty());
    assert_eq!(low, widths(&b));
    assert!(low.iter().all(|w| *w == 7.5));
    assert!(a.contains(r##"stroke="#cbd5e1""##));
    assert!(b.contains(r##"stroke="#be123c""##));
}

#[test]
fn secondary_metabolites_render_as_small_escher_style_orange_circles_with_labels() {
    let m = map("gapd-only");
    let svg = escher_render(&m, 10.0, |_| {});
    let satellites: Vec<&Node> = m.nodes.values().filter(|n| n.kind == NodeKind::Currency).collect();
    assert!(satellites.len() >= 3);
    assert!(satellites.iter().all(|n| n.r == 7.0));
    assert!(svg.contains(r##"fill="#e67e4c""##));
    let circles = all(r#"class="emc-node[^"]*"[^>]*r="([^"]+)"[^>]*fill="([^"]+)""#, &svg);
    assert!(!circles.is_empty());
    assert!(circles.iter().all(|c| ["7", "13"].contains(&c[1].as_str())));
    assert!(circles.iter().all(|c| c[2] == "#e67e4c"));
    for name in [">nad<", ">nadh<", ">h<"] {
        assert!(svg.contains(name), "{name}");
    }
}

#[test]
fn flux_direction_uses_metabolite_arrowheads_and_never_adds_a_central_backbone_arrow() {
    let m = map("gapd-only");
    let heads = |svg: &str| -> Vec<String> {
        let mut h: Vec<String> = all(r#"class="emc-arrow"[^>]*data-node="([^"]+)""#, svg).into_iter().map(|c| c[1].clone()).collect();
        h.sort();
        h
    };
    let side_nodes = |side: &str| -> Vec<String> {
        let mut v: Vec<String> =
            m.segments.iter().filter(|s| s.side.as_deref() == Some(side)).filter_map(|s| s.node_id.clone()).collect();
        v.sort();
        v
    };
    let positive = escher_render(&m, 8.0, |_| {});
    assert_eq!(heads(&positive), side_nodes("product"));
    assert!(!positive.contains("emc-arrow-flux"));
    let negative = escher_render(&m, -8.0, |_| {});
    assert_eq!(heads(&negative), side_nodes("substrate"));
    let zero = escher_render(&m, 0.0, |_| {});
    assert!(heads(&zero).is_empty());
    assert!(!zero.contains("emc-arrow-flux"));
}

#[test]
fn primary_and_secondary_branches_and_their_heads_use_one_colour_and_opacity() {
    let m = map("gapd-only");
    let svg = escher_render(&m, 8.0, |s| {
        s.reaction_color_mode = ReactionColorMode::Subsystem;
        s.reaction_colors = [("eco@GAPD".to_string(), "#234abc".to_string())].into();
        s.cross_feeding_explorer = Some(CrossFeedingExplorerView::default());
        s.show_cross_feeding = true;
    });
    let paths = all(r#"<path class="emc-seg[^"]*"[^>]*stroke="([^"]+)"[^>]*opacity="([^"]+)""#, &svg);
    let heads = all(r#"<polygon class="emc-arrow"[^>]*fill="([^"]+)"[^>]*opacity="([^"]+)""#, &svg);
    assert!(!paths.is_empty() && !heads.is_empty());
    assert!(paths.iter().chain(&heads).all(|c| c[1] == "#234abc" && c[2] == "0.14"));
    let css = std::fs::read_to_string(repo_root().join("assets/map.css")).unwrap();
    assert!(!Regex::new(r"\.emc-seg-currency\s*\{[^}]*opacity\s*:").unwrap().is_match(&css));
}

#[test]
fn selected_reactions_retain_matching_branch_and_arrow_opacity_while_cross_feeding_is_highlighted() {
    let m = map("gapd-only");
    for multi in [false, true] {
        let svg = escher_render(&m, 8.0, |s| {
            s.cross_feeding_explorer = Some(CrossFeedingExplorerView::default());
            s.show_cross_feeding = true;
            if multi {
                s.multi_selection = Some(MultiSelection { reactions: set(&["eco@GAPD"]), ..Default::default() });
            } else {
                s.selected_reaction = Some("eco@GAPD".into());
            }
        });
        let paths = all(r#"<path class="emc-seg[^"]*"[^>]*opacity="([^"]+)""#, &svg);
        let heads = all(r#"<polygon class="emc-arrow"[^>]*opacity="([^"]+)""#, &svg);
        assert!(!paths.is_empty() && !heads.is_empty());
        assert!(paths.iter().chain(&heads).all(|c| c[1] == "1"));
    }
}

fn test_node(kind: &str, primary: Option<bool>) -> f64 {
    let mut v = json!({"id": "n", "kind": kind, "x": 0, "y": 0});
    if let Some(p) = primary {
        v["primary"] = json!(p);
    }
    endpoint_node_radius(&serde_json::from_value::<Node>(v).unwrap())
}

#[test]
fn the_shaft_ends_at_the_arrow_base_and_leaves_a_clear_gap_before_either_node_size() {
    let path_end = Regex::new(r"L ([\d.-]+) ([\d.-]+)$").unwrap();
    let first_l = Regex::new(r"L ([\d.-]+)").unwrap();
    for (kind, primary) in [("metabolite", Some(true)), ("currency", Some(false)), ("metabolite", Some(false))] {
        let points = [Point::new(0.0, 0.0), Point::new(100.0, 0.0)];
        let radius = test_node(kind, primary);
        let g = reaction_endpoint_geometry(&points, Some(radius), true, 7.5);
        let expected_radius = if primary == Some(true) { 13.0 } else { 7.0 };
        let arrow = g.arrow.unwrap();
        assert!((100.0 - arrow.tip.x - expected_radius - 4.0).abs() < 1e-6);
        assert_eq!(arrow.tip.y, 0.0);
        assert!(arrow.base.x < arrow.tip.x);
        let end = path_end.captures(&g.d).unwrap();
        assert!((end[1].parse::<f64>().unwrap() - arrow.base.x).abs() < 0.01);
        assert_eq!(end[2].parse::<f64>().unwrap(), arrow.base.y);
        let no_head = reaction_endpoint_geometry(&points, Some(radius), false, 7.5);
        assert!(no_head.arrow.is_none());
        let x: f64 = first_l.captures(&no_head.d).unwrap()[1].parse().unwrap();
        assert!((x - (100.0 - expected_radius - 4.0)).abs() < 0.01);
    }
}

#[test]
fn curved_branches_keep_their_bezier_shape_while_clipping_away_the_node_end_stretch() {
    let points = [Point::new(0.0, 0.0), Point::new(30.0, 60.0), Point::new(80.0, 60.0), Point::new(100.0, 0.0)];
    let g = reaction_endpoint_geometry(&points, Some(test_node("metabolite", None)), true, 7.5);
    assert!(g.d.starts_with("M 0 0 C "));
    let a = g.arrow.unwrap();
    assert!((coxeter_core::js::hypot(a.tip.x - 100.0, a.tip.y) - 17.0).abs() < 1e-6);
    assert!((coxeter_core::js::hypot(a.tip.x - a.base.x, a.tip.y - a.base.y) - 17.8125).abs() < 1e-6);
    assert!(a.tip.y > 0.0, "the tip follows the edited curve, rather than a straight chord");
}

#[test]
fn boundary_arrows_also_have_no_shaft_beyond_the_tip_and_collapsed_branches_do_not_draw_inside_nodes() {
    let g = reaction_endpoint_geometry(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], None, true, 7.5);
    let a = g.arrow.unwrap();
    assert!((a.tip.x - 98.0).abs() < 1e-6);
    assert!(a.base.x < 98.0);
    let collapsed =
        reaction_endpoint_geometry(&[Point::new(96.0, 0.0), Point::new(100.0, 0.0)], Some(test_node("currency", None)), true, 7.5);
    assert_eq!(collapsed.d, "");
    assert!(collapsed.arrow.is_none());
}

// crossfeed-routes

fn gut9() -> MapGraph {
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(repo_root().join("models/published/steadycom-gut-nine/map.json")).unwrap())
            .unwrap();
    MapGraph::from_data(serde_json::from_value::<MapData>(raw["map"].clone()).unwrap())
}

fn links_of(m: &MapGraph, pairs: &[(&str, &str, &str)]) -> Vec<CrossFeedingLink> {
    let ends = medium_endpoints(m);
    pairs
        .iter()
        .filter(|(from, base, to)| ends.contains_key(&format!("{from}|{base}")) && ends.contains_key(&format!("{to}|{base}")))
        .map(|(from, base, to)| CrossFeedingLink { base: base.to_string(), from: Some(from.to_string()), to: Some(to.to_string()), flux: 1.0 })
        .collect()
}

#[test]
fn protons_and_water_get_no_cross_feeding_line() {
    assert!(!is_drawn_cross_feeding_base("h"));
    assert!(!is_drawn_cross_feeding_base("h2o"));
    assert!(!is_drawn_cross_feeding_base("cpd00067"));
    assert!(is_drawn_cross_feeding_base("ac"));
}

#[test]
fn a_cross_feeding_line_runs_from_the_producers_medium_node_to_the_consumers_outside_every_cell() {
    let _turn = ROUTING.lock().unwrap_or_else(|e| e.into_inner());
    let m = gut9();
    let ends = medium_endpoints(&m);
    let links = links_of(
        &m,
        &[("Bt", "ac", "Fp"), ("St", "lac-L", "Bt"), ("Kp", "nh4", "St"), ("Ef", "gln-L", "Lc"), ("Ec", "asp-L", "Ef"), ("Bt", "h", "Fp")],
    );
    let routes = route_cross_feeding(&m, &links);
    assert!(!routes.contains_key("Bt|h|Fp"), "no line for protons");
    let inside = |p: Point, r: &coxeter_core::mapdata::Rect| p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h;
    for link in links.iter().filter(|l| l.base != "h") {
        let (from, to) = (link.from.as_deref().unwrap(), link.to.as_deref().unwrap());
        let route = routes.get(&format!("{from}|{}|{to}", link.base)).unwrap_or_else(|| panic!("{} {from}→{to} is routed", link.base));
        let a = &ends[&format!("{from}|{}", link.base)];
        let b = &ends[&format!("{to}|{}", link.base)];
        assert_eq!(route.points[0], Point::new(a.x, a.y), "starts at the producer's node");
        assert_eq!(*route.points.last().unwrap(), Point::new(b.x, b.y), "ends at the consumer's node");
        for w in route.points.windows(2) {
            assert!((w[0].x - w[1].x).abs() < 1e-6 || (w[0].y - w[1].y).abs() < 1e-6, "runs are straight");
        }
        let n = route.points.len();
        for i in 1..n - 1 {
            let p = route.points[i];
            let q = route.points[i + 1];
            let mid = Point::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0);
            for cell in &m.cells {
                assert!(!inside(p, &cell.rect), "{}: corner inside {}", link.base, cell.id);
                if i < n - 2 {
                    assert!(!inside(mid, &cell.rect), "{}: run across {}", link.base, cell.id);
                }
            }
        }
    }
}

#[test]
fn routes_are_reused_while_the_same_links_exist() {
    let _turn = ROUTING.lock().unwrap_or_else(|e| e.into_inner());
    let m = gut9();
    let links = links_of(&m, &[("Bt", "ac", "Fp"), ("St", "lac-L", "Bt")]);
    let first = route_cross_feeding(&m, &links);
    let again = route_cross_feeding(&m, &links.iter().map(|l| CrossFeedingLink { flux: 7.0, ..l.clone() }).collect::<Vec<_>>());
    assert!(Arc::ptr_eq(&first, &again), "a new flux does not re-route");
}

// flux-color-stops

fn stops() -> Vec<FluxColorStop> {
    [(-20.0, "#ff0000"), (0.0, "#ffffff"), (5.0, "#0000ff")]
        .iter()
        .map(|(v, c)| FluxColorStop { value: *v, color: c.to_string() })
        .collect()
}

#[test]
fn custom_flux_stops_are_sorted_and_owned_copies_with_no_arbitrary_point_limit() {
    let s = stops();
    let input = vec![s[2].clone(), s[0].clone(), s[1].clone()];
    let original = input.clone();
    let normalized = normalize_flux_color_stops(&input).unwrap();
    assert_eq!(normalized, s);
    assert_eq!(input, original, "normalizing must not reorder caller input");
    let many: Vec<FluxColorStop> = (0..50).map(|i| FluxColorStop { value: i as f64 - 25.0, color: "#123456".into() }).collect();
    assert_eq!(normalize_flux_color_stops(&many).unwrap().len(), 50);
    assert_eq!(normalize_flux_color_stops_value(&Value::Null).unwrap(), None);
}

#[test]
fn invalid_flux_stops_are_rejected_instead_of_becoming_a_misleading_palette() {
    for input in [
        json!([]),
        json!({"value": 0, "color": "#ffffff"}),
        json!([{"value": 0, "color": "#ffffff"}, {"value": 0, "color": "#000000"}]),
        json!([{"value": "not a number", "color": "#ffffff"}]),
        json!([{"value": 0, "color": "#fff"}]),
        json!([{"value": 0, "color": "#gg0000"}]),
        json!([{"value": 0, "color": "red"}]),
    ] {
        assert!(normalize_flux_color_stops_value(&input).is_err(), "{input}");
    }
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(normalize_flux_color_stops(&[FluxColorStop { value: v, color: "#ffffff".into() }]).is_err());
    }
}

fn stop_scale(values: &[f64], stops: Vec<FluxColorStop>) -> FluxScale {
    FluxScale::new(values.to_vec(), FluxScaleOptions { color_stops: Some(stops), ..Default::default() }).unwrap()
}

#[test]
fn negative_zero_and_positive_fluxes_use_their_exact_signed_color_anchors() {
    let scale = stop_scale(&[-10000.0, 10000.0], stops());
    for stop in stops() {
        assert_eq!(scale.color(stop.value), stop.color);
    }
    assert_ne!(scale.color(-5.0), scale.color(5.0), "negative flux must not become its absolute value");
    let legend: Vec<(f64, String)> = scale.legend_stops(6).into_iter().map(|l| (l.value, l.color)).collect();
    assert_eq!(legend, stops().into_iter().map(|s| (s.value, s.color)).collect::<Vec<_>>());
}

#[test]
fn rgb_interpolation_uses_real_flux_distances_rather_than_log_or_normalized_magnitude() {
    let palette: Vec<FluxColorStop> = [(-100.0, "#000000"), (0.0, "#ff0000"), (10.0, "#ffffff")]
        .iter()
        .map(|(v, c)| FluxColorStop { value: *v, color: c.to_string() })
        .collect();
    let scale = stop_scale(&[-10000.0, 1.0, 10000.0], palette.clone());
    assert_eq!(scale.color(-25.0), "#bf0000", "-25 is three quarters through [-100, 0]");
    assert_eq!(scale.color(5.0), "#ff8080", "5 is halfway through [0, 10]");
    let different = stop_scale(&[0.0001, 0.001], palette);
    assert_eq!(different.color(-25.0), scale.color(-25.0));
    assert_eq!(different.color(5.0), scale.color(5.0));
}

#[test]
fn custom_flux_colors_clamp_outside_anchors_and_one_anchor_gives_a_constant_color() {
    let scale = stop_scale(&[1.0], stops());
    assert_eq!(scale.color(-1e9), stops()[0].color);
    assert_eq!(scale.color(1e9), stops()[2].color);
    let constant = stop_scale(&[-10.0, 0.0, 10.0], vec![FluxColorStop { value: -3.0, color: "#123456".into() }]);
    for v in [-1e9, -3.0, 0.0, 1e9] {
        assert_eq!(constant.color(v), "#123456");
    }
}

#[test]
fn automatic_colors_keep_their_existing_behavior_and_custom_stops_never_change_flux_widths() {
    let values = [-20.0, 0.0, 5.0, 100.0];
    let automatic = FluxScale::automatic(values, false);
    let explicit = FluxScale::new(values, FluxScaleOptions::default()).unwrap();
    let custom = stop_scale(&values, stops());
    assert_eq!(automatic.color(0.0), "#cbd5e1");
    assert_eq!(FluxScale::automatic(values, true).color(0.0), "#475569");
    assert_eq!(automatic.color(20.0), automatic.color(-20.0));
    assert_eq!(automatic.color(automatic.reference), "#be123c");
    for v in [-1000.0, -20.0, 0.0, 5.0, 1000.0] {
        assert_eq!(explicit.color(v), automatic.color(v));
        assert_eq!(custom.width(v), automatic.width(v));
    }
}

// name-plates

#[derive(Debug)]
struct Plate {
    id: String,
    x: f64,
    y: f64,
    fit: f64,
    pill: (f64, f64, f64, f64),
    text_at: f64,
    text_length: f64,
    text: String,
}

fn plates(svg: &str) -> Vec<Plate> {
    let re = r#"<g class="emc-comp-label[^"]*" data-compartment-label="([^"]+)" transform="translate\(([-\d.]+) ([-\d.]+)\)">\s*<g class="emc-frame-label" style="--fit:([\d.]+)">\s*<rect x="([-\d.]+)" y="([-\d.]+)" width="([-\d.]+)" height="([-\d.]+)"[^>]*/>\s*<text x="([-\d.]+)" y="([-\d.]+)" text-anchor="middle" textLength="([-\d.]+)"[^>]*>([^<]*)</text>"#;
    all(re, svg)
        .into_iter()
        .map(|m| {
            let f = |i: usize| m[i].parse::<f64>().unwrap();
            Plate {
                id: m[1].clone(),
                x: f(2),
                y: f(3),
                fit: f(4),
                pill: (f(5), f(6), f(7), f(8)),
                text_at: f(9),
                text_length: f(11),
                text: m[12].clone(),
            }
        })
        .collect()
}

/// Where a plate is drawn on the map at a given zoom: (scale, x, y, w, h).
fn drawn(p: &Plate, zoom: f64) -> (f64, f64, f64, f64, f64) {
    let scale = frame_label_scale(12.0 / zoom).min(p.fit);
    (scale, p.x + p.pill.0 * scale, p.y + p.pill.1 * scale, p.pill.2 * scale, p.pill.3 * scale)
}

const ZOOMS: [f64; 6] = [2.0, 1.0, 0.5, 0.2, 0.1, 0.05];

fn plate_render(m: &MapGraph) -> String {
    render_map_svg(
        m,
        &RenderState {
            scale: Arc::new(TestScale { reference: 1.0, width: |_| 1.0, color: |_| "#94a3b8".into() }),
            ..Default::default()
        },
    )
}

#[test]
fn every_compartment_and_the_medium_is_named_each_text_cut_to_its_own_pill() {
    let m = map("alga");
    let svg = plate_render(&m);
    let found = plates(&svg);
    let mut ids: Vec<&str> = found.iter().map(|p| p.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["c", "e", "g", "h", "i", "m", "s", "u"]);
    for p in &found {
        assert!(p.text_length > 0.0);
        assert_eq!(coxeter_core::js::round(p.pill.2 - p.text_length), 18.0, "{}: the pill is cut to its text", p.text);
        assert!((p.text_at - (p.pill.0 + p.pill.2 / 2.0)).abs() < 0.01, "{}: and the text is centred in it", p.text);
    }
    assert!(svg.contains(r#"class="emc-frame-label" style="--fit:"#));
}

fn rect_of<'a>(m: &'a MapGraph, id: &str) -> &'a coxeter_core::mapdata::Rect {
    if id == "s" { &m.environment.rect } else { &m.cells[0].compartments.iter().find(|c| c.id == id).unwrap().rect }
}

#[test]
fn a_name_plate_stays_inside_its_own_compartment_at_every_zoom() {
    let m = map("alga");
    for p in plates(&plate_render(&m)) {
        let r = rect_of(&m, &p.id);
        for zoom in ZOOMS {
            let (_, x, _, w, _) = drawn(&p, zoom);
            assert!(x >= r.x - 0.01 && x + w <= r.x + r.w + 0.01, "{} at {}%", p.text, zoom * 100.0);
        }
    }
}

#[test]
fn plates_are_all_one_size_on_screen_until_a_compartment_is_too_small_for_that() {
    let m = map("alga");
    let found = plates(&plate_render(&m));
    let at = |zoom: f64| -> Vec<f64> { found.iter().map(|p| drawn(p, zoom).0).collect() };
    let distinct = |v: Vec<f64>| v.iter().map(|x| format!("{x:.3}")).collect::<HashSet<_>>().len();
    assert_eq!(distinct(at(1.0)), 1);
    assert_eq!(distinct(at(0.8)), 1);
    let max = |v: Vec<f64>| v.into_iter().fold(f64::MIN, f64::max);
    assert!(max(at(0.1)) > max(at(1.0)) * 2.0);
    for s in at(0.05) {
        assert!(s <= frame_label_scale(12.0 / 0.05) + 1e-9);
    }
    assert_eq!(frame_label_scale(12.0 / 2.0), 1.0, "zoomed in, a plate is its base size and no smaller");
}

#[test]
fn a_plate_keeps_out_of_the_compartments_nested_in_its_own() {
    let m = map("alga");
    let comps = &m.cells[0].compartments;
    for p in plates(&plate_render(&m)).iter().filter(|p| p.id != "s") {
        if ["m", "u", "g", "i"].contains(&p.id.as_str()) {
            continue; // these sit astride their own top edge
        }
        for child in comps.iter().filter(|c| c.parent_id.as_deref() == Some(p.id.as_str())) {
            for zoom in ZOOMS {
                let (_, x, y, w, h) = drawn(p, zoom);
                let c = &child.rect;
                let apart = x + w <= c.x || c.x + c.w <= x || y + h <= c.y || c.y + c.h <= y;
                assert!(apart, "{} runs into {} at {}%", p.text, child.id, zoom * 100.0);
            }
        }
    }
}

#[test]
fn a_compartment_is_named_even_when_one_nested_in_it_reaches_its_top_edge() {
    let m = map("alga-h-moved");
    let comps = &m.cells[0].compartments;
    let c = comps.iter().find(|c| c.id == "c").unwrap();
    let h = comps.iter().find(|c| c.id == "h").unwrap();
    assert!(h.rect.y - c.rect.y < 27.0);
    assert!(plates(&plate_render(&m)).iter().any(|p| p.id == "c" && p.text == "Cytosol"));
}

#[test]
fn two_plates_astride_closely_nested_top_edges_share_the_band_between_them() {
    let m = map("alga");
    let found = plates(&plate_render(&m));
    let comps = &m.cells[0].compartments;
    let band = comps.iter().find(|c| c.id == "m").unwrap().rect.y - comps.iter().find(|c| c.id == "i").unwrap().rect.y;
    let outer = found.iter().find(|p| p.id == "i").unwrap();
    let inner = found.iter().find(|p| p.id == "m").unwrap();
    for zoom in ZOOMS {
        let a = drawn(outer, zoom);
        let b = drawn(inner, zoom);
        assert!(a.2 + a.4 <= b.2 + 0.01, "written over each other at {}%", zoom * 100.0);
        assert!((a.4 + b.4) / 2.0 <= band + 0.01);
    }
}

#[test]
fn the_cell_is_named_on_a_plate_as_wide_as_its_name_held_to_the_cell_and_the_room_above_it() {
    let m = map("alga");
    let svg = plate_render(&m);
    let caps = all(
        r#"data-cell-plate="alga" transform="translate\(([-\d.]+) ([-\d.]+)\)">\s*<g class="emc-frame-label" style="--fit:([\d.]+)">\s*<rect x="0" y="-30" width="([-\d.]+)" height="30""#,
        &svg,
    );
    let c = caps.first().expect("the plate is drawn");
    let v: Vec<f64> = c[1..5].iter().map(|s| s.parse().unwrap()).collect();
    let r = &m.cells[0].rect;
    let round2 = |x: f64| coxeter_core::js::round(x * 100.0) / 100.0;
    assert_eq!((v[0], v[1]), (round2(r.x), round2(r.y - 10.0)));
    assert!(v[3] * v[2] <= r.w + 0.01, "never wider than the cell");
    assert!(30.0 * v[2] <= r.y - m.environment.rect.y, "never taller than the medium above the cell");
    assert!(Regex::new(r#"class="emc-cell-name"[^>]*textLength="#).unwrap().is_match(&svg));
}

// pathway-ribbons, reaction-coloring

fn stretch() -> (MapGraph, String) {
    let m = map("ecoli-stretch");
    let rid = m.segments.iter().find(|s| s.kind.as_str() != "currency").unwrap().reaction_id.clone();
    (m, rid)
}

#[test]
fn ribbons_render_behind_reaction_segments_and_preserve_flux_colour() {
    let (m, rid) = stretch();
    let scale = FluxScale::automatic([4.0], false);
    let color = scale.color(4.0);
    let svg = render_map_svg(
        &m,
        &RenderState {
            fluxes: fluxes(&[(&rid, 4.0)]),
            scale: Arc::new(scale),
            pathway_ribbon_mode: PathwayRibbonMode::Pathway,
            pathway_ribbon_colors: [(rid.clone(), "#7c3aed".to_string())].into(),
            pathway_ribbon_groups: [(rid.clone(), "Glycolysis".to_string())].into(),
            pathway_ribbon_width: 60.0,
            pathway_ribbon_opacity: 0.2,
            ..Default::default()
        },
    );
    let ribbon_at = svg.find("emc-layer-pathway-ribbons").unwrap();
    let segments_at = svg.find("emc-layer-segments").unwrap();
    assert!(segments_at > ribbon_at);
    assert!(svg.contains(r##"stroke="#7c3aed""##));
    assert!(svg.contains(&format!(r#"stroke="{color}""#)));
    assert!(svg.contains(r#"stroke-width="60""#));
}

#[test]
fn unannotated_reactions_do_not_receive_a_ribbon() {
    let (m, rid) = stretch();
    let svg = render_map_svg(
        &m,
        &RenderState {
            pathway_ribbon_mode: PathwayRibbonMode::Pathway,
            pathway_ribbon_colors: [(rid.clone(), "#2563eb".to_string())].into(),
            pathway_ribbon_groups: [(rid, "Sin pathway".to_string())].into(),
            ..Default::default()
        },
    );
    assert!(!svg.contains("data-ribbon-rxn="));
}

#[test]
fn reaction_pathway_metadata_is_read_from_explicit_and_common_cobra_metadata_locations() {
    let model = coxeter_core::Model::from_value(&json!({
        "id": "groups",
        "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "subsystem": "Glycolysis", "pathway": "Central carbon"},
            {"id": "R2", "metabolites": {"b_c": -1, "a_c": 1}, "notes": {"pathway": "Redox"}},
            {"id": "R3", "metabolites": {"a_c": -1, "b_c": 1}, "annotation": {"pathway": ["Amino acid metabolism"]}},
        ],
    }))
    .unwrap();
    assert_eq!(reaction_category(&model.reactions["R1"], ReactionColorMode::Subsystem).as_deref(), Some("Glycolysis"));
    assert_eq!(reaction_category(&model.reactions["R1"], ReactionColorMode::Pathway).as_deref(), Some("Central carbon"));
    assert_eq!(reaction_category(&model.reactions["R2"], ReactionColorMode::Pathway).as_deref(), Some("Redox"));
    assert_eq!(reaction_category(&model.reactions["R3"], ReactionColorMode::Pathway).as_deref(), Some("Amino acid metabolism"));
    assert_eq!(reaction_category(&model.reactions["R1"], ReactionColorMode::Flux), None);
}

#[test]
fn categorical_colours_are_deterministic_and_missing_metadata_has_an_explicit_group() {
    let model = coxeter_core::Model::from_value(&json!({
        "id": "groups",
        "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "subsystem": "TCA cycle"},
            {"id": "R2", "metabolites": {"b_c": -1, "a_c": 1}, "subsystem": "TCA cycle"},
            {"id": "R3", "metabolites": {"a_c": -1, "b_c": 1}},
        ],
    }))
    .unwrap();
    let first = build_reaction_coloring(Some(&model), ReactionColorMode::Subsystem, None, false);
    let second = build_reaction_coloring(Some(&model), ReactionColorMode::Subsystem, None, false);
    assert_eq!(first.colors["R1"], first.colors["R2"]);
    assert_eq!(first.colors["R1"], second.colors["R1"]);
    assert_eq!(first.groups["R3"], "Sin subsystem");
    assert_eq!(first.legend.iter().find(|l| l.label == "TCA cycle").map(|l| l.count), Some(2));
}

#[test]
fn map_renderer_can_colour_reactions_by_category_while_reaction_width_stays_fixed() {
    let m = map("ecoli-stretch");
    let rid = m.segments[0].reaction_id.clone();
    let svg = render_map_svg(
        &m,
        &RenderState {
            fluxes: fluxes(&[(&rid, 3.0)]),
            scale: Arc::new(FluxScale::automatic([3.0], false)),
            reaction_color_mode: ReactionColorMode::Subsystem,
            reaction_colors: [(rid.clone(), "#123456".to_string())].into(),
            ..Default::default()
        },
    );
    assert!(svg.contains(r##"stroke="#123456""##));
    assert!(svg.contains(r#"stroke-width="7.5""#));
}

#[test]
fn flux_mode_ignores_categorical_colours() {
    let m = map("ecoli-stretch");
    let rid = m.segments[0].reaction_id.clone();
    let scale = FluxScale::automatic([2.0], false);
    let color = scale.color(2.0);
    let svg = render_map_svg(
        &m,
        &RenderState {
            fluxes: fluxes(&[(&rid, 2.0)]),
            scale: Arc::new(scale),
            reaction_color_mode: ReactionColorMode::Flux,
            reaction_colors: [(rid, "#123456".to_string())].into(),
            ..Default::default()
        },
    );
    assert!(svg.contains(&format!(r#"stroke="{color}""#)));
}

// zuniga-render

fn zuniga_scale() -> TestScale {
    TestScale {
        reference: 1.0,
        width: |v| if v.abs() > 1.0 { 12.0 } else { 2.0 },
        color: |v| if v < 0.0 { "#2563eb".into() } else { "#ef4444".into() },
    }
}

fn zuniga_render(m: &MapGraph, reference: f64, f: impl FnOnce(&mut RenderState)) -> String {
    let mut s = RenderState { scale: Arc::new(TestScale { reference, ..zuniga_scale() }), ..Default::default() };
    f(&mut s);
    render_map_svg(m, &s)
}

fn zheads(svg: &str, rid: &str) -> Vec<String> {
    let mut v: Vec<String> = all(r#"<polygon class="emc-arrow"[^>]*data-rxn="([^"]+)"[^>]*data-node="([^"]+)""#, svg)
        .into_iter()
        .filter(|c| c[1] == rid)
        .map(|c| c[2].clone())
        .collect();
    v.sort();
    v
}

fn zpaths(svg: &str, rid: &str) -> Vec<String> {
    let needle = format!(r#"data-rxn="{rid}""#);
    all(r#"<path class="emc-seg[^>]+>"#, svg).into_iter().map(|c| c[0].clone()).filter(|t| t.contains(&needle)).collect()
}

#[test]
fn study_map_contains_both_cells_original_compartment_ids_and_the_complete_visible_phenylalanine_path() {
    let m = map("zuniga");
    assert_eq!(m.cells.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["Se", "Ec"]);
    assert!(m.cells[0].compartments.iter().any(|c| c.id == "u"));
    for species in ["Se", "Ec"] {
        for rid in ["DDPA", "DHQS", "DHQTi", "SHK3Dr", "SHKK", "PSCVT", "CHORS", "CHORM", "PPNDH", "PHETA1"] {
            assert!(m.reactions.contains_key(&format!("{species}@{rid}")));
        }
    }
    for rid in ["Se@PHELtex", "Ec@PHEtex", "Ec@PHEt2rpp"] {
        assert!(m.reactions.contains_key(rid));
    }
    assert_eq!(m.nodes.values().filter(|n| n.merged_id.as_deref() == Some("M@phe__L")).count(), 1);
    for base in ["phe__L_[smp]", "photon690_[smp]", "h_u_A"] {
        assert!(m.nodes.values().any(|n| n.base.as_deref() == Some(base)));
    }
    assert!(m.reactions.len() < 50);
    assert!(m.width() >= 1700.0 && m.height() >= 1000.0);
}

#[test]
fn flux_width_is_opt_in_and_all_branches_of_a_reaction_share_the_same_width() {
    let m = map("zuniga");
    let rid = "Ec@CHORM";
    let stroke_width = Regex::new(r#"stroke-width="([^"]+)""#).unwrap();
    for mode in [ReactionWidthMode::Constant, ReactionWidthMode::Flux] {
        let low = zuniga_render(&m, 1.0, |s| {
            s.fluxes = fluxes(&[(rid, 0.1)]);
            s.reaction_width_mode = mode;
        });
        let high = zuniga_render(&m, 1.0, |s| {
            s.fluxes = fluxes(&[(rid, 10.0)]);
            s.reaction_width_mode = mode;
        });
        let widths = |svg: &str| -> Vec<f64> {
            zpaths(svg, rid).iter().map(|t| stroke_width.captures(t).unwrap()[1].parse().unwrap()).collect()
        };
        let flux = mode == ReactionWidthMode::Flux;
        assert!(widths(&low).len() >= 3);
        assert!(widths(&low).iter().all(|w| *w == if flux { 2.0 } else { 7.5 }));
        assert!(widths(&high).iter().all(|w| *w == if flux { 12.0 } else { 7.5 }));
    }
}

#[test]
fn signed_difference_colours_and_widths_do_not_reverse_arrows_that_follow_positive_actual_flux() {
    let m = map("zuniga");
    let rid = "Ec@CHORM";
    let side_nodes = |side: &str| -> Vec<String> {
        let mut v: Vec<String> = m
            .segments
            .iter()
            .filter(|s| s.reaction_id == rid && s.side.as_deref() == Some(side))
            .filter_map(|s| s.node_id.clone())
            .collect();
        v.sort();
        v
    };
    let svg = zuniga_render(&m, 1.0, |s| {
        s.labels = LabelMode::All;
        s.fluxes = fluxes(&[(rid, -4.0)]);
        s.solved_fluxes = Some(fluxes(&[(rid, 2.0)]));
        s.reaction_width_mode = ReactionWidthMode::Flux;
    });
    assert_eq!(zheads(&svg, rid), side_nodes("product"));
    assert!(zpaths(&svg, rid).iter().all(|t| t.contains(r##"stroke="#2563eb""##) && t.contains(r#"stroke-width="12""#)));
    let negative_actual = zuniga_render(&m, 1.0, |s| {
        s.fluxes = fluxes(&[(rid, 4.0)]);
        s.solved_fluxes = Some(fluxes(&[(rid, -2.0)]));
    });
    assert_eq!(zheads(&negative_actual, rid), side_nodes("substrate"));
    let zero_actual = zuniga_render(&m, 1.0, |s| {
        s.fluxes = fluxes(&[(rid, 4.0)]);
        s.solved_fluxes = Some(fluxes(&[(rid, 0.0)]));
    });
    assert!(zheads(&zero_actual, rid).is_empty());
}

#[test]
fn knockout_is_visible_as_magenta_dashed_branches_and_a_cross_with_no_flux_arrow() {
    let m = map("zuniga");
    let rid = "Ec@PPNDH";
    let svg = zuniga_render(&m, 1.0, |s| {
        s.labels = LabelMode::Auto;
        s.fluxes = fluxes(&[(rid, 0.0)]);
        s.disabled_reaction_ids = set(&[rid]);
    });
    let paths = zpaths(&svg, rid);
    assert!(paths.len() >= 3);
    assert!(paths.iter().all(|t| t.contains("is-disabled-reaction") && t.contains(r##"stroke="#db2777""##) && t.contains(r#"stroke-dasharray="8 5""#)));
    assert!(svg.contains(r#"data-disabled-reaction="Ec@PPNDH""#));
    assert!(svg.contains(">PPNDH [KO]  0</text>"));
    assert!(zheads(&svg, rid).is_empty());
    assert!(!paths.join("").contains("is-active"));
}

#[test]
fn the_small_but_essential_phenylalanine_transfer_remains_labelled_in_auto_mode() {
    let m = map("zuniga");
    let rid = "Ec@PHEtex";
    let svg = zuniga_render(&m, 0.1, |s| {
        s.fluxes = fluxes(&[(rid, 0.014371756)]);
        s.labels = LabelMode::Auto;
        s.emphasized_reaction_ids = set(&[rid]);
    });
    assert!(svg.contains(">PHEtex  0.014</text>"));
    let zero = zuniga_render(&m, 0.1, |s| {
        s.labels = LabelMode::Auto;
        s.emphasized_reaction_ids = set(&[rid]);
    });
    assert!(!zero.contains(">PHEtex  "));
}

#[test]
fn monoculture_ghosts_keep_geometry_but_are_not_shown_as_simulated_growing_cells() {
    let m = map("zuniga");
    let before = m.clone();
    let rid = "Se@CHORM";
    let svg = zuniga_render(&m, 1.0, |s| {
        s.labels = LabelMode::All;
        s.fluxes = fluxes(&[(rid, 100.0)]);
        s.growth = fluxes(&[("Se", 0.1234), ("Ec", 0.2)]);
        s.inactive_species_ids = set(&["Se"]);
    });
    assert!(svg.contains(r#"<g class="emc-cell is-inactive-species" data-cell="Se" opacity="0.32">"#));
    assert!(Regex::new(r"S\. elongatus cscB\+ · inactive context").unwrap().is_match(&svg));
    assert!(!Regex::new(r"S\. elongatus cscB\+[^<]*µ").unwrap().is_match(&svg));
    assert!(Regex::new(r"E\. coli K-12[^<]*µ = 0\.2").unwrap().is_match(&svg));
    assert!(zpaths(&svg, rid).iter().all(|t| t.contains("is-inactive-species") && t.contains(r#"opacity="0.22""#)));
    assert!(zheads(&svg, rid).is_empty());
    assert!(Regex::new(r#"data-metabolite-label="met:Se@chor_c"[^>]*opacity="0.28""#).unwrap().is_match(&svg));
    assert_eq!(m, before, "renderer never edits shared geometry");
}

/// Left, top, right, bottom.
type Box4 = (f64, f64, f64, f64);

/// Where a metabolite's name is drawn, as a box. The width is an estimate, 0.56 em
/// a character, which the label fonts stay within; the sizes are map.css's.
fn name_box(node: &Node) -> Box4 {
    use coxeter_core::metabolite_label::{Anchor, Baseline, is_secondary_node, metabolite_label_position};
    let size = if node.kind == NodeKind::Medium { 15.0 } else if is_secondary_node(node) { 9.5 } else { 12.5 };
    let w = node.base.as_deref().unwrap_or("").chars().count() as f64 * size * 0.56;
    let p = metabolite_label_position(node);
    let left = match p.anchor {
        Anchor::Start => p.x,
        Anchor::End => p.x - w,
        Anchor::Middle => p.x - w / 2.0,
    };
    let top = match p.baseline {
        Baseline::Auto => p.y - size * 0.8,
        Baseline::Hanging => p.y,
        Baseline::Middle => p.y - size / 2.0,
    };
    (left, top, left + w, top + size)
}

fn boxes_overlap(a: Box4, b: Box4) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

fn on_circle(b: Box4, node: &Node) -> bool {
    let (cx, cy) = (node.x.clamp(b.0, b.2), node.y.clamp(b.1, b.3));
    (cx - node.x).hypot(cy - node.y) < node.r
}

#[test]
fn no_metabolite_name_is_written_over_another_name_or_another_metabolite() {
    let m = map("zuniga");
    let nodes: Vec<&Node> = m.nodes.values().collect();
    let mut clashes = Vec::new();
    for (i, a) in nodes.iter().enumerate() {
        for b in &nodes[i + 1..] {
            if boxes_overlap(name_box(a), name_box(b)) {
                clashes.push(format!("{} / {}", a.id, b.id));
            }
        }
        for b in &nodes {
            if a.id != b.id && on_circle(name_box(a), b) {
                clashes.push(format!("{} on {}", a.id, b.id));
            }
        }
    }
    assert_eq!(clashes, Vec::<String>::new());
}

#[test]
fn every_compartment_name_plate_reaches_full_size_and_at_it_covers_no_metabolite_name() {
    let m = map("zuniga");
    let svg = zuniga_render(&m, 1.0, |s| s.labels = LabelMode::All);
    let plates: Vec<(String, f64, Box4)> = all(
        r#"data-compartment-label="([^"]+)" transform="translate\(([^ ]+) ([^)]+)\)">\s*<g class="emc-frame-label" style="--fit:([^"]+)">\s*<rect x="([^"]+)" y="([^"]+)" width="([^"]+)" height="([^"]+)""#,
        &svg,
    )
    .into_iter()
    .map(|c| {
        let v: Vec<f64> = c[2..].iter().map(|x| x.parse().unwrap()).collect();
        let (tx, ty, fit, x, y, w, h) = (v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
        let s = fit.min(1.0);
        (c[1].clone(), fit, (tx + x * s, ty + y * s, tx + (x + w) * s, ty + (y + h) * s))
    })
    .collect();
    assert_eq!(plates.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["s", "p", "c", "k", "u", "p", "c"]);
    // k and u are nested closely in S. elongatus: k's plate used to shrink to a third.
    for (id, fit, _) in &plates {
        assert!(*fit >= 1.0, "{id} plate fits at {fit}");
    }
    let covered: Vec<String> = plates
        .iter()
        .flat_map(|(id, _, b)| m.nodes.values().filter(|n| boxes_overlap(*b, name_box(n))).map(move |n| format!("{id}: {}", n.id)))
        .collect();
    assert_eq!(covered, Vec::<String>::new());
}

// ui-audit-fixes (rendering)

fn audit_state(extra: impl FnOnce(&mut RenderState)) -> RenderState {
    let mut s = common::state_from_spec(&case_state("audit-labels-auto"));
    s.labels = LabelMode::Auto;
    extra(&mut s);
    s
}

#[test]
fn labels_none_hides_metabolite_labels_as_well_as_reaction_labels() {
    let m = map("audit");
    let draw = |labels| render_map_svg(&m, &audit_state(|s| s.labels = labels));
    assert!(draw(LabelMode::Auto).contains("emc-label-met"));
    assert!(draw(LabelMode::Auto).contains("emc-label-medium"));
    let none = draw(LabelMode::None);
    assert!(!Regex::new("emc-label-met|emc-label-medium|emc-label-currency|emc-label-rxn").unwrap().is_match(&none));
}

#[test]
fn an_exported_map_carries_no_selection_handles_pointer_targets_or_resize_grips() {
    let m = map("audit");
    let editor_state = audit_state(|s| {
        s.selected_reaction = Some("demo@At".into());
        s.reaction_edit = true;
        s.selected_geometry = Some(GeometrySelection { cell_id: Some("demo".into()), compartment_id: "c".into() });
        s.multi_selection = Some(MultiSelection { nodes: set(&["met:demo@a_c", "met:demo@a_e"]), ..Default::default() });
        s.min_hit_radius = 12.0;
    });
    let editor = render_map_svg(&m, &editor_state);
    let markers = ["emc-hit", "data-canvas-resize", "emc-multi-area", "emc-multi-drag", "is-selected", "is-multi-selected", "emc-geometry-selection"];
    for marker in markers {
        assert!(editor.contains(marker), "the editor view should contain {marker}");
    }
    let exported = render_map_svg(&m, &RenderState { for_export: true, ..editor_state });
    for marker in markers.iter().chain(&["emc-layer-edit-handles"]) {
        assert!(!exported.contains(marker), "the export must not contain {marker}");
    }
    assert!(exported.contains("emc-node"), "the map itself is still drawn");
    assert!(exported.contains("emc-canvas-frame"), "and so is its white canvas");
}

#[test]
fn cell_and_compartment_colours_are_drawn() {
    let m = map("audit-colours");
    let cell = m.cells.iter().find(|c| c.id == "demo").unwrap();
    let body = cell.outer_compartment.clone().unwrap();
    let inner = cell.compartments.iter().find(|c| c.id != body).unwrap().id.clone();
    let svg = render_map_svg(&m, &audit_state(|_| {}));
    assert!(Regex::new(&format!(r##"data-compartment-space="{body}"[^>]*fill="#12ab34""##)).unwrap().is_match(&svg));
    assert!(Regex::new(r##"class="emc-cell-boundary"[^>]*stroke="#aa0000""##).unwrap().is_match(&svg));
    assert!(Regex::new(&format!(r##"data-compartment-space="{inner}"[^>]*fill="#0000ff""##)).unwrap().is_match(&svg));
    assert!(svg.contains(r##"stroke="#00ff00""##));
    let plain = render_map_svg(&map("audit"), &audit_state(|_| {}));
    assert!(!Regex::new("#12ab34|#aa0000|#0000ff|#00ff00").unwrap().is_match(&plain), "no custom colour, the theme");
}

#[test]
fn the_cofactors_toggles_hide_every_secondary_node_its_lines_and_its_label() {
    let m = map("audit-secondary");
    let draw = |show: bool, names: bool| {
        render_map_svg(&m, &audit_state(|s| {
            s.show_currency = show;
            s.currency_labels = names;
        }))
    };
    let shown = draw(true, true);
    assert!(Regex::new(r#"class="emc-node[^"]*"[^>]*data-node="met:demo@a_c""#).unwrap().is_match(&shown));
    assert!(shown.contains(r#"data-metabolite-label="met:demo@a_c""#));
    let hidden = draw(false, true);
    assert!(!hidden.contains(r#"data-node="met:demo@a_c""#), "node, hit target and lines go");
    assert!(!hidden.contains(r#"data-metabolite-label="met:demo@a_c""#));
    assert!(hidden.contains(r#"data-node="met:demo@a_e""#), "primary metabolites stay");
    let unnamed = draw(true, false);
    assert!(Regex::new(r#"class="emc-node[^"]*"[^>]*data-node="met:demo@a_c""#).unwrap().is_match(&unnamed));
    assert!(!unnamed.contains(r#"data-metabolite-label="met:demo@a_c""#));
    assert!(unnamed.contains(r#"data-metabolite-label="met:demo@a_e""#));
}

#[test]
fn all_active_names_only_metabolites_drawn_on_a_reaction_that_carries_flux() {
    let m = map("audit");
    let solved = common::state_from_spec(&case_state("audit-active-solved")).solved_fluxes.unwrap();
    let draw = |labels: LabelMode, solved: HashMap<String, f64>, fva: bool| {
        render_map_svg(&m, &audit_state(|s| {
            s.labels = labels;
            s.solved_fluxes = Some(solved);
            s.fva_enabled = fva;
        }))
    };
    let met_labels = |svg: &str| svg.matches("data-metabolite-label=\"").count();
    let active = draw(LabelMode::Active, solved.clone(), false);
    assert!(active.contains(r#"data-metabolite-label="met:demo@a_c""#), "a_c is on the active transporter");
    assert_eq!(met_labels(&draw(LabelMode::Active, HashMap::new(), false)), 0, "nothing carries flux, so nothing is named");
    assert_eq!(met_labels(&draw(LabelMode::All, HashMap::new(), false)), met_labels(&draw(LabelMode::All, solved, false)));
    assert!(met_labels(&draw(LabelMode::Active, HashMap::new(), true)) > 0, "FVA mode keeps every name");
}

#[test]
fn a_switched_off_species_is_drawn_dimmed_and_labelled() {
    let m = map("pair");
    let mut s = common::state_from_spec(&case_state("pair-inactive"));
    s.inactive_species_ids = set(&["other"]);
    s.inactive_species_label = "switched off".into();
    assert!(render_map_svg(&m, &s).contains("· switched off"));
}

#[test]
fn an_exported_map_does_not_keep_the_cross_feeding_event_picked_in_the_explorer() {
    let _turn = ROUTING.lock().unwrap_or_else(|e| e.into_inner());
    let m = map("pair");
    let link = CrossFeedingLink { base: "a".into(), from: Some("demo".into()), to: Some("other".into()), flux: 1.0 };
    let event = CrossFeedingEvent { from: "demo".into(), to: "other".into(), base: "a".into(), flux: 1.0, ..Default::default() };
    let mut state = common::state_from_spec(&case_state("pair-crossfeed-picked"));
    state.show_cross_feeding = true;
    state.cross_feeding = Some(coxeter_core::community::CrossFeeding { links: vec![link], per_metabolite: vec![] });
    state.cross_feeding_explorer = Some(CrossFeedingExplorerView {
        enabled: true,
        visible_events: vec![event.clone()],
        selected_event: Some(event),
        selected_at: None,
    });
    // The JS test's pattern, `class="emc-crossfeed(?:-route)?"`, misses the
    // picked line (`class="emc-crossfeed-route is-picked"`) and fails in JS too;
    // the extra classes are allowed here.
    let opacity = |svg: &str| -> f64 {
        Regex::new(r#"class="emc-crossfeed(?:-route)?(?: [^"]*)?"[^>]*opacity="([\d.]+)""#).unwrap().captures(svg).unwrap()[1]
            .parse()
            .unwrap()
    };
    let on_screen = opacity(&render_map_svg(&m, &state));
    let in_figure = opacity(&render_map_svg(&m, &RenderState { for_export: true, ..state }));
    assert!(on_screen > in_figure, "on screen the picked event stands out; in the figure it is one event among the others");
}
