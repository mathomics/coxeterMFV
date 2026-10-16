//! The render assertions of `test/compartment-shape.test.mjs` ("a selected
//! compartment shows an orange corner diamond and a yellow oval diamond") and
//! `test/geometry-edit.test.mjs` ("a selected compartment has a handle in the
//! middle of each side as well as at each corner"), on the yeast map of the
//! render fixtures with its mitochondrion reshaped by hand.

mod common;

use std::sync::Arc;

use coxeter_core::geometry::{compartment_corners, shape_handle_point};
use coxeter_core::mapdata::MapGraph;
use coxeter_render::{GeometrySelection, RenderState, Scale, geometry_handle_side, render_map_svg};
use regex::Regex;

struct Flat;

impl Scale for Flat {
    fn reference(&self) -> f64 {
        1.0
    }
    fn width(&self, _: f64) -> f64 {
        2.0
    }
    fn color(&self, _: f64) -> String {
        "#000".into()
    }
}

fn yeast() -> MapGraph {
    common::load_map(&common::dir(false).join("maps/yeast-transport.json"))
}

fn selected(cell: Option<&str>, compartment: &str) -> RenderState {
    RenderState {
        scale: Arc::new(Flat),
        geometry_edit: true,
        selected_geometry: Some(GeometrySelection { cell_id: cell.map(str::to_string), compartment_id: compartment.into() }),
        ..Default::default()
    }
}

#[test]
fn a_selected_compartment_has_a_handle_in_the_middle_of_each_side_as_well_as_at_each_corner() {
    let m = yeast();
    let cell = m.cells[0].id.clone();
    let comp = m.cells[0].compartments[0].clone();
    let svg = render_map_svg(&m, &selected(Some(&cell), &comp.id));
    let re = Regex::new(r#"<rect class="emc-geometry-handle[^>]*data-geometry-handle="(\w+)" x="([\d.-]+)" y="([\d.-]+)"\s+width="([\d.-]+)" height="([\d.-]+)""#).unwrap();
    let handles: Vec<(String, f64, f64)> = re
        .captures_iter(&svg)
        .map(|c| {
            let num = |i: usize| c[i].parse::<f64>().unwrap();
            (c[1].to_string(), num(2) + num(4) / 2.0, num(3) + num(5) / 2.0)
        })
        .collect();
    let mut names: Vec<&str> = handles.iter().map(|h| h.0.as_str()).collect();
    names.sort();
    assert_eq!(names, ["e", "n", "ne", "nw", "s", "se", "sw", "w"]);
    assert!(svg.contains(r#"data-geometry-handle="move""#));
    let r = comp.rect;
    let at = |name: &str| handles.iter().find(|h| h.0 == name).map(|h| (h.1, h.2)).unwrap();
    let near = |a: f64, b: f64| (a - b).abs() < 0.1;
    assert!(near(at("n").0, r.x + r.w / 2.0) && near(at("n").1, r.y), "n sits mid-top");
    assert!(near(at("s").0, r.x + r.w / 2.0) && near(at("s").1, r.y + r.h), "s sits mid-bottom");
    assert!(near(at("w").0, r.x) && near(at("w").1, r.y + r.h / 2.0), "w sits mid-left");
    assert!(near(at("e").0, r.x + r.w) && near(at("e").1, r.y + r.h / 2.0), "e sits mid-right");
}

#[test]
fn a_selected_compartment_shows_an_orange_corner_diamond_and_a_yellow_oval_diamond() {
    let mut m = yeast();
    let cell = m.cells[0].id.clone();
    let m_comp = m.cells[0].compartments.iter_mut().find(|c| c.id == "m").unwrap();
    m_comp.shape = Some(serde_json::json!({ "corner": 0.4, "oval": 0.6 }));
    let comp = m_comp.clone();
    let svg = render_map_svg(&m, &selected(Some(&cell), "m"));
    let side = geometry_handle_side(0.0);
    for (which, colour, amount) in [("corner", "#f97316", 0.4), ("oval", "#ffc512", 0.6)] {
        let tag = Regex::new(&format!(r#"<path class="emc-geometry-handle emc-geometry-shape emc-geometry-{which}"[^>]*>"#))
            .unwrap()
            .find(&svg)
            .unwrap_or_else(|| panic!("{which} diamond"))
            .as_str()
            .to_string();
        assert!(tag.contains(&format!(r#"fill="{colour}""#)));
        let p = shape_handle_point(&comp.rect, which, amount, side);
        let c = Regex::new(r#"d="M([\d.-]+) ([\d.-]+)"#).unwrap().captures(&tag).unwrap();
        let (x, y): (f64, f64) = (c[1].parse().unwrap(), c[2].parse().unwrap());
        assert!((x - p.x).abs() < 0.1 && (y - (p.y - side * 0.62)).abs() < 0.1, "{which} placed by its amount");
    }
    // Over the name plates: the mitochondrion's sits astride the top edge, on the corner diamond's track.
    assert!(
        svg.find("emc-layer-geometry-editor").unwrap() > svg.find("emc-layer-compartment-labels").unwrap(),
        "handles are drawn after the compartment names"
    );
    // And the outline is drawn with the elliptical corners.
    let corners = compartment_corners(&comp);
    assert!(corners.rx > corners.ry);
    let rx = coxeter_render::format::n(corners.rx);
    let ry = coxeter_render::format::n(corners.ry);
    assert!(Regex::new(&format!(r#"data-compartment-space="m" [^>]*rx="{rx}" ry="{ry}""#)).unwrap().is_match(&svg));

    // The shared medium is not reshaped, so it gets no diamonds.
    let env = m.environment.id.clone();
    let medium = render_map_svg(&m, &selected(None, &env));
    assert!(!medium.contains("emc-geometry-shape"));
}
