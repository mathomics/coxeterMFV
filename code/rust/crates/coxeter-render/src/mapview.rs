//! SVG renderer (`src/render/mapview.js`).
//!
//! Produces the complete SVG of a map: as an element tree ([`render_map`]),
//! and as the very string the JavaScript `renderMap` returns
//! ([`render_map_svg`]), which the web app swaps into the page and re-binds
//! event handlers to through delegation on stable `data-` attributes.
//!
//! Drawing order matters and mirrors the containment hierarchy: medium, then
//! cells outside-in, then flux, then nodes, then text on top of everything.

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

use coxeter_core::boundaries::{boundary_key, environment_key};
use coxeter_core::community::{CrossFeedingLink, LINK_COMPARTMENT};
use coxeter_core::geometry::{
    Corners, SIZES, closest_perimeter_point, closest_perimeter_point_corners, compartment_corners, compartment_shape,
    is_round_compartment, shape_handle_point,
};
use coxeter_core::js;
use coxeter_core::mapdata::{MapCell, MapCompartment, MapGraph, MapReaction, MapText, Node, NodeKind, Point, Rect, Segment, SegmentKind};
use coxeter_core::metabolite_label::{is_secondary_node, metabolite_label_position};
use coxeter_core::text_style::{DEFAULT_TEXT_FONT, text_box_of, text_font_stack};
use serde_json::Value;

use crate::crossfeed_routes::{is_drawn_cross_feeding_base, route_anchor, route_cross_feeding, route_grid, rounded_path, Endpoint};
use crate::format::{esc, js_trim, n, utf16_len};
use crate::reaction_colors::{PathwayRibbonMode, ReactionColorMode, is_missing_group};
use crate::line_hops::{HopItem, MAX_LINE_HOPS, find_line_hops, hopped_path};
use crate::reaction_path::{EndpointGeometry, endpoint_node_radius, reaction_endpoint_geometry};
use crate::state::{CrossFeedingEvent, GeometrySelection, LabelMode, MultiSelection, ReactionWidthMode, RenderState};
use crate::svg::Element;
use crate::theme::{CompartmentStyle, Scale, compartment_style, format_flux};

/// Write the map as the string the JavaScript `renderMap` returns.
pub fn render_map_svg(map: &MapGraph, state: &RenderState) -> String {
    render_map(map, state).to_markup()
}

/// The explorer as the renderer sees it (the export drops its pick).
struct Explorer<'a> {
    enabled: bool,
    visible_events: &'a [CrossFeedingEvent],
    selected: Option<&'a CrossFeedingEvent>,
    selected_at: Option<Point>,
}

/// The state with the export's overrides applied, and the map.
struct Ctx<'a> {
    map: &'a MapGraph,
    s: &'a RenderState,
    scale: &'a dyn Scale,
    dark: bool,
    selected_reaction: Option<&'a str>,
    multi: Option<&'a MultiSelection>,
    min_hit_radius: f64,
    explorer: Option<Explorer<'a>>,
    omics_fallback: String,
}

impl<'a> Ctx<'a> {
    fn flux(&self, rid: &str) -> f64 {
        self.s.fluxes.get(rid).copied().unwrap_or(0.0)
    }
    fn solved(&self, rid: &str) -> f64 {
        let solved = self.s.solved_fluxes.as_ref().unwrap_or(&self.s.fluxes);
        solved.get(rid).copied().unwrap_or(0.0)
    }
    fn disabled(&self, rid: &str) -> bool {
        self.s.disabled_reaction_ids.contains(rid)
    }
    fn inactive(&self, cell_id: Option<&str>) -> bool {
        cell_id.is_some_and(|id| self.s.inactive_species_ids.contains(id))
    }
    fn multi_has_reaction(&self, rid: &str) -> bool {
        self.multi.is_some_and(|m| m.reactions.contains(rid))
    }
    fn multi_has_node(&self, id: &str) -> bool {
        self.multi.is_some_and(|m| m.nodes.contains(id))
    }
    fn multi_has_text(&self, id: &str) -> bool {
        self.multi.is_some_and(|m| m.texts.contains(id))
    }
    fn node(&self, id: Option<&str>) -> Option<&'a Node> {
        id.filter(|id| !id.is_empty()).and_then(|id| self.map.nodes.get(id))
    }

    /// `reactionDisplayColor`: a reaction's colour. An omics overlay wins;
    /// then flux colouring, when lines are coloured by flux; otherwise the
    /// colour set on the reaction itself, then its pathway's or subsystem's.
    fn reaction_display_color(&self, rid: &str, flux: f64, own: Option<&str>) -> String {
        if self.s.omics_enabled {
            return self.s.omics_colors.get(rid).cloned().unwrap_or_else(|| self.omics_fallback.clone());
        }
        match self.s.reaction_color_mode {
            ReactionColorMode::Flux => self.scale.color(flux),
            _ => match own {
                Some(c) => c.to_string(),
                None => self.s.reaction_colors.get(rid).cloned().unwrap_or_else(|| self.scale.color(flux)),
            },
        }
    }

    /// `isOffReaction`: a solution exists and the reaction carries nothing
    /// in it.
    fn is_off(&self, rid: &str, inactive: bool) -> bool {
        if !self.s.off_style || inactive || self.disabled(rid) {
            return false;
        }
        let solved = self.s.solved_fluxes.as_ref().unwrap_or(&self.s.fluxes);
        solved.get(rid).copied().unwrap_or(0.0).abs() <= 1e-9
    }

    /// The hover text of a reaction, when the state carries tooltips.
    fn reaction_tooltip(&self, rid: &str) -> Option<String> {
        self.s.tooltips.as_ref().map(|t| t.reactions.get(rid).cloned().unwrap_or_else(|| rid.to_string()))
    }

    /// The hover text of a node, when the state carries tooltips.
    fn node_tooltip(&self, id: &str) -> Option<String> {
        self.s.tooltips.as_ref().map(|t| t.nodes.get(id).cloned().unwrap_or_else(|| id.to_string()))
    }
}

fn str_opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("")
}

fn truthy(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.is_empty())
}

/// `renderMap(layout, state)` as an element tree.
pub fn render_map(map: &MapGraph, state: &RenderState) -> Element {
    // An exported figure is the map, not the editor around it.
    let export = state.for_export;
    let explorer = state.cross_feeding_explorer.as_ref().map(|e| Explorer {
        enabled: e.enabled,
        visible_events: &e.visible_events,
        // The event picked in the cross-feeding explorer is a selection too.
        selected: if export { None } else { e.selected_event.as_ref() },
        selected_at: e.selected_at,
    });
    let dark = state.dark;
    let cx = Ctx {
        map,
        s: state,
        scale: state.scale.as_ref(),
        dark,
        selected_reaction: if export { None } else { state.selected_reaction.as_deref() },
        multi: if export { None } else { state.multi_selection.as_ref() },
        min_hit_radius: if export { 0.0 } else { state.min_hit_radius },
        explorer,
        omics_fallback: state
            .omics_fallback_color
            .clone()
            .unwrap_or_else(|| if dark { "#475569".into() } else { "#cbd5e1".into() }),
    };
    let selected_geometry = if export { None } else { state.selected_geometry.as_ref() };
    let geometry_edit = !export && state.geometry_edit;
    let reaction_edit = !export && state.reaction_edit;

    let mut svg = Element::paired("svg")
        .with("xmlns", "http://www.w3.org/2000/svg")
        .with("class", if dark { "emc-map emc-dark" } else { "emc-map" })
        .with("viewBox", format!("0 0 {} {}", n(map.width()), n(map.height())))
        .with("width", n(map.width()))
        .with("height", n(map.height()))
        .with("role", "img")
        .with("aria-label", "Multicell community metabolic map");
    svg.push(defs(dark));
    svg.push(render_canvas_frame(map, dark));
    if let Some(env) = render_environment(map, dark, cx.min_hit_radius) {
        svg.push(env);
    }
    // Routed lines run through the medium, so they go over the cells' fills and
    // under the exchanges they end at. A link that cannot be routed is drawn as
    // an arc, before the cells so it passes behind them instead of cutting
    // across the pathways it connects.
    let cross_feeding_layers = match (&state.cross_feeding, state.show_cross_feeding) {
        (Some(cf), true) => Some(render_cross_feeding(&cx, &cf.links)),
        _ => None,
    };
    let (behind, routed, tag) = match cross_feeding_layers {
        Some((b, r, t)) => (Some(b), Some(r), t),
        None => (None, None, None),
    };
    if let Some(b) = behind {
        svg.push(b);
    }
    svg.push(render_cells(&cx));
    if let Some(r) = routed {
        svg.push(r);
    }
    if let Some(r) = render_pathway_ribbons(&cx) {
        svg.push(r);
    }
    let cross_feeding_reactions = active_cross_feeding_reaction_ids(&cx);
    svg.push(render_segments(&cx, &cross_feeding_reactions));
    // The multi-selection area goes under the marks, so it reads as the area
    // they sit on rather than a box drawn over them.
    let selection_bounds = multi_selection_bounds(map, cx.multi);
    if let Some(b) = &selection_bounds {
        svg.push(render_multi_selection_area(b, &selected_compartment_rects(map, cx.multi)));
    }
    // Invisible pointer targets keep a glyph clickable at any zoom without
    // changing how it is drawn. They go between the segments and the markers,
    // which is where the workspace tests expect them in the hit-test order.
    if !export {
        svg.push(render_hit_targets(&cx));
    }
    svg.push(render_markers(&cx));
    svg.push(render_nodes(&cx));
    if reaction_edit && !map.reactions.is_empty() {
        svg.push(render_edit_handles(&cx));
    }
    svg.push(render_text(&cx));
    if let Some(t) = render_map_annotations(&cx) {
        svg.push(t);
    }
    if let Some(t) = tag {
        svg.push(t);
    }
    if !export {
        svg.push(render_canvas_resize_handles(map, dark));
    }
    svg.push(render_compartment_labels(&cx));
    // The handles (corners, sides, the centre to move by and the two shape
    // diamonds) belong to any compartment that is selected, not only to the
    // geometry-editing tools. Select sets the same geometry selection, so
    // choosing a compartment there shows them and they can be dragged straight
    // away. They go over the name plates, since a plate astride the top edge of
    // a round compartment would cover the handles running along it.
    let handles = if geometry_edit || selected_geometry.is_some() { selected_geometry } else { None };
    if let Some(h) = handles.and_then(|sel| render_geometry_handles(map, sel, dark, cx.min_hit_radius)) {
        svg.push(h);
    }
    if let Some(b) = &selection_bounds {
        svg.push(render_multi_selection_border(b, cx.min_hit_radius));
    }
    svg
}

// Geometry editor

/// `geometryHandleSide(minHitRadius)`: the side of a corner handle, in map
/// units; the shape handles are spaced by it.
pub fn geometry_handle_side(min_hit_radius: f64) -> f64 {
    js::max(12.0, or_zero(min_hit_radius) * 2.0)
}

/// Orange and sunflower yellow, the same in both themes, so the inspector can
/// name each by its colour (`SHAPE_HANDLE_COLOURS`).
pub const SHAPE_HANDLE_CORNER_COLOUR: &str = "#f97316";
/// The sunflower yellow of the oval handle (see [`SHAPE_HANDLE_CORNER_COLOUR`]).
pub const SHAPE_HANDLE_OVAL_COLOUR: &str = "#ffc512";

fn render_geometry_handles(map: &MapGraph, selection: &GeometrySelection, dark: bool, min_hit_radius: f64) -> Option<Element> {
    let is_environment = selection.cell_id.is_none() && selection.compartment_id == map.environment.id;
    let (cell, comp, id, r) = if is_environment {
        (None, None, map.environment.id.as_str(), map.environment.rect)
    } else {
        let cell = map.cells.iter().find(|c| Some(c.id.as_str()) == selection.cell_id.as_deref())?;
        let comp = cell.compartments.iter().find(|c| c.id == selection.compartment_id)?;
        (Some(cell), Some(comp), comp.id.as_str(), comp.rect)
    };
    let stroke = if dark { "#f8fafc" } else { "#0f172a" };
    let fill = if dark { "#111827" } else { "#ffffff" };
    let cell_attr = cell.map(|c| c.id.clone()).unwrap_or_default();
    let corners = [("nw", r.x, r.y), ("ne", r.x + r.w, r.y), ("se", r.x + r.w, r.y + r.h), ("sw", r.x, r.y + r.h)];
    // The middle of each side moves that side alone: width or height only.
    let sides = [("n", r.x + r.w / 2.0, r.y), ("e", r.x + r.w, r.y + r.h / 2.0), ("s", r.x + r.w / 2.0, r.y + r.h), ("w", r.x, r.y + r.h / 2.0)];

    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-geometry-editor");
    out.push(
        Element::new("rect")
            .with("class", "emc-geometry-selection")
            .with("x", n(r.x))
            .with("y", n(r.y))
            .with("width", n(r.w))
            .with("height", n(r.h))
            .with_ws("\n      ", "fill", "none")
            .with("stroke", stroke)
            .with("stroke-width", "2")
            .with("stroke-dasharray", "7 5")
            .with("pointer-events", "none"),
    );

    // Each handle says what it does, on the handle. Moving a compartment takes
    // two steps (select it, then drag its middle), and a tooltip where the
    // pointer already is beats explaining it anywhere else.
    // Sized in screen terms, like the reaction halos: a 12-unit square is a
    // pixel and a half once a large map is fitted to the stage, which is neither
    // visible nor clickable.
    // minHitRadius is MIN_HIT_PX in map units, and MIN_HIT_PX is a radius: twice
    // it is the 24-pixel target the halos already use, so a handle ends up the
    // same size on screen as everything else that can be grabbed.
    let side = geometry_handle_side(min_hit_radius);
    let half = side / 2.0;
    let move_radius = js::max(8.0, or_zero(min_hit_radius));

    // A bar lying along its side, so it reads as "this side" and not as one
    // more corner. Its short dimension is the corner's, which keeps the hit
    // target. Drawn before the corners, so a corner stays on top where they
    // meet.
    let long = side * 1.75;
    for (handle, x, y) in sides {
        let across = handle == "n" || handle == "s";
        let w = if across { long } else { side };
        let h = if across { side } else { long };
        out.push(
            Element::paired("rect")
                .with("class", format!("emc-geometry-handle emc-geometry-{handle}"))
                .with("data-geometry-cell", cell_attr.clone())
                .with("data-geometry-comp", id.to_string())
                .with_ws("\n      ", "data-geometry-handle", handle)
                .with("x", n(x - w / 2.0))
                .with("y", n(y - h / 2.0))
                .with_ws("\n      ", "width", n(w))
                .with("height", n(h))
                .with_ws("\n      ", "rx", n(side / 2.0))
                .with("fill", fill)
                .with("stroke", stroke)
                .with("stroke-width", "2")
                .closing("\n      ")
                .child(Element::paired("title").text(format!(
                    "Drag to change this compartment's {}",
                    if across { "height" } else { "width" }
                ))),
        );
    }

    for (handle, x, y) in corners {
        out.push(
            Element::paired("rect")
                .with("class", format!("emc-geometry-handle emc-geometry-{handle}"))
                .with("data-geometry-cell", cell_attr.clone())
                .with("data-geometry-comp", id.to_string())
                .with_ws("\n      ", "data-geometry-handle", handle)
                .with("x", n(x - half))
                .with("y", n(y - half))
                .with_ws("\n      ", "width", n(side))
                .with("height", n(side))
                .with_ws("\n      ", "rx", "2")
                .with("fill", fill)
                .with("stroke", stroke)
                .with("stroke-width", "2")
                .closing("\n      ")
                .child(Element::paired("title").text("Drag to resize this compartment")),
        );
    }
    out.push(
        Element::paired("circle")
            .with("class", "emc-geometry-handle emc-geometry-move")
            .with("data-geometry-cell", cell_attr.clone())
            .with("data-geometry-comp", id.to_string())
            .with_ws("\n    ", "data-geometry-handle", "move")
            .with("cx", n(r.x + r.w / 2.0))
            .with("cy", n(r.y + r.h / 2.0))
            .with("r", n(move_radius))
            .with_ws("\n    ", "fill", fill)
            .with("stroke", stroke)
            .with("stroke-width", "2")
            .closing("\n    ")
            .child(Element::paired("title").text("Drag to move this compartment and everything in it")),
    );

    // The shape diamonds, a cell's compartments only: the orange one along the
    // top softens the corners (and, on an oval, makes it rounder or boxier),
    // the yellow one down the left side rounds the outline into an oval. Where
    // each stands on its edge is how far it goes.
    if let (false, Some(comp)) = (is_environment, comp) {
        let shape = compartment_shape(comp);
        let reach = side * 0.62;
        for (handle, amount, colour, edge, title) in [
            (
                "corner",
                shape.corner,
                SHAPE_HANDLE_CORNER_COLOUR,
                if dark { "#fed7aa" } else { "#9a3412" },
                "Drag along the edge to soften the corners (on an oval: rounder or boxier)",
            ),
            (
                "oval",
                shape.oval,
                SHAPE_HANDLE_OVAL_COLOUR,
                if dark { "#fef08a" } else { "#854d0e" },
                "Drag along the edge to make this compartment oval",
            ),
        ] {
            let p = shape_handle_point(&r, handle, amount, side);
            out.push(
                Element::paired("path")
                    .with("class", format!("emc-geometry-handle emc-geometry-shape emc-geometry-{handle}"))
                    .with("data-geometry-cell", cell_attr.clone())
                    .with("data-geometry-comp", id.to_string())
                    .with_ws("\n        ", "data-geometry-handle", handle)
                    .with("data-shape-amount", n(amount))
                    .with_ws(
                        "\n        ",
                        "d",
                        format!(
                            "M{} {}L{} {}L{} {}L{} {}Z",
                            n(p.x),
                            n(p.y - reach),
                            n(p.x + reach),
                            n(p.y),
                            n(p.x),
                            n(p.y + reach),
                            n(p.x - reach),
                            n(p.y)
                        ),
                    )
                    .with_ws("\n        ", "fill", colour)
                    .with("stroke", edge)
                    .with("stroke-width", "2")
                    .with("stroke-linejoin", "round")
                    .closing("\n        ")
                    .child(Element::paired("title").text(title)),
            );
        }
    }
    Some(out)
}

/// `value || 0` for a number.
fn or_zero(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v }
}

// Defs

fn defs(dark: bool) -> Element {
    let wall = if dark { "#78716c" } else { "#a8a29e" };
    Element::paired("defs")
        .text("\n    ")
        .child(
            Element::paired("pattern")
                .with("id", "emc-wall")
                .with("width", "9")
                .with("height", "9")
                .with("patternUnits", "userSpaceOnUse")
                .with("patternTransform", "rotate(45)")
                .text("\n      ")
                .child(
                    Element::new("line")
                        .with("x1", "0")
                        .with("y1", "0")
                        .with("x2", "0")
                        .with("y2", "9")
                        .with("stroke", wall)
                        .with("stroke-width", "3")
                        .with("opacity", "0.5"),
                )
                .text("\n    "),
        )
        .text("\n    ")
        .child(
            Element::paired("filter")
                .with("id", "emc-cell-shadow")
                .with("x", "-12%")
                .with("y", "-12%")
                .with("width", "124%")
                .with("height", "124%")
                .text("\n      ")
                .child(
                    Element::new("feDropShadow")
                        .with("dx", "0")
                        .with("dy", "6")
                        .with("stdDeviation", "12")
                        .with("flood-opacity", if dark { "0.55" } else { "0.16" }),
                )
                .text("\n    "),
        )
        .text("\n    ")
        .child(
            Element::paired("filter")
                .with("id", "emc-glow")
                .with("x", "-40%")
                .with("y", "-40%")
                .with("width", "180%")
                .with("height", "180%")
                .text("\n      ")
                .child(Element::new("feGaussianBlur").with("stdDeviation", "4").with("result", "b"))
                .text("\n      ")
                .child(
                    Element::paired("feMerge")
                        .child(Element::new("feMergeNode").with("in", "b"))
                        .child(Element::new("feMergeNode").with("in", "SourceGraphic")),
                )
                .text("\n    "),
        )
        .text("\n    ")
        .child(
            Element::paired("filter")
                .with("id", "emc-ribbon-soft")
                .with("x", "-30%")
                .with("y", "-30%")
                .with("width", "160%")
                .with("height", "160%")
                .text("\n      ")
                .child(Element::new("feGaussianBlur").with("stdDeviation", "5.5"))
                .text("\n    "),
        )
        .text("\n  ")
}

// Canvas

fn render_canvas_frame(map: &MapGraph, dark: bool) -> Element {
    let fill = if dark { "#0f172a" } else { "#ffffff" };
    let stroke = if dark { "#475569" } else { "#cbd5e1" };
    let mut rect = Element::new("rect").with("class", "emc-canvas-frame");
    rect.flag("data-canvas-bg");
    let rect = rect
        .with_ws("\n      ", "x", "0.75")
        .with("y", "0.75")
        .with("width", n(js::max(0.0, map.width() - 1.5)))
        .with("height", n(js::max(0.0, map.height() - 1.5)))
        .with_ws("\n      ", "fill", fill)
        .with("stroke", stroke)
        .with("stroke-width", "1.5");
    Element::paired("g").with("class", "emc-layer emc-layer-canvas").text("\n    ").child(rect).text("\n  ")
}

fn render_canvas_resize_handles(map: &MapGraph, dark: bool) -> Element {
    let stroke = if dark { "#60a5fa" } else { "#2563eb" };
    let w = map.width();
    let h = map.height();
    let x2 = n(js::max(8.0, w - 8.0));
    let y2 = n(js::max(8.0, h - 8.0));

    // All four edges, so the canvas can grow upwards or to the left without
    // moving everything on it by hand first. Dragging the top or the left edge
    // grows the canvas there and carries the drawing with it, which is what
    // keeps the picture still while the paper around it changes.
    let mut g = Element::paired("g").with("class", "emc-layer emc-layer-canvas-resize");
    let mut edge = |name: &'static str, x1: String, yy1: String, xx2: String, yy2: String| {
        g.push_text("\n    \n    ");
        g.push(
            Element::new("line")
                .with("class", format!("emc-canvas-resize-hit emc-canvas-resize-{name}"))
                .with_ws("\n      ", "data-canvas-resize", name)
                .with("x1", x1.clone())
                .with("y1", yy1.clone())
                .with("x2", xx2.clone())
                .with("y2", yy2.clone())
                .with_ws("\n      ", "stroke", "transparent")
                .with("stroke-width", "16")
                .with("pointer-events", "stroke"),
        );
        g.push_text("\n    ");
        g.push(
            Element::new("line")
                .with("class", format!("emc-canvas-resize-guide emc-canvas-resize-{name}"))
                .with_ws("\n      ", "x1", x1)
                .with("y1", yy1)
                .with("x2", xx2)
                .with("y2", yy2)
                .with_ws("\n      ", "stroke", stroke)
                .with("stroke-width", "1.5")
                .with("opacity", "0.28")
                .with("pointer-events", "none"),
        );
    };
    edge("right", n(w - 1.0), "8".into(), n(w - 1.0), y2.clone());
    edge("bottom", "8".into(), n(h - 1.0), x2.clone(), n(h - 1.0));
    edge("left", "1".into(), "8".into(), "1".into(), y2);
    edge("top", "8".into(), "1".into(), x2, "1".into());
    g.push_text("\n    ");
    g.push(
        Element::new("rect")
            .with("class", "emc-canvas-resize-corner")
            .with("data-canvas-resize", "corner")
            .with_ws("\n      ", "x", n(w - 12.0))
            .with("y", n(h - 12.0))
            .with("width", "12")
            .with("height", "12")
            .with("rx", "2")
            .with_ws("\n      ", "fill", stroke)
            .with("opacity", "0.48"),
    );
    g.push_text("\n  ");
    g
}

// Environment

/// How wide a grab band has to be, in map units, to stay a real target.
///
/// Everything on this map is drawn in map units and scaled by the camera, so a
/// stroke of 18 is 18 units, about two screen pixels at the fitted view of a
/// large map. The reaction and metabolite halos compensate through
/// minHitRadius (MIN_HIT_PX / scale), and membranes and geometry handles need
/// the same, or a compartment can be selected by a test that dispatches on the
/// element but not by a hand aiming at it.
///
/// Capped against the shape itself, so a band never swallows a small compartment.
fn grab_band(rect: &Rect, min_hit_radius: f64) -> f64 {
    let base = 18.0;
    let wanted = js::max(base, or_zero(min_hit_radius) * 2.0);
    let smallest = js::min(rect.w, rect.h);
    js::max(base, js::min(wanted, smallest / 3.0))
}

/// `x="…" y="…" width="…" height="…" rx="…"` of a rectangle.
fn rect_attrs(e: Element, lead: &'static str, r: &Rect, rx: String) -> Element {
    e.with_ws(lead, "x", n(r.x)).with("y", n(r.y)).with("width", n(r.w)).with("height", n(r.h)).with("rx", rx)
}

/// `x="…" y="…" width="…" height="…" rx="…" ry="…"`: a compartment's corners
/// may be elliptical (`cornerAttrs`).
fn corner_rect_attrs(e: Element, lead: &'static str, r: &Rect, corners: Corners, inset: f64) -> Element {
    e.with_ws(lead, "x", n(r.x))
        .with("y", n(r.y))
        .with("width", n(r.w))
        .with("height", n(r.h))
        .with("rx", n(js::max(0.0, corners.rx - inset)))
        .with("ry", n(js::max(0.0, corners.ry - inset)))
}

fn render_environment(map: &MapGraph, dark: bool, min_hit_radius: f64) -> Option<Element> {
    if map.environment.hidden == Some(true) {
        return None;
    }
    let s = compartment_style("s", dark);
    let r = &map.environment.rect;
    let rx = || "26".to_string();
    let fill = rect_attrs(Element::new("rect"), " ", r, rx())
        .with("fill", s.fill)
        .with("stroke", s.stroke)
        .with("stroke-width", "2")
        .with("stroke-dasharray", "10 7");
    let hit = rect_attrs(
        Element::new("rect").with("class", "emc-boundary-hit emc-environment-boundary").with("data-boundary", environment_key()),
        "\n      ",
        r,
        rx(),
    )
    .with("fill", "none")
    .with("stroke", "transparent")
    .with("stroke-width", n(grab_band(r, min_hit_radius)))
    .with_ws("\n      ", "pointer-events", "stroke");
    let glow = rect_attrs(Element::new("rect").with("class", "emc-boundary-glow"), " ", r, rx())
        .with("fill", "none")
        .with("stroke", "transparent")
        .with("stroke-width", "5")
        .with_ws("\n      ", "pointer-events", "none");
    Some(
        Element::paired("g")
            .with("class", "emc-layer emc-layer-env")
            .text("\n    ")
            .child(fill)
            .text("\n    ")
            .child(hit)
            .text("\n    ")
            .child(glow)
            .text("\n  "),
    )
}

// Cells

/// The compartments of a cell, outermost first (stable on equal depth).
fn by_depth(cell: &MapCell) -> Vec<&MapCompartment> {
    let mut ordered: Vec<&MapCompartment> = cell.compartments.iter().collect();
    ordered.sort_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap_or(std::cmp::Ordering::Equal));
    ordered
}

fn render_cells(cx: &Ctx) -> Element {
    let map = cx.map;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-cells");
    for cell in &map.cells {
        let inactive = cx.inactive(Some(&cell.id));
        let mut g = Element::paired("g")
            .with("class", if inactive { "emc-cell is-inactive-species" } else { "emc-cell" })
            .with("data-cell", cell.id.clone());
        if inactive {
            g.attr("opacity", "0.32");
        }

        // Compartments, outermost first, so nesting reads as containment. Shapes and
        // labels go in two passes: a nested compartment's opaque label pill would
        // otherwise be painted over by the compartment drawn after it.
        let ordered = by_depth(cell);
        for comp in &ordered {
            g.push(render_compartment_shape(comp, cx.dark, cell, comp.depth == 0.0, cx.min_hit_radius));
        }

        let outer = ordered
            .iter()
            .find(|c| Some(c.id.as_str()) == cell.outer_compartment.as_deref())
            .or_else(|| ordered.iter().find(|c| c.parent_id.is_none()));
        if let Some(outer) = outer {
            g.push(render_cell_boundary(cell, outer, cx.dark));
        }

        // Cell identity plate, standing on the outer membrane's top-left corner. It
        // is as wide as its text and grows with it, up to the width of the cell.
        let r = &cell.rect;
        let mu = if inactive { None } else { cx.s.growth.get(&cell.id).copied() };
        let mu_text = match mu {
            None => String::new(),
            Some(mu) => format!("  ·  µ = {} h⁻¹", format_flux(Some(mu), 4)),
        };
        let plate_text = format!(
            "{}{}",
            cell.name,
            if inactive { format!(" · {}", cx.s.inactive_species_label) } else { mu_text }
        );
        let text_width = utf16_len(&plate_text) as f64 * PLATE_CHAR;
        let plate_width = text_width + 44.0;
        let fit = fit_scale(cell_plate_fit(cell, Some(&map.environment.rect), &plate_text));
        let frame = Element::paired("g")
            .with("class", "emc-frame-label")
            .with("style", format!("--fit:{fit}"))
            .text("\n        ")
            .child(
                Element::new("rect")
                    .with("x", "0")
                    .with("y", "-30")
                    .with("width", n(plate_width))
                    .with("height", "30")
                    .with("rx", "15")
                    .with_ws("\n          ", "fill", cell.color.clone())
                    .with("opacity", "0.14"),
            )
            .text("\n        ")
            .child(Element::new("circle").with("cx", "17").with("cy", "-15").with("r", "6").with("fill", cell.color.clone()))
            .text("\n        ")
            .child(
                Element::paired("text")
                    .with("class", "emc-cell-name")
                    .with("x", "30")
                    .with("y", "-10")
                    .with("textLength", n(text_width))
                    .with("lengthAdjust", "spacingAndGlyphs")
                    .with_ws("\n          ", "fill", cell.color.clone())
                    .text(plate_text),
            )
            .text("\n      ");
        g.push(
            Element::paired("g")
                .with("class", "emc-cell-plate")
                .with("data-cell-plate", cell.id.clone())
                .with("transform", format!("translate({} {})", n(r.x), n(r.y - 10.0)))
                .text("\n      ")
                .child(frame)
                .text("\n    "),
        );
        out.push(g);
    }
    out
}

fn render_cell_boundary(cell: &MapCell, outer: &MapCompartment, dark: bool) -> Element {
    let r = &outer.rect;
    let corners = compartment_corners(outer);
    let boundary_id = boundary_key(Some(&cell.id), &outer.id);
    // A border chosen in the inspector wins over the species colour.
    let stroke = if truthy(&cell.stroke) {
        cell.stroke.clone().unwrap_or_default()
    } else if !cell.color.is_empty() {
        cell.color.clone()
    } else if dark {
        "#60a5fa".into()
    } else {
        "#2563eb".into()
    };
    corner_rect_attrs(
        Element::new("rect")
            .with("class", "emc-cell-boundary")
            .with("data-cell-boundary", cell.id.clone())
            .with_ws("\n    ", "data-boundary", boundary_id),
        "\n    ",
        r,
        corners,
        0.0,
    )
    .with_ws("\n    ", "fill", "none")
    .with("stroke", stroke)
    .with("stroke-width", "3")
    .with("opacity", "0.72")
    .with_ws("\n    ", "pointer-events", "stroke")
}

/// The drawn identity of a compartment (`visualId ?? id`).
fn visual_id(comp: &MapCompartment) -> &str {
    if comp.visual_id.is_empty() { &comp.id } else { &comp.visual_id }
}

fn chosen<'a>(custom: &'a Option<String>, theme: &'a str) -> String {
    if truthy(custom) { custom.clone().unwrap_or_default() } else { theme.to_string() }
}

fn render_compartment_shape(comp: &MapCompartment, dark: bool, cell: &MapCell, is_outer: bool, min_hit_radius: f64) -> Element {
    let theme = compartment_style(visual_id(comp), dark);
    // Colours chosen in the inspector are map-only and win over the theme.
    let fill = chosen(&comp.fill, theme.fill);
    let stroke = chosen(&comp.stroke, theme.stroke);
    let r = &comp.rect;
    let corners = compartment_corners(comp);
    let rect_attrs = |e: Element, lead: &'static str, r: &Rect, _: ()| corner_rect_attrs(e, lead, r, corners, 0.0);
    let boundary_id = boundary_key(Some(&cell.id), &comp.id);
    let mut out = Element::paired("g")
        .with("class", format!("emc-compartment emc-comp-{}", comp.id))
        .with("data-compartment", comp.id.clone());

    let mut space = rect_attrs(
        Element::new("rect")
            .with("class", "emc-compartment-space")
            .with("data-cell-space", cell.id.clone())
            .with("data-compartment-space", comp.id.clone()),
        " ",
        r,
        (),
    )
    .with("fill", fill);
    if is_outer {
        space.attr("filter", "url(#emc-cell-shadow)");
    }
    out.push(space);

    match comp.membrane.as_str() {
        "bilayer" => {
            // Two concentric strokes read as the two leaflets of a lipid bilayer.
            out.push(
                rect_attrs(Element::new("rect"), " ", r, ())
                    .with("fill", "none")
                    .with("stroke", stroke.clone())
                    .with("stroke-width", "4.5"),
            );
            out.push(
                Element::new("rect")
                    .with("x", n(r.x + 5.0))
                    .with("y", n(r.y + 5.0))
                    .with("width", n(js::max(0.0, r.w - 10.0)))
                    .with("height", n(js::max(0.0, r.h - 10.0)))
                    .with("rx", n(js::max(0.0, corners.rx - 5.0)))
                    .with("ry", n(js::max(0.0, corners.ry - 5.0)))
                    .with("fill", "none")
                    .with("stroke", stroke)
                    .with("stroke-width", "1.4")
                    .with("opacity", "0.55"),
            );
        }
        "wall" => {
            out.push(rect_attrs(Element::new("rect"), " ", r, ()).with("fill", "url(#emc-wall)").with("opacity", "0.7"));
            out.push(
                rect_attrs(Element::new("rect"), " ", r, ())
                    .with("fill", "none")
                    .with("stroke", stroke)
                    .with("stroke-width", "5"),
            );
        }
        _ => {
            out.push(
                rect_attrs(Element::new("rect"), " ", r, ())
                    .with("fill", "none")
                    .with("stroke", stroke)
                    .with("stroke-width", "1.8")
                    .with("stroke-dasharray", "7 5"),
            );
        }
    }

    // Wide transparent stroke: easy membrane hit-testing without making the
    // compartment interior clickable.
    out.push(
        rect_attrs(Element::new("rect").with("class", "emc-boundary-hit").with("data-boundary", boundary_id), " ", r, ())
            .with("fill", "none")
            .with("stroke", "transparent")
            .with("stroke-width", n(grab_band(r, min_hit_radius)))
            .with("pointer-events", "stroke"),
    );
    // Paints the hover; takes no pointer events, so it cannot move the band that does.
    out.push(
        rect_attrs(Element::new("rect").with("class", "emc-boundary-glow"), " ", r, ())
            .with("fill", "none")
            .with("stroke", "transparent")
            .with("stroke-width", "5")
            .with("pointer-events", "none"),
    );
    out
}

// Name plates of compartments, cells and the medium
//
// A plate is a pill with its text, drawn once at its base size and then scaled
// as a whole by the stylesheet: up as the map is zoomed out, so that a name
// stays the same size on screen, and never past what its own compartment has
// room for (--fit). Scaled together, the pill and the text cannot disagree
// about how big the text is. Enlarging the text alone would, at the fitted
// view of a large map, run it out of both ends of a pill three times too short
// for it, across the membrane it is supposed to sit inside.
//
// The width of a piece of text is not knowable here, so it is not left to
// chance: textLength pins it to the estimate the pill is cut to.
const LABEL_CHAR: f64 = 8.6; // one capital of a compartment name, at base size
const LABEL_HEIGHT: f64 = 19.0;
const PLATE_CHAR: f64 = 9.4; // one character of a cell name, at base size
const FRAME_SCALE_MAX: f64 = 16.0;
const FRAME_SCALE_MIN: f64 = 0.2;
const FRAME_SCALE_GOOD: f64 = 6.0; // a plate that can grow this far is not cramped

/// `frameLabelScale(minHitRadius)`: how much the plates are enlarged at the
/// current zoom; 1 once zoomed in to full size.
pub fn frame_label_scale(min_hit_radius: f64) -> f64 {
    // minHitRadius is 12 px in map units, so this is 1 / zoom.
    js::min(FRAME_SCALE_MAX, js::max(1.0, or_zero(min_hit_radius) / 12.0))
}

/// How far a cell's name plate may grow: as wide as the cell, and no taller
/// than the strip of medium above the cell it stands in.
fn cell_plate_fit(cell: &MapCell, env_rect: Option<&Rect>, text: &str) -> f64 {
    let width = utf16_len(text) as f64 * PLATE_CHAR + 44.0;
    let above = match env_rect {
        Some(env) => cell.rect.y - 10.0 - env.y,
        None => f64::INFINITY,
    };
    js::max(1.0, js::min_n([cell.rect.w / width, above / 32.0, FRAME_SCALE_MAX]))
}

/// A fit factor as the stylesheet wants it.
fn fit_scale(value: f64) -> String {
    if value.is_finite() { js::to_fixed(js::max(FRAME_SCALE_MIN, value), 3) } else { "1000".into() }
}

/// The largest scale, up to `most`, at which a plate anchored at (x, y) stays
/// clear of every rectangle in `taken`. A plate only ever grows away from its
/// anchor, so once it touches something it keeps touching it: a bisection finds
/// the edge. Zero when it is not clear even at its smallest.
#[allow(clippy::too_many_arguments)]
fn clear_scale(x: f64, y: f64, bx: f64, by: f64, w: f64, h: f64, taken: &[Rect], most: f64) -> f64 {
    let hits = |scale: f64| {
        let left = x + bx * scale;
        let top = y + by * scale;
        taken.iter().any(|t| left < t.x + t.w && t.x < left + w * scale && top < t.y + t.h && t.y < top + h * scale)
    };
    if !hits(most) {
        return most;
    }
    let mut low = FRAME_SCALE_MIN;
    if hits(low) {
        return 0.0;
    }
    let mut high = most;
    for _ in 0..14 {
        let mid = (low + high) / 2.0;
        if hits(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    low
}

struct FrameLabel<'a> {
    id: &'a str,
    label: &'a str,
    rect: &'a Rect,
    fill: String,
    stroke: String,
    text: &'static str,
    margin: f64,
    centred: bool,
    taken: Vec<Rect>,
    limit: f64,
    cls: &'static str,
    position: Option<Point>,
}

/// One name plate in a corner of `rect`, or centred on its top edge.
///
/// Where it goes and how large it may get are settled together, and for every
/// zoom at once: the corner is the first that lets the plate reach a good size
/// without running into anything in `taken` (mostly the compartments nested
/// inside), and that size is its limit. Chosen that way, a plate neither jumps
/// from corner to corner as the map is zoomed nor grows over its neighbours.
fn frame_label(f: FrameLabel) -> Option<Element> {
    let text = js_trim(f.label);
    if text.is_empty() {
        return None;
    }
    let r = f.rect;
    let text_width = utf16_len(text) as f64 * LABEL_CHAR;
    let w = text_width + 18.0;
    let h = LABEL_HEIGHT;
    // As large as the others, unless the compartment is too small to hold that.
    let room = js::min_n([(r.w - 2.0 * f.margin) / w, (r.h * 0.34) / h, f.limit, FRAME_SCALE_MAX]);
    let most = js::max(FRAME_SCALE_MIN, room);

    let mut centred = f.centred;
    let (x, y, bx, by);
    let mut fit = most;
    if let Some(p) = f.position {
        x = p.x;
        y = p.y;
        bx = -w / 2.0;
        by = -h / 2.0;
        centred = true;
    } else if centred {
        // Astride the top edge, in the middle of it.
        x = r.x + r.w / 2.0;
        y = r.y;
        bx = -w / 2.0;
        by = -h / 2.0;
        let clear = clear_scale(x, y, bx, by, w, h, &f.taken, most);
        if clear > 0.0 {
            fit = clear;
        }
    } else {
        let inset = 15.0;
        let corners = [
            (r.x + f.margin, r.y + inset, 0.0, 0.0),
            (r.x + r.w - f.margin, r.y + inset, -w, 0.0),
            (r.x + f.margin, r.y + r.h - inset, 0.0, -h),
            (r.x + r.w - f.margin, r.y + r.h - inset, -w, -h),
        ]
        .map(|(cx, cy, bx, by)| (cx, cy, bx, by, clear_scale(cx, cy, bx, by, w, h, &f.taken, most)));
        // The top-left corner is where a name is looked for, so it keeps the plate
        // as long as it lets it grow to a good size; another corner takes it only
        // where that one is cramped and it is not.
        let best = js::max_n(corners.iter().map(|c| c.4));
        let enough = js::min(best, FRAME_SCALE_GOOD);
        let chosen = corners.iter().find(|c| c.4 >= enough).copied().unwrap_or(corners[0]);
        x = chosen.0;
        y = chosen.1;
        bx = chosen.2;
        by = chosen.3;
        if chosen.4 > 0.0 {
            fit = chosen.4;
        }
    }

    let class = if f.cls.is_empty() { "emc-comp-label".to_string() } else { format!("emc-comp-label {}", f.cls) };
    let frame = Element::paired("g")
        .with("class", "emc-frame-label")
        .with("style", format!("--fit:{}", fit_scale(fit)))
        .text("\n      ")
        .child(
            Element::new("rect")
                .with("x", n(bx))
                .with("y", n(by))
                .with("width", n(w))
                .with("height", js::number_to_string(h))
                .with("rx", js::number_to_string(h / 2.0))
                .with_ws("\n        ", "fill", f.fill)
                .with("stroke", f.stroke)
                .with("stroke-width", if centred { "1.2" } else { "0" })
                .with("opacity", if centred { "1" } else { "0.18" }),
        )
        .text("\n      ")
        .child(
            Element::paired("text")
                .with("x", n(bx + w / 2.0))
                .with("y", n(by + 13.5))
                .with("text-anchor", "middle")
                .with("textLength", n(text_width))
                .with_ws("\n        ", "lengthAdjust", "spacingAndGlyphs")
                .with("fill", f.text)
                .with("class", "emc-comp-text")
                .text(text.to_string()),
        )
        .text("\n    ");
    Some(
        Element::paired("g")
            .with("class", class)
            .with("data-compartment-label", f.id.to_string())
            .with("transform", format!("translate({} {})", n(x), n(y)))
            .text("\n    ")
            .child(frame)
            .text("\n  "),
    )
}

/// A dragged label position (fractions of the rectangle), on the outline.
fn compartment_label_position(rect: &Rect, extra: &serde_json::Map<String, Value>, corners: Corners) -> Option<Point> {
    let position = extra.get("labelPosition")?;
    let (Some(Value::Number(px)), Some(Value::Number(py))) = (position.get("x"), position.get("y")) else {
        return None;
    };
    let (px, py) = (px.as_f64()?, py.as_f64()?);
    if !px.is_finite() || !py.is_finite() {
        return None;
    }
    let p = closest_perimeter_point_corners(rect, Point::new(rect.x + px * rect.w, rect.y + py * rect.h), corners);
    Some(p.point())
}

fn render_compartment_labels(cx: &Ctx) -> Element {
    let map = cx.map;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-compartment-labels");
    let env = &map.environment;
    if env.hidden != Some(true) {
        let taken = map
            .cells
            .iter()
            .map(|cell| {
                let plate = 40.0 * cell_plate_fit(cell, Some(&env.rect), &cell.name);
                Rect::new(cell.rect.x, cell.rect.y - plate, cell.rect.w, cell.rect.h + plate)
            })
            .collect();
        let s = compartment_style("s", cx.dark);
        if let Some(label) = frame_label(FrameLabel {
            id: if env.id.is_empty() { "s" } else { &env.id },
            label: &env.label,
            rect: &env.rect,
            fill: s.fill.into(),
            stroke: s.stroke.into(),
            text: s.text,
            margin: 22.0,
            centred: false,
            taken,
            limit: f64::INFINITY,
            cls: "emc-env-label",
            position: compartment_label_position(&env.rect, &env.extra, Corners::circular(26.0)),
        }) {
            out.push(label);
        }
    }
    for cell in &map.cells {
        let inactive = cx.inactive(Some(&cell.id));
        let mut g = Element::paired("g").with("data-cell", cell.id.clone());
        if inactive {
            g.attr("opacity", "0.32");
        }
        for comp in by_depth(cell) {
            if let Some(label) = render_compartment_label(comp, cx.dark, cell) {
                g.push(label);
            }
        }
        out.push(g);
    }
    out
}

/// Compartment name plate. Rounded organelles get it straddling the top edge;
/// boxy compartments keep it in a corner, out of the pathway and clear of the
/// compartments nested inside them.
fn render_compartment_label(comp: &MapCompartment, dark: bool, cell: &MapCell) -> Option<Element> {
    let vid = visual_id(comp);
    let corners = compartment_corners(comp);
    let centred = is_round_compartment(comp);
    let children: Vec<&MapCompartment> =
        cell.compartments.iter().filter(|o| o.parent_id.as_deref() == Some(comp.id.as_str())).collect();
    let mut limit = f64::INFINITY;
    if centred {
        // A plate astride a top edge reaches half its height into whatever is
        // above. Where two such compartments are nested closely (the matrix in
        // the intermembrane space), each plate gets half of the band between
        // their edges, or they are written over one another.
        if let Some(parent) = comp.parent_id.as_deref().and_then(|p| cell.compartments.iter().find(|o| o.id == p)) {
            limit = js::min(limit, js::max(0.0, comp.rect.y - parent.rect.y) / LABEL_HEIGHT);
        }
        for child in &children {
            if is_round_compartment(child) {
                limit = js::min(limit, js::max(0.0, child.rect.y - comp.rect.y) / LABEL_HEIGHT);
            }
        }
    }
    let theme: CompartmentStyle = compartment_style(vid, dark);
    frame_label(FrameLabel {
        id: &comp.id,
        label: &comp.label,
        rect: &comp.rect,
        // The name plate keeps the colours the compartment is drawn in.
        fill: chosen(&comp.fill, theme.fill),
        stroke: chosen(&comp.stroke, theme.stroke),
        text: theme.text,
        margin: js::max(14.0, js::max(corners.rx, corners.ry) * 0.45),
        centred,
        taken: children.iter().map(|o| o.rect).collect(),
        limit,
        cls: "",
        position: compartment_label_position(&comp.rect, &comp.extra, corners),
    })
}

// Pathway ribbons

/// Omix-style pathway ribbons. They reuse the exact edited reaction geometry,
/// but are drawn as broad translucent strokes underneath the real reaction
/// layer. This keeps flux colours, metabolite colours, arrowheads and editing
/// semantics untouched.
fn render_pathway_ribbons(cx: &Ctx) -> Option<Element> {
    let s = cx.s;
    if s.pathway_ribbon_mode == PathwayRibbonMode::Off || s.pathway_ribbon_groups.is_empty() {
        return None;
    }
    let or_default = |v: f64, d: f64| if v == 0.0 || v.is_nan() { d } else { v };
    let ribbon_width = js::min(120.0, js::max(20.0, or_default(s.pathway_ribbon_width, 56.0)));
    let alpha = js::min(0.45, js::max(0.04, or_default(s.pathway_ribbon_opacity, 0.18)));
    let outer_width = ribbon_width * 1.34;
    let inner_opacity = alpha;
    let outer_opacity = alpha * 0.34;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-pathway-ribbons").with("pointer-events", "none");

    for seg in &cx.map.segments {
        if seg.kind == SegmentKind::Currency {
            continue;
        }
        let Some(rxn) = cx.map.reactions.get(&seg.reaction_id) else { continue };
        let Some(group) = s.pathway_ribbon_groups.get(&seg.reaction_id) else { continue };
        if group.is_empty() || is_missing_group(group) {
            continue;
        }
        let target = cx.node(seg.node_id.as_deref());
        if target.is_some_and(|t| t.hidden == Some(true)) {
            continue;
        }
        if !s.show_currency && target.is_some_and(is_secondary_node) {
            continue;
        }
        let Some(color) = s.pathway_ribbon_colors.get(&seg.reaction_id).filter(|c| !c.is_empty()) else { continue };
        let d = curve_path(&segment_curve(seg, rxn));
        let ribbon = |class: &'static str, width: f64, opacity: f64| {
            Element::new("path")
                .with("class", class)
                .with("data-ribbon-rxn", seg.reaction_id.clone())
                .with("data-ribbon-group", group.clone())
                .with_ws("\n      ", "d", d.clone())
                .with("fill", "none")
                .with("stroke", color.clone())
                .with("stroke-width", n(width))
                .with_ws("\n      ", "stroke-linecap", "round")
                .with("stroke-linejoin", "round")
                .with("opacity", n(opacity))
        };
        out.push(ribbon("emc-pathway-ribbon emc-pathway-ribbon-outer", outer_width, outer_opacity).with_ws(
            "\n      ",
            "filter",
            "url(#emc-ribbon-soft)",
        ));
        out.push(ribbon("emc-pathway-ribbon emc-pathway-ribbon-core", ribbon_width, inner_opacity));
    }
    Some(out)
}

fn active_cross_feeding_reaction_ids<'a>(cx: &'a Ctx) -> HashSet<&'a str> {
    let mut ids = HashSet::new();
    if !cx.s.show_cross_feeding {
        return ids;
    }
    if let Some(explorer) = &cx.explorer {
        for event in explorer.visible_events {
            if let Some(t) = event.producer_transfer.as_deref().filter(|t| !t.is_empty()) {
                ids.insert(t);
            }
            if let Some(t) = event.consumer_transfer.as_deref().filter(|t| !t.is_empty()) {
                ids.insert(t);
            }
        }
    }
    ids
}

// Segments

const ESCHER_REACTION_STROKE_WIDTH: f64 = 7.5;

// A reaction that carries no flux in the solution, drawn the way a
// switched-off reaction is in a topology figure: grey and dashed, so what the
// solution uses stands out from what it does not.
fn off_color(dark: bool) -> &'static str {
    if dark { "#64748b" } else { "#9b9da5" }
}

fn off_dash(width: f64) -> String {
    format!("{} {}", n(js::max(6.0, width * 1.5)), n(js::max(5.0, width * 1.15)))
}

/// A segment collected before it is written: a line that hops over another
/// needs to know where the others run.
struct Drawn {
    path: Element,
    d: String,
    item: HopItem,
    hops: bool,
    reaction_id: String,
}

fn render_segments(cx: &Ctx, cross_feeding_reactions: &HashSet<&str>) -> Element {
    let s = cx.s;
    let highlight_cross_feeding = s.show_cross_feeding;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-segments");
    let mut arrows: Vec<Element> = Vec::new();
    // Segments are collected before they are written: a line that hops over
    // another needs to know where the others run.
    let mut drawn: Vec<Drawn> = Vec::new();

    for seg in &cx.map.segments {
        let Some(rxn) = cx.map.reactions.get(&seg.reaction_id) else { continue };
        let target = cx.node(seg.node_id.as_deref());
        if !s.show_currency && (seg.kind == SegmentKind::Currency || target.is_some_and(is_secondary_node)) {
            continue;
        }
        // A lane is the stretch of a transported metabolite's path that lies in the
        // transporter, and the transporter's own glyph draws it, as an arrow. Drawn
        // here as well it would be a stub of full line width poking out of both
        // sides of the glyph, doubling up with the end of the curve that meets it.
        if seg.kind == SegmentKind::TransportLane && rxn.transport_lanes.as_ref().map_or(0, Vec::len) >= 2 {
            continue;
        }
        if target.is_some_and(|t| t.hidden == Some(true)) {
            continue;
        }

        let rid = seg.reaction_id.as_str();
        let v = cx.flux(rid);
        let disabled = cx.disabled(rid);
        let inactive = cx.inactive(rxn.cell_id.as_deref());
        let solved = if disabled || inactive { 0.0 } else { cx.solved(rid) };
        let width = match s.reaction_width_mode {
            ReactionWidthMode::Flux => cx.scale.width(v),
            ReactionWidthMode::Constant => ESCHER_REACTION_STROKE_WIDTH,
        };
        let off = cx.is_off(rid, inactive);
        let color = if disabled {
            "#db2777".to_string()
        } else if off {
            off_color(cx.dark).to_string()
        } else {
            cx.reaction_display_color(rid, v, rxn.own_color())
        };
        let active = solved.abs() > 1e-9;
        let is_sel = cx.selected_reaction == Some(rid);
        let is_multi = cx.multi_has_reaction(rid);

        let is_cross_feed = cross_feeding_reactions.contains(rid);
        let cross_feed_opacity = if inactive {
            0.22
        } else if highlight_cross_feeding && !is_sel && !is_multi {
            if is_cross_feed { 1.0 } else { 0.14 }
        } else {
            1.0
        };
        let mut cls = format!("emc-seg emc-seg-{}", seg.kind.as_str());
        for (on, c) in [
            (disabled, " is-disabled-reaction"),
            (off, " is-off"),
            (inactive, " is-inactive-species"),
            (active, " is-active"),
            (is_cross_feed, " is-crossfeeding"),
            (is_sel, " is-selected"),
            (is_multi, " is-multi-selected"),
        ] {
            if on {
                cls.push_str(c);
            }
        }
        // Arrowheads follow the sign of the flux, so a reversed reaction reads as
        // reversed on the map instead of only in the numbers.
        let forward = solved >= 0.0;
        let want_arrow = seg.side.as_deref() == Some(if forward { "product" } else { "substrate" })
            || (seg.kind == SegmentKind::Boundary && forward);
        let geometry = segment_geometry(
            seg,
            rxn,
            target,
            !disabled && !inactive && want_arrow && (active || seg.kind == SegmentKind::Boundary),
            width,
        );
        let node_attr = seg.node_id.as_deref().filter(|id| !id.is_empty());
        if !geometry.d.is_empty() {
            // `d` goes in when the path is written, hopped or not.
            let mut path = Element::new("path")
                .with("class", cls)
                .with("fill", "none")
                .with("stroke", color.clone())
                .with("stroke-width", n(width))
                .with("stroke-linecap", if target.is_some() || geometry.arrow.is_some() { "butt" } else { "round" })
                .with("opacity", n(cross_feed_opacity));
            if disabled {
                path.attr("stroke-dasharray", "8 5");
            } else if off {
                path.attr("stroke-dasharray", off_dash(width));
            }
            path.attr("data-rxn", rid.to_string());
            if let Some(id) = node_attr {
                path.attr("data-node", id.to_string());
            }
            drawn.push(Drawn {
                path,
                d: geometry.d.clone(),
                item: HopItem { curve: geometry.curve.clone(), reaction_id: rid.to_string(), width },
                // Cofactor spurs are short and many; a hop on one, or over one, says nothing.
                hops: seg.kind != SegmentKind::Currency && !target.is_some_and(is_secondary_node),
                reaction_id: rid.to_string(),
            });
        }
        if let Some(arrow) = geometry.arrow {
            let points = arrow.points.iter().map(|p| format!("{},{}", n(p.x), n(p.y))).collect::<Vec<_>>().join(" ");
            let mut polygon = Element::new("polygon")
                .with("class", "emc-arrow")
                .with("points", points)
                .with("fill", color)
                .with("opacity", n(cross_feed_opacity))
                .with("data-rxn", rid.to_string());
            if let Some(id) = node_attr {
                polygon.attr("data-node", id.to_string());
            }
            arrows.push(polygon);
        }
    }

    let hopping: Vec<usize> = if s.line_hops { (0..drawn.len()).filter(|&i| drawn[i].hops).collect() } else { Vec::new() };
    let items: Vec<HopItem> = hopping.iter().map(|&i| drawn[i].item.clone()).collect();
    let found = if s.line_hops { find_line_hops(&items, MAX_LINE_HOPS) } else { Default::default() };
    let mut hops_of: HashMap<usize, &Vec<crate::line_hops::Hop>> = HashMap::new();
    for (k, list) in &found {
        hops_of.insert(hopping[*k], list);
    }
    for (i, item) in drawn.into_iter().enumerate() {
        let d = match hops_of.get(&i) {
            Some(hops) => hopped_path(&item.item.curve, hops),
            None => item.d,
        };
        let mut path = item.path;
        // `d` sits after `class`, where the JavaScript markup has it.
        path.attrs.insert(1, crate::svg::Attr { lead: " ", name: "d", value: Some(d.into()) });
        if let Some(title) = cx.reaction_tooltip(&item.reaction_id) {
            path.paired = true;
            path.push(Element::paired("title").text(title.to_string()));
        }
        out.push(path);
    }

    for a in arrows {
        out.push(a);
    }
    out
}

fn segment_geometry(seg: &Segment, rxn: &MapReaction, node: Option<&Node>, arrow: bool, width: f64) -> EndpointGeometry {
    let radius = node.map(endpoint_node_radius);
    if node.is_some()
        && !matches!(seg.kind, SegmentKind::Boundary | SegmentKind::TransportLane | SegmentKind::Axis)
    {
        let (b1, b2) = branch_controls(seg);
        let (metabolite, reaction) = if seg.is_substrate() { (seg.from, seg.to) } else { (seg.to, seg.from) };
        return reaction_endpoint_geometry(&[reaction, b2, b1, metabolite], radius, arrow, width);
    }
    if arrow || node.is_some() {
        return reaction_endpoint_geometry(&[seg.from, seg.to], radius, arrow, width);
    }
    let curve = segment_curve(seg, rxn);
    EndpointGeometry { d: curve_path(&curve), arrow: None, curve }
}

/// `segmentCurve(seg)`: the line or cubic a segment without a node or
/// arrowhead is drawn as.
fn segment_curve(seg: &Segment, _rxn: &MapReaction) -> Vec<Point> {
    let (from, to) = (seg.from, seg.to);
    if matches!(seg.kind, SegmentKind::Axis | SegmentKind::Boundary | SegmentKind::TransportLane) {
        return vec![from, to];
    }
    let (b1, b2) = branch_controls(seg);
    // metabolite -> reaction backbone
    if seg.is_substrate() {
        return vec![from, b1, b2, to];
    }
    // reaction backbone -> metabolite; semantic b2 is reaction-side, b1 metabolite-side.
    vec![from, b2, b1, to]
}

/// `curvePath(curve)`.
fn curve_path(curve: &[Point]) -> String {
    let (from, to) = (curve[0], curve[curve.len() - 1]);
    if curve.len() == 2 {
        return format!("M {} {} L {} {}", n(from.x), n(from.y), n(to.x), n(to.y));
    }
    let (c1, c2) = (curve[1], curve[2]);
    format!("M {} {} C {} {}, {} {}, {} {}", n(from.x), n(from.y), n(c1.x), n(c1.y), n(c2.x), n(c2.y), n(to.x), n(to.y))
}

/// A legacy single control point (`seg.control`), if the segment has one.
fn legacy_control(seg: &Segment) -> Option<Point> {
    let q = seg.extra.get("control")?;
    if !js::truthy(q) {
        return None;
    }
    let coord = |k: &str| q.get(k).map_or(f64::NAN, js::to_number);
    Some(Point::new(coord("x"), coord("y")))
}

/// `branchControls(seg)`: (b1 at the metabolite end, b2 at the reaction end).
fn branch_controls(seg: &Segment) -> (Point, Point) {
    let (metabolite, reaction) = if seg.is_substrate() { (seg.from, seg.to) } else { (seg.to, seg.from) };
    if let (Some(b1), Some(b2)) = (seg.b1, seg.b2) {
        return (b1, b2);
    }
    if let Some(q) = legacy_control(seg) {
        return (
            Point::new(metabolite.x + (2.0 / 3.0) * (q.x - metabolite.x), metabolite.y + (2.0 / 3.0) * (q.y - metabolite.y)),
            Point::new(reaction.x + (2.0 / 3.0) * (q.x - reaction.x), reaction.y + (2.0 / 3.0) * (q.y - reaction.y)),
        );
    }
    (
        Point::new(metabolite.x + (reaction.x - metabolite.x) * 0.34, metabolite.y + (reaction.y - metabolite.y) * 0.34),
        Point::new(metabolite.x + (reaction.x - metabolite.x) * 0.72, metabolite.y + (reaction.y - metabolite.y) * 0.72),
    )
}

/// The area a multi-selection covers, shaded.
///
/// Outlining each selected mark says which ones are in; it does not say what
/// region they make up, which is what matters when the next action moves,
/// aligns or deletes them as one. The shading is the group, and it is drawn
/// beneath everything so no mark is tinted by it.
fn multi_selection_bounds(map: &MapGraph, multi: Option<&MultiSelection>) -> Option<Rect> {
    let multi = multi?;
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for id in &multi.nodes {
        if let Some(node) = map.nodes.get(id) {
            xs.push(node.x);
            ys.push(node.y);
        }
    }
    for id in &multi.reactions {
        if let Some(rxn) = map.reactions.get(id) {
            xs.push(rxn.x);
            ys.push(rxn.y);
        }
    }
    for id in &multi.texts {
        let Some(annotation) = map.texts.get(id) else { continue };
        let b = text_box_of(annotation);
        xs.push(b.left);
        xs.push(b.right);
        ys.push(b.top);
        ys.push(b.bottom);
    }
    for r in selected_compartment_rects(map, Some(multi)) {
        xs.push(r.x);
        xs.push(r.x + r.w);
        ys.push(r.y);
        ys.push(r.y + r.h);
    }
    // One object is already shown as selected; a region around it says nothing.
    if xs.len() < 2 {
        return None;
    }
    let pad = 30.0;
    let (min_x, max_x) = (js::min_n(xs.iter().copied()), js::max_n(xs.iter().copied()));
    let (min_y, max_y) = (js::min_n(ys.iter().copied()), js::max_n(ys.iter().copied()));
    Some(Rect::new(min_x - pad, min_y - pad, (max_x - min_x) + pad * 2.0, (max_y - min_y) + pad * 2.0))
}

/// Where the compartments in a multi-selection are, in map order.
fn selected_compartment_rects(map: &MapGraph, multi: Option<&MultiSelection>) -> Vec<Rect> {
    let Some(multi) = multi.filter(|m| !m.compartments.is_empty()) else { return Vec::new() };
    let mut out = Vec::new();
    for cell in &map.cells {
        for comp in &cell.compartments {
            if multi.compartments.contains(&(cell.id.clone(), comp.id.clone())) {
                out.push(comp.rect);
            }
        }
    }
    out
}

/// The shaded area, and a dashed outline round each selected compartment:
/// those move with the selection, membrane and all, and should look it.
fn render_multi_selection_area(b: &Rect, compartments: &[Rect]) -> Element {
    let mut g = Element::paired("g")
        .with("class", "emc-layer emc-layer-multi-area")
        .with("pointer-events", "none")
        .text("\n    ")
        .child(
            Element::new("rect")
                .with("class", "emc-multi-area")
                .with("x", n(b.x))
                .with("y", n(b.y))
                .with("width", n(b.w))
                .with("height", n(b.h))
                .with("rx", "18"),
        );
    for r in compartments {
        g = g.text("\n    ").child(
            Element::new("rect")
                .with("class", "emc-multi-compartment")
                .with("x", n(r.x - 4.0))
                .with("y", n(r.y - 4.0))
                .with("width", n(r.w + 8.0))
                .with("height", n(r.h + 8.0))
                .with("rx", "18")
                .with("fill", "none")
                .with("stroke", "#2563eb")
                .with("stroke-width", "3")
                .with("stroke-dasharray", "10 6"),
        );
    }
    g.text("\n  ")
}

fn render_multi_selection_border(b: &Rect, min_hit_radius: f64) -> Element {
    let stroke_width = js::max(12.0, min_hit_radius);
    let mut g = Element::paired("g").with("class", "emc-layer emc-layer-multi-border");
    g.flag("data-selection-drag");
    g.text("\n    ")
        .child(Element::paired("title").text("Drag the border to move the selection"))
        .text("\n    ")
        .child(
            Element::new("rect")
                .with("class", "emc-multi-drag-hit")
                .with("x", n(b.x))
                .with("y", n(b.y))
                .with("width", n(b.w))
                .with("height", n(b.h))
                .with("rx", "18")
                .with("stroke-width", n(stroke_width)),
        )
        .text("\n    ")
        .child(
            Element::new("rect")
                .with("class", "emc-multi-drag-outline")
                .with("x", n(b.x))
                .with("y", n(b.y))
                .with("width", n(b.w))
                .with("height", n(b.h))
                .with("rx", "18"),
        )
        .text("\n  ")
}

// Markers

fn render_markers(cx: &Ctx) -> Element {
    let map = cx.map;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-markers");

    // A hand-placed transporter records the compartment whose membrane it is on,
    // not what that compartment is drawn as; the cell knows (`i` is drawn as the
    // intermembrane space, a cell's own `s` as the eyespot).
    let mut drawn_as: HashMap<String, &str> = HashMap::new();
    for cell in &map.cells {
        for comp in &cell.compartments {
            drawn_as.insert(format!("{}/{}", cell.id, comp.id), visual_id(comp));
        }
    }

    for (rid, rxn) in &map.reactions {
        let v = cx.flux(rid);
        let disabled = cx.disabled(rid);
        let inactive = cx.inactive(rxn.cell_id.as_deref());
        let solved = if disabled || inactive { 0.0 } else { cx.solved(rid) };
        let off = cx.is_off(rid, inactive);
        let color = if disabled {
            "#db2777".to_string()
        } else if off {
            off_color(cx.dark).to_string()
        } else {
            cx.reaction_display_color(rid, v, rxn.own_color())
        };
        let is_sel = cx.selected_reaction == Some(rid.as_str());
        let is_multi = cx.multi_has_reaction(rid);
        let angle = rxn.axis.map_or(0.0, |a| (js::atan2(a.dir.y, a.dir.x) * 180.0) / PI);
        let mut cls = String::from("emc-marker");
        for (on, c) in [
            (disabled, " is-disabled-reaction"),
            (off, " is-off"),
            (inactive, " is-inactive-species"),
            (is_sel, " is-selected"),
            (is_multi, " is-multi-selected"),
            (solved.abs() > 1e-9, " is-active"),
        ] {
            if on {
                cls.push_str(c);
            }
        }
        let mut glyphs: Vec<Element> = Vec::new();

        if rxn.on_membrane {
            // Multi-solute transporters get one visible lane per transported base.
            // Symport lanes point together; antiport lanes point in opposite directions.
            let drawn = match (rxn.cell_id.as_deref(), rxn.membrane.as_deref()) {
                (Some(c), Some(m)) => drawn_as.get(&format!("{c}/{m}")).copied(),
                _ => None,
            };
            let membrane_id = rxn
                .membrane_visual_id
                .as_deref()
                .or(drawn)
                .or(rxn.membrane.as_deref())
                .unwrap_or("c");
            let s = compartment_style(membrane_id, cx.dark);
            let lanes = rxn.transport_lanes.as_deref().unwrap_or(&[]);
            if lanes.len() >= 2 {
                // One arrow per transported metabolite, drawn where its lane is.
                //
                // The glyph is laid out in the frame of the membrane, not of the
                // reaction's axis: +x points out of the compartment the transporter
                // sits on, which is what a lane's direction is recorded against. The
                // axis may point either way across the membrane, and read against it a
                // symporter drawn the other way round would show both arrows backwards.
                let out_dir = rxn.normal.or(rxn.axis.map(|a| a.dir)).unwrap_or(Point::new(1.0, 0.0));
                let frame = (js::atan2(out_dir.y, out_dir.x) * 180.0) / PI;
                let count = lanes.len() as f64;
                let offsets: Vec<f64> = lanes
                    .iter()
                    .enumerate()
                    .map(|(i, lane)| if lane.offset.is_finite() { lane.offset } else { (i as f64 - (count - 1.0) / 2.0) * 12.0 })
                    .collect();
                let h = js::max_n(offsets.iter().copied()) - js::min_n(offsets.iter().copied()) + 22.0;
                let mut g = Element::paired("g")
                    .with(
                        "class",
                        format!("{cls} emc-transporter emc-transporter-{}", str_opt(&rxn.transport_mode)),
                    )
                    .with("data-rxn", rid.clone())
                    .with("transform", format!("translate({} {}) rotate({})", n(rxn.x), n(rxn.y), n(frame)))
                    .text("\n          ")
                    .child(
                        Element::new("rect")
                            .with("x", "-13")
                            .with("y", n(-h / 2.0))
                            .with("width", "26")
                            .with("height", n(h))
                            .with("rx", "8")
                            .with("fill", s.fill)
                            .with("stroke", s.stroke)
                            .with("stroke-width", "2.4"),
                    )
                    .text("\n          ");
                for (i, lane) in lanes.iter().enumerate() {
                    let direction = if lane.direction == 0.0 || lane.direction.is_nan() { 0.0 } else { lane.direction };
                    let sign = if disabled || inactive {
                        0.0
                    } else {
                        (if solved >= 0.0 { 1.0 } else { -1.0 }) * direction
                    };
                    for e in lane_arrow(offsets[i], sign, &color) {
                        g.push(e);
                    }
                }
                g.push_text("\n        ");
                glyphs.push(g);
            } else {
                glyphs.push(
                    Element::paired("g")
                        .with("class", format!("{cls} emc-transporter"))
                        .with("data-rxn", rid.clone())
                        .with("transform", format!("translate({} {}) rotate({})", n(rxn.x), n(rxn.y), n(angle)))
                        .text("\n          ")
                        .child(
                            Element::new("rect")
                                .with("x", "-13")
                                .with("y", "-11")
                                .with("width", "26")
                                .with("height", "22")
                                .with("rx", "7")
                                .with("fill", s.fill)
                                .with("stroke", s.stroke)
                                .with("stroke-width", "2.4"),
                        )
                        .text("\n          ")
                        .child(
                            Element::new("rect")
                                .with("x", "-4.5")
                                .with("y", "-11")
                                .with("width", "9")
                                .with("height", "22")
                                .with("rx", "4")
                                .with("fill", color.clone())
                                .with("opacity", "0.85"),
                        )
                        .text("\n        "),
                );
            }
        } else {
            glyphs.push(
                Element::paired("g")
                    .with("class", cls)
                    .with("data-rxn", rid.clone())
                    .with("transform", format!("translate({} {}) rotate({})", n(rxn.x), n(rxn.y), n(angle + 90.0)))
                    .text("\n        ")
                    .child(
                        Element::new("rect")
                            .with("x", "-9")
                            .with("y", "-3")
                            .with("width", "18")
                            .with("height", "6")
                            .with("rx", "3")
                            .with("fill", color.clone()),
                    )
                    .text("\n        ")
                    .child(
                        Element::new("rect")
                            .with("x", "-9")
                            .with("y", "-3")
                            .with("width", "18")
                            .with("height", "6")
                            .with("rx", "3")
                            .with("fill", "none")
                            .with("stroke", if cx.dark { "#0f172a" } else { "#ffffff" })
                            .with("stroke-width", "1.1")
                            .with("opacity", "0.8"),
                    )
                    .text("\n      "),
            );
        }
        // Hover text, first in the marker.
        if let (Some(title), Some(glyph)) = (cx.reaction_tooltip(rid), glyphs.first_mut()) {
            let at = if matches!(glyph.children.first(), Some(crate::svg::Node::Text(_))) { 1 } else { 0 };
            glyph.children.insert(at, crate::svg::Node::Element(Element::paired("title").text(title)));
        }
        if disabled {
            glyphs.push(
                Element::paired("g")
                    .with("class", "emc-ko-mark")
                    .with("data-disabled-reaction", rid.clone())
                    .with("transform", format!("translate({} {})", n(rxn.x), n(rxn.y)))
                    .with("stroke", "#db2777")
                    .with("stroke-width", "3")
                    .with("pointer-events", "none")
                    .text("\n        ")
                    .child(Element::paired("title").text("Disabled reaction: both bounds fixed to zero"))
                    .child(Element::new("path").with("d", "M -9 -9 L 9 9 M -9 9 L 9 -9")),
            );
        } else if !cx.s.for_export && cx.s.modified_reaction_ids.contains(rid) {
            // A small amber dot off the marker's corner: the bounds are not
            // the model's. A note to the editor, not part of the figure.
            glyphs.push(
                Element::paired("g")
                    .with("class", "emc-modified-badge")
                    .with("data-modified-reaction", rid.clone())
                    .with("transform", format!("translate({} {})", n(rxn.x + 13.0), n(rxn.y - 13.0)))
                    .with("pointer-events", "none")
                    .text("\n        ")
                    .child(Element::paired("title").text("Bounds changed from the model's"))
                    .child(
                        Element::new("circle")
                            .with("r", "5.5")
                            .with("fill", if cx.dark { "#fbbf24" } else { "#d97706" })
                            .with("stroke", if cx.dark { "#0f172a" } else { "#ffffff" })
                            .with("stroke-width", "1.6"),
                    ),
            );
        }
        if inactive {
            let mut wrapper = Element::paired("g").with("opacity", "0.22");
            for gl in glyphs {
                wrapper.push(gl);
            }
            out.push(wrapper);
        } else {
            for gl in glyphs {
                out.push(gl);
            }
        }
        if is_sel || is_multi {
            out.push(
                Element::new("circle")
                    .with("class", if is_multi { "emc-select-ring is-multi" } else { "emc-select-ring" })
                    .with("cx", n(rxn.x))
                    .with("cy", n(rxn.y))
                    .with("r", "21")
                    .with("fill", "none")
                    .with("stroke", if is_multi { "#2563eb".to_string() } else { color.clone() })
                    .with("stroke-width", "2.5")
                    .with("stroke-dasharray", "4 3"),
            );
        }
    }
    out
}

/// One lane of a transporter: an ordinary arrow, a shaft with a head on it.
///
/// A thick bar with a triangle at one end barely wider than the bar itself
/// would read as a rod with a blunt tip rather than as a direction.
///
/// `y` is where the lane lies across the glyph; `sign` is +1 out of the
/// compartment, -1 into it, 0 if not known.
fn lane_arrow(y: f64, sign: f64, color: &str) -> Vec<Element> {
    let half = 9.5;
    if sign == 0.0 || sign.is_nan() {
        return vec![
            Element::new("line")
                .with("class", "emc-lane")
                .with("x1", js::number_to_string(-half))
                .with("y1", n(y))
                .with("x2", js::number_to_string(half))
                .with("y2", n(y))
                .with("stroke", color.to_string())
                .with("stroke-width", "2.6")
                .with("stroke-linecap", "round"),
        ];
    }
    let tip = half * sign;
    let neck = tip - 7.5 * sign;
    vec![
        Element::new("line")
            .with("class", "emc-lane")
            .with("x1", n(-half * sign))
            .with("y1", n(y))
            .with("x2", n(neck + sign))
            .with("y2", n(y))
            .with("stroke", color.to_string())
            .with("stroke-width", "2.6")
            .with("stroke-linecap", "round"),
        Element::new("polygon")
            .with("class", "emc-lane-head")
            .with("data-lane-direction", if sign > 0.0 { "out" } else { "in" })
            .with("points", format!("{},{} {},{} {},{}", n(tip), n(y), n(neck), n(y - 4.6), n(neck), n(y + 4.6)))
            .with("fill", color.to_string()),
    ]
}

// Hit targets

/// Invisible, constant-size pointer targets.
///
/// A reaction marker is about 26×22 map units across and a metabolite 26. At the
/// zoom the map opens on (28% on a large screen, 19% on a laptop) that is 3 to
/// 7 screen pixels, far under the ~24 px a pointer can reliably hit, so the first
/// thing a new user tries (click a reaction) fails silently. These circles sit
/// *under* the artwork, so the glyphs keep their own hover styling, and they
/// carry the same data- attributes, so a click near a node or marker still
/// resolves to it.
///
/// `data-hit-base` is the glyph's own radius and acts as a floor. `applyCamera()`
/// in the app rewrites `r` from the live zoom so the halo stays a constant size
/// on screen: wide enough to hit when zoomed out, never smaller than the glyph
/// when zoomed in.
fn render_hit_targets(cx: &Ctx) -> Element {
    let map = cx.map;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-hit").with("aria-hidden", "true");
    for (rid, rxn) in &map.reactions {
        let base = if rxn.on_membrane { 15.0 } else { 11.0 };
        out.push(
            Element::new("circle")
                .with("class", "emc-hit emc-hit-reaction")
                .with("data-rxn", rid.clone())
                .with("data-hit-base", js::number_to_string(base))
                .with("cx", n(rxn.x))
                .with("cy", n(rxn.y))
                .with("r", n(js::max(base, cx.min_hit_radius))),
        );
    }
    for node in map.nodes.values() {
        if node.hidden == Some(true) || (!cx.s.show_currency && is_secondary_node(node)) || is_link(node) {
            continue;
        }
        let base = js::max(node.r, 6.0);
        out.push(
            Element::new("circle")
                .with("class", "emc-hit emc-hit-node")
                .with("data-node", node.id.clone())
                .with("data-met", str_opt(&node.merged_id).to_string())
                .with("data-hit-base", n(base))
                .with("cx", n(node.x))
                .with("cy", n(node.y))
                .with("r", n(js::max(base, cx.min_hit_radius))),
        );
    }
    out
}

fn is_link(node: &Node) -> bool {
    node.compartment.as_deref() == Some(LINK_COMPARTMENT)
}

// Nodes

// The Cofactors toggles act on every secondary node (isSecondaryNode): a
// hand-built map draws its cofactors as instances set to Secondary, so a toggle
// that knew only the layout's own helper nodes would leave all of them on screen.

fn render_nodes(cx: &Ctx) -> Element {
    let dark = cx.dark;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-nodes");
    // A colour chosen for the map, or for one metabolite, holds in both themes.
    let style = cx.map.node_style();
    let orange = style.fill.clone().unwrap_or_else(|| if dark { "#f59e0b" } else { "#e67e4c" }.to_string());
    let orange_border = style.stroke.clone().unwrap_or_else(|| if dark { "#7c2d12" } else { "#a64613" }.to_string());
    let fallback_stroke = if dark { "#0f172a" } else { "#ffffff" };

    for node in cx.map.nodes.values() {
        if node.hidden == Some(true) || (!cx.s.show_currency && is_secondary_node(node)) || is_link(node) {
            continue;
        }
        let metabolite_like = node.is_metabolite_kind();
        let secondary = is_secondary_node(node);
        let is_multi = cx.multi_has_node(&node.id);
        let role_class = if !metabolite_like {
            ""
        } else if secondary {
            " emc-node-secondary"
        } else {
            " emc-node-primary"
        };
        let inactive = cx.inactive(node.cell_id.as_deref());
        let cls = format!(
            "emc-node emc-node-{}{role_class}{}{}",
            node.kind.as_str(),
            if inactive { " is-inactive-species" } else { "" },
            if is_multi { " is-multi-selected" } else { "" }
        );
        let circle = Element::new("circle")
            .with("class", cls)
            .with("data-node", node.id.clone())
            .with("data-met", str_opt(&node.merged_id).to_string())
            .with("cx", n(node.x))
            .with("cy", n(node.y));
        let circle = if metabolite_like {
            let radius = if secondary { SIZES.currency_radius } else { SIZES.primary_radius };
            let mut circle = circle
                .with("r", n(radius))
                .with("fill", node.own_fill().map_or_else(|| orange.clone(), str::to_string))
                .with("stroke", node.own_stroke().map_or_else(|| orange_border.clone(), str::to_string))
                .with("stroke-width", "1.8");
            if let Some(title) = cx.node_tooltip(&node.id) {
                circle.paired = true;
                circle.push(Element::paired("title").text(title));
            }
            circle
        } else {
            let compartment = node.visual_compartment.as_deref().or(node.compartment.as_deref()).unwrap_or("");
            let style = compartment_style(compartment, dark);
            circle.with("r", n(node.r)).with("fill", style.stroke).with("stroke", fallback_stroke).with("stroke-width", "2.4")
        };
        if inactive {
            out.push(Element::paired("g").with("opacity", "0.28").child(circle));
        } else {
            out.push(circle);
        }
    }
    out
}

// Edit handles

/// `rotationArm(minHitRadius)`: how far from a reaction's centre its rotation
/// handle stands, in map units.
///
/// Far enough to be told apart from the centre handle it turns about, at any
/// zoom: the handles are kept a constant size on screen, so the gap between two
/// of them has to be kept in screen terms as well. The app recomputes it when
/// the zoom changes without a repaint.
pub fn rotation_arm(min_hit_radius: f64) -> f64 {
    js::min(170.0, js::max(30.0, or_zero(min_hit_radius) * 2.7))
}

/// The handles of the Edit Control Points tool, for every drawn reaction.
///
/// They belong to that tool alone. Putting them up on selecting a reaction too
/// would cover the marker with targets at exactly the moment someone is
/// reaching for it, to drag it or just to read it in the inspector.
///
/// Per reaction: the centre, to move it; one lever to turn its backbone; and
/// per branch the two Bézier handles, blue at the metabolite end and red at the
/// reaction end. The points where the branches meet the backbone are not
/// handles: they stay on the backbone and turn with it.
fn render_edit_handles(cx: &Ctx) -> Element {
    let map = cx.map;
    let centre_stroke = if cx.dark { "#f8fafc" } else { "#0f172a" };
    let centre_fill = if cx.dark { "#0f172a" } else { "#ffffff" };
    let blue = "#2563eb";
    let red = "#dc2626";
    let green = "#16a34a";
    let arm = rotation_arm(cx.min_hit_radius);
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-edit-handles");

    let mut branches: HashMap<&str, Vec<&Segment>> = HashMap::new();
    for seg in &map.segments {
        if !seg.node_id.as_deref().is_some_and(|id| !id.is_empty()) {
            continue;
        }
        if matches!(seg.kind, SegmentKind::Axis | SegmentKind::Boundary | SegmentKind::TransportLane) {
            continue;
        }
        branches.entry(seg.reaction_id.as_str()).or_default().push(seg);
    }

    for (reaction_id, rxn) in &map.reactions {
        out.push(
            Element::new("circle")
                .with("class", "emc-reaction-handle")
                .with("data-reaction-handle", reaction_id.clone())
                .with_ws("\n      ", "data-edit-handle-base", "7.5")
                .with_ws("\n      ", "cx", n(rxn.x))
                .with("cy", n(rxn.y))
                .with("r", "7.5")
                .with("fill", centre_fill)
                .with("stroke", centre_stroke)
                .with("stroke-width", "2"),
        );

        // A straight reaction lies along the line between two metabolites, so
        // neither its direction nor the two branches on that line are free.
        let straight = rxn.linear.as_ref().filter(|l| {
            l.from.as_deref().is_some_and(|f| map.nodes.contains_key(f)) && l.to.as_deref().is_some_and(|t| map.nodes.contains_key(t))
        });

        if let Some(axis) = rxn.axis.filter(|_| !rxn.on_membrane && straight.is_none()) {
            // The lever stands across the backbone, where no branch leaves it.
            let ux = -axis.dir.y;
            let uy = axis.dir.x;
            let hx = rxn.x + ux * arm;
            let hy = rxn.y + uy * arm;
            let rotate_attrs = |e: Element| {
                e.with("data-rotate-ox", n(rxn.x))
                    .with("data-rotate-oy", n(rxn.y))
                    .with("data-rotate-ux", js::to_fixed(ux, 5))
                    .with("data-rotate-uy", js::to_fixed(uy, 5))
            };
            out.push(
                rotate_attrs(
                    Element::new("line")
                        .with("class", "emc-control-guide emc-rotate-guide")
                        .with("data-rotate-guide", reaction_id.clone()),
                )
                .with_ws("\n        ", "x1", n(rxn.x))
                .with("y1", n(rxn.y))
                .with("x2", n(hx))
                .with("y2", n(hy))
                .with("stroke", green),
            );
            out.push(
                rotate_attrs(
                    Element::paired("circle")
                        .with("class", "emc-control-handle emc-rotate-handle")
                        .with("data-rotate-handle", reaction_id.clone()),
                )
                .with_ws("\n        ", "data-edit-handle-base", "5.5")
                .with("cx", n(hx))
                .with("cy", n(hy))
                .with("r", "5.5")
                .with_ws("\n        ", "fill", centre_fill)
                .with("stroke", green)
                .with("stroke-width", "1.8")
                .closing("\n        ")
                .child(Element::paired("title").text("Drag to rotate the reaction marker")),
            );
        }

        for seg in branches.get(reaction_id.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
            let node_id = seg.node_id.clone().unwrap_or_default();
            if map.nodes.get(&node_id).is_some_and(|n| n.hidden == Some(true)) {
                continue;
            }
            if straight.is_some_and(|l| Some(node_id.as_str()) == l.from.as_deref() || Some(node_id.as_str()) == l.to.as_deref()) {
                continue;
            }
            let (b1, b2) = branch_controls(seg);
            let (metabolite, reaction_anchor) = if seg.is_substrate() { (seg.from, seg.to) } else { (seg.to, seg.from) };
            let hit_r = "12";

            let guide = |class: &'static str, from: Point, to: Point, stroke: &'static str| {
                Element::new("line")
                    .with("class", class)
                    .with_ws("\n        ", "x1", n(from.x))
                    .with("y1", n(from.y))
                    .with_ws("\n        ", "x2", n(to.x))
                    .with("y2", n(to.y))
                    .with("stroke", stroke)
            };
            let hit = |handle: &'static str, p: Point| {
                Element::new("circle")
                    .with("class", "emc-control-hit")
                    .with_ws("\n        ", "data-control-rxn", reaction_id.clone())
                    .with("data-control-node", node_id.clone())
                    .with_ws("\n        ", "data-control-handle", handle)
                    .with("cx", n(p.x))
                    .with("cy", n(p.y))
                    .with("r", hit_r)
            };
            let handle = |class: &'static str, handle: &'static str, p: Point, stroke: &'static str| {
                Element::new("circle")
                    .with("class", class)
                    .with_ws("\n        ", "data-control-rxn", reaction_id.clone())
                    .with("data-control-node", node_id.clone())
                    .with_ws("\n        ", "data-control-handle", handle)
                    .with("data-edit-handle-base", "5.5")
                    .with_ws("\n        ", "cx", n(p.x))
                    .with("cy", n(p.y))
                    .with("r", "5.5")
                    .with_ws("\n        ", "fill", centre_fill)
                    .with("stroke", stroke)
                    .with("stroke-width", "1.8")
            };
            out.push(guide("emc-control-guide emc-control-guide-metabolite", metabolite, b1, blue));
            out.push(hit("b1", b1));
            out.push(handle("emc-control-handle emc-control-b1", "b1", b1, blue));
            out.push(guide("emc-control-guide emc-control-guide-reaction", reaction_anchor, b2, red));
            out.push(hit("b2", b2));
            out.push(handle("emc-control-handle emc-control-b2", "b2", b2, red));
        }
    }
    out
}

// Map annotations

fn render_map_annotations(cx: &Ctx) -> Option<Element> {
    let map = cx.map;
    if map.texts.is_empty() {
        return None;
    }
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-map-text");
    let halo = if cx.dark { "#0b1220" } else { "#ffffff" };
    let fill = if cx.dark { "#f8fafc" } else { "#111827" };

    for annotation in map.texts.values() {
        // A selected annotation has to read as selected, the way a selected node
        // does, or a rectangle that caught it gives no sign it did.
        let is_multi = cx.multi_has_text(&annotation.id);
        if is_multi {
            let b = text_box_of(annotation);
            out.push(
                Element::new("rect")
                    .with("class", "emc-map-text-selected")
                    .with("x", n(b.left - 4.0))
                    .with("y", n(b.top - 2.0))
                    .with_ws("\n        ", "width", n(b.right - b.left + 8.0))
                    .with("height", n(b.bottom - b.top + 6.0))
                    .with("rx", "4"),
            );
        }
        out.push(map_text(annotation, is_multi, halo, fill));
    }
    Some(out)
}

fn map_text(annotation: &MapText, is_multi: bool, halo: &'static str, fill: &'static str) -> Element {
    let mut text = Element::paired("text")
        .with("class", if is_multi { "emc-map-text is-multi" } else { "emc-map-text" })
        .with("data-map-text", annotation.id.clone());
    // A chosen font goes in the element's own style: the stylesheet sets the
    // default font on .emc-map-text, and would win over a font-family attribute.
    if let Some(font) = annotation.font_family.as_deref().filter(|f| !f.is_empty() && *f != DEFAULT_TEXT_FONT) {
        text.attr("style", format!("font-family: {}", text_font_stack(font)));
    }
    text.with_ws("\n      ", "x", n(annotation.x))
        .with("y", n(annotation.y))
        .with_ws("\n      ", "font-size", n(annotation.font_size.unwrap_or(22.0)))
        .with_ws("\n      ", "text-anchor", "start")
        .with("dominant-baseline", "alphabetic")
        .with_ws("\n      ", "stroke", halo)
        .with("stroke-width", "3.5")
        .with("stroke-linejoin", "round")
        .with("paint-order", "stroke")
        .with_ws("\n      ", "fill", fill)
        .text(annotation.text.clone())
}

// Text

/// Metabolite nodes drawn on at least one reaction that carries flux.
fn active_node_ids<'a>(cx: &Ctx<'a>) -> HashSet<&'a str> {
    let mut ids = HashSet::new();
    for seg in &cx.map.segments {
        let Some(node_id) = seg.node_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        if ids.contains(node_id) {
            continue;
        }
        let Some(rxn) = cx.map.reactions.get(&seg.reaction_id) else { continue };
        if cx.disabled(&seg.reaction_id) || cx.inactive(rxn.cell_id.as_deref()) {
            continue;
        }
        if cx.solved(&seg.reaction_id).abs() > 1e-9 {
            ids.insert(node_id);
        }
    }
    ids
}

fn render_text(cx: &Ctx) -> Element {
    let s = cx.s;
    let map = cx.map;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-text");
    let halo = if cx.dark { "#0b1220" } else { "#ffffff" };

    // "None" means none: metabolite names go too, not only reaction labels.
    // "All active" names what carries flux: a metabolite is named when one of
    // the reactions drawn to it is active, as a reaction is named when it is.
    let active_nodes = (s.labels == LabelMode::Active && !s.fva_enabled).then(|| active_node_ids(cx));
    if s.labels != LabelMode::None {
        for node in map.nodes.values() {
            if node.hidden == Some(true) || is_link(node) {
                continue;
            }
            if active_nodes.as_ref().is_some_and(|ids| !ids.contains(node.id.as_str())) {
                continue;
            }
            let ghost_class = if cx.inactive(node.cell_id.as_deref()) { " is-inactive-species" } else { "" };
            if is_secondary_node(node) && (!s.show_currency || !s.currency_labels) {
                continue;
            }
            let text = s.metabolite_labels.get(&node.id).cloned();
            if node.kind == NodeKind::Currency {
                out.push(metabolite_label(node, &format!("emc-label-currency{ghost_class}"), halo, cx.dark, text));
            } else {
                let role = if node.primary == Some(false) { " emc-label-secondary" } else { " emc-label-primary" };
                let kind = if node.kind == NodeKind::Medium { "emc-label-medium" } else { "emc-label-met" };
                out.push(metabolite_label(node, &format!("{kind}{role}{ghost_class}"), halo, cx.dark, text));
            }
        }
    }

    if s.labels != LabelMode::None || s.fva_enabled {
        let mut nodes_of: Option<HashMap<&str, Vec<&Node>>> = None;
        for (rid, rxn) in &map.reactions {
            let v = cx.flux(rid);
            let disabled = cx.disabled(rid);
            let inactive = cx.inactive(rxn.cell_id.as_deref());
            let solved = if disabled || inactive { 0.0 } else { cx.solved(rid) };
            if !s.fva_enabled && !disabled {
                if s.labels == LabelMode::Active && solved.abs() < 1e-9 {
                    continue;
                }
                // `auto` keeps the overview readable by naming only the reactions that
                // actually carry the map: everything else appears on hover or on zoom.
                if s.labels == LabelMode::Auto
                    && v.abs() < cx.scale.reference() * 0.18
                    && !(s.emphasized_reaction_ids.contains(rid) && solved.abs() > 1e-9)
                {
                    continue;
                }
            }

            // Place the ID and its value beside the marker, leaving a gap even
            // for rotated transporters with several lanes.
            let angle = rxn.axis.map_or(0.0, |a| js::atan2(a.dir.y, a.dir.x));
            let half_width = if rxn.on_membrane { 13.0 } else { 3.0 };
            let lanes = rxn.transport_lanes.as_ref().map_or(0, Vec::len);
            let half_height = if rxn.on_membrane {
                if lanes >= 2 { (14.0 + lanes as f64 * 10.0) / 2.0 } else { 11.0 }
            } else {
                9.0
            };
            let (cos, sin) = (js::cos(angle), js::sin(angle));
            let marker_right = half_width * cos.abs() + half_height * sin.abs();
            let mut tx = rxn.x + marker_right + 8.0;
            let mut ty = rxn.y;
            let mut placement = "emc-label-rxn-adjacent";
            // A reaction kept straight and lying on its side has its own arm where
            // the name would go, so there the name stands over the bar, on the side
            // with fewer of its metabolites.
            if rxn.linear.is_some() && rxn.axis.is_some() && !rxn.on_membrane && sin.abs() < 0.5 {
                let marker_top = half_width * sin.abs() + half_height * cos.abs();
                let nodes = nodes_of.get_or_insert_with(|| reaction_nodes(map));
                let below = emptier_side(nodes.get(rid.as_str()).map(Vec::as_slice).unwrap_or(&[]), rxn) > 0.0;
                tx = rxn.x;
                ty = rxn.y + (marker_top + 5.0) * if below { 1.0 } else { -1.0 };
                placement = if below { "emc-label-rxn-below" } else { "emc-label-rxn-above" };
            }
            let local_id = js_trim(str_opt(&rxn.local_id));
            let cell_prefix = rxn.cell_id.as_deref().filter(|c| !c.is_empty()).map(|c| format!("{c}@")).unwrap_or_default();
            let reaction_id = match s.reaction_labels.get(rid) {
                Some(label) => label.clone(),
                None if !local_id.is_empty() => local_id.to_string(),
                None if !cell_prefix.is_empty() && rid.starts_with(&cell_prefix) => {
                    let rest = &rid[cell_prefix.len()..];
                    if rest.is_empty() { rid.clone() } else { rest.to_string() }
                }
                None => rid.clone(),
            };

            let text = if s.fva_enabled {
                match s.fva_ranges.get(rid).filter(|r| r.min.is_finite() && r.max.is_finite()) {
                    Some(range) => format!(
                        "{reaction_id}  [{}, {}]",
                        format_flux(Some(range.min), 3),
                        format_flux(Some(range.max), 3)
                    ),
                    None => format!("{reaction_id}  [calculando…]"),
                }
            } else {
                format!("{reaction_id}{}  {}", if disabled { " [KO]" } else { "" }, format_flux(Some(v), 3))
            };
            let off = !s.fva_enabled && cx.is_off(rid, inactive);
            let cls = format!(
                "emc-label-rxn{}{}{}{} {placement}",
                if disabled { " is-disabled-reaction" } else { "" },
                if off { " is-off" } else { "" },
                if inactive { " is-inactive-species" } else { "" },
                if s.fva_enabled { " emc-label-fva" } else { "" }
            );
            let anchor = if placement == "emc-label-rxn-adjacent" { "start" } else { "middle" };
            let mut el = label(tx, ty, text, &cls, halo, cx.dark, Some(rid), anchor);
            if inactive {
                el.attr("opacity", "0.28");
            }
            out.push(el);
        }
    }
    out
}

/// The metabolite nodes each reaction reaches.
fn reaction_nodes(map: &MapGraph) -> HashMap<&str, Vec<&Node>> {
    let mut by_reaction: HashMap<&str, Vec<&Node>> = HashMap::new();
    for seg in &map.segments {
        let Some(node) = seg.node_id.as_deref().filter(|id| !id.is_empty()).and_then(|id| map.nodes.get(id)) else {
            continue;
        };
        if seg.reaction_id.is_empty() {
            continue;
        }
        by_reaction.entry(seg.reaction_id.as_str()).or_default().push(node);
    }
    by_reaction
}

/// Above (-1) or below (+1) a reaction lying on its side: where fewer of its metabolites stand.
fn emptier_side(nodes: &[&Node], rxn: &MapReaction) -> f64 {
    let mut above = 0;
    let mut below = 0;
    for node in nodes {
        if node.y < rxn.y - 4.0 {
            above += 1;
        } else if node.y > rxn.y + 4.0 {
            below += 1;
        }
    }
    if below < above { 1.0 } else { -1.0 }
}

fn metabolite_label(node: &Node, cls: &str, halo: &'static str, dark: bool, text: Option<String>) -> Element {
    let text = text.unwrap_or_else(|| str_opt(&node.base).to_string());
    if !node.is_metabolite_kind() {
        let gap = if node.primary == Some(false) { 3.5 } else { 5.0 };
        return label(node.x, node.y - node.r - gap, str_opt(&node.base).to_string(), cls, halo, dark, None, "middle");
    }
    let p = metabolite_label_position(node);
    let mut el = label(p.x, p.y, text, &format!("{cls} emc-label-draggable"), halo, dark, None, p.anchor.as_str());
    el.attr("data-metabolite-label", node.id.clone());
    el.attr("dominant-baseline", p.baseline.as_str());
    if cls.contains("is-inactive-species") {
        el.attr("opacity", "0.28");
    }
    el
}

#[allow(clippy::too_many_arguments)]
fn label(x: f64, y: f64, text: String, cls: &str, halo: &'static str, dark: bool, rid: Option<&str>, anchor: &'static str) -> Element {
    let mut el = Element::paired("text")
        .with("class", format!("emc-label {cls}"))
        .with("x", n(x))
        .with("y", n(y))
        .with("text-anchor", anchor)
        .with("stroke", halo)
        .with("stroke-width", "3.2")
        .with("stroke-linejoin", "round")
        .with("paint-order", "stroke")
        .with("fill", if dark { "#e2e8f0" } else { "#1e293b" });
    if let Some(rid) = rid.filter(|r| !r.is_empty()) {
        el.attr("data-rxn", rid.to_string());
    }
    el.text(text)
}

// Cross-feeding

fn link_key(from: &str, base: &str, to: &str) -> String {
    format!("{from}|{base}|{to}")
}

/// One line per metabolite a species passes to another, from the producer's
/// medium node to the consumer's, in the producer's colour, ending in an arrow.
/// Protons and water get none. Returns the arcs for links that could not be
/// routed (drawn behind the cells), the routed lines, and the name tag of the
/// picked line.
fn render_cross_feeding(cx: &Ctx, links: &[CrossFeedingLink]) -> (Element, Element, Option<Element>) {
    let map = cx.map;
    let mut out = Element::paired("g").with("class", "emc-layer emc-layer-crossfeed");
    let mut routed_out = Element::paired("g").with("class", "emc-layer emc-layer-crossfeed emc-layer-crossfeed-routed");
    let cell_by_id: HashMap<&str, &MapCell> = map.cells.iter().map(|c| (c.id.as_str(), c)).collect();

    let explorer = cx.explorer.as_ref();
    let filtered: Option<HashSet<String>> = explorer
        .filter(|e| e.enabled)
        .map(|e| e.visible_events.iter().map(|ev| link_key(&ev.from, &ev.base, &ev.to)).collect());
    let selected = explorer.and_then(|e| e.selected);
    let is_picked = |link: &CrossFeedingLink| {
        selected.is_some_and(|sel| {
            link.from.as_deref() == Some(sel.from.as_str()) && link.to.as_deref() == Some(sel.to.as_str()) && sel.base == link.base
        })
    };
    // only the metabolites chosen to be shown (and the picked line); the routes
    // are worked out for those alone, so fewer lines also run straighter
    let bases = cx.s.cross_feeding_bases.as_ref();
    let links: Vec<CrossFeedingLink> = links
        .iter()
        .filter(|l| {
            l.from.as_deref().is_some_and(|f| !f.is_empty())
                && l.to.as_deref().is_some_and(|t| !t.is_empty())
                && is_drawn_cross_feeding_base(&l.base)
                && (bases.is_none_or(|b| b.contains(&l.base)) || is_picked(l))
        })
        .cloned()
        .collect();
    let routes = route_cross_feeding(map, &links);
    let grid = route_grid(map);
    // Thinnest first, so a thick line never hides a thin one that shares its
    // lane; the picked line last, over all of them.
    let mut ordered: Vec<&CrossFeedingLink> = links.iter().collect();
    ordered.sort_by(|p, q| {
        let picked = (is_picked(p) as i32) - (is_picked(q) as i32);
        if picked != 0 {
            return picked.cmp(&0);
        }
        (p.flux - q.flux).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut tag = None;
    for link in ordered {
        let from = link.from.as_deref().unwrap_or("");
        let to = link.to.as_deref().unwrap_or("");
        let key = link_key(from, &link.base, to);
        if filtered.as_ref().is_some_and(|f| !f.contains(&key)) {
            continue;
        }
        let (Some(a), Some(b)) = (cell_by_id.get(from), cell_by_id.get(to)) else { continue };
        let is_selected = is_picked(link);
        if let Some(route) = routes.get(&key) {
            // lanes are 0.6 grid apart: the widest line leaves a gap to the next
            let widest = grid * 0.27;
            let base = js::min(widest, js::max(grid * 0.08, cx.scale.width(link.flux) * 1.4));
            let w = if is_selected { js::min(widest * 1.3, base * 1.6) } else { base };
            let opacity = if is_selected {
                1.0
            } else if explorer.is_some_and(|e| e.enabled) && selected.is_some() {
                0.3
            } else {
                0.8
            };
            let pts = &route.points;
            // stop short of the node so the arrow tip touches its rim
            let end = &route.end;
            let tip = Point::new(end.x + end.nx * (end.r + 2.0), end.y + end.ny * (end.r + 2.0));
            let head = js::max(w * 2.4, grid * 0.38);
            let neck = Point::new(tip.x + end.nx * head, tip.y + end.ny * head);
            let start = &route.start;
            let line_from = Point::new(start.x + start.nx * start.r, start.y + start.ny * start.r);
            let mut line = vec![line_from];
            if pts.len() > 2 {
                line.extend_from_slice(&pts[1..pts.len() - 1]);
            }
            line.push(neck);
            let d = rounded_path(&line, grid * 2.2);
            let name = route.start.name.as_deref().or(route.end.name.as_deref());
            let what = cross_feeding_text(link, name);
            // --w lets the stylesheet thicken the line under the pointer; the wide
            // transparent stroke is what the pointer finds, so a thin line is not a
            // one-pixel target.
            let g = Element::paired("g")
                .with("class", if is_selected { "emc-crossfeed-route is-picked" } else { "emc-crossfeed-route" })
                .with("opacity", n(opacity))
                .with("style", format!("--w: {}", n(w)))
                .with("data-crossfeed-route", "")
                .with("data-crossfeed", link.base.clone())
                .with("data-crossfeed-from", from.to_string())
                .with("data-crossfeed-to", to.to_string())
                .child(
                    Element::paired("title")
                        .text(format!("{what} mmol·gDW⁻¹·h⁻¹ of the whole community, split in proportion")),
                )
                .child(
                    Element::new("path")
                        .with("class", "emc-crossfeed")
                        .with("d", d.clone())
                        .with("fill", "none")
                        .with("stroke", a.color.clone())
                        .with("stroke-width", n(w))
                        .with("stroke-linecap", "round")
                        .with("stroke-linejoin", "round"),
                )
                .child(
                    Element::new("polygon")
                        .with("class", "emc-crossfeed-arrow")
                        .with("points", arrow_head(tip, end, head, js::max(w * 1.9, grid * 0.3)))
                        .with("fill", a.color.clone()),
                )
                .child(
                    Element::new("path")
                        .with("class", "emc-crossfeed-hit")
                        .with("d", d)
                        .with("fill", "none")
                        .with("stroke", "transparent")
                        .with("stroke-width", n(grid * 0.55))
                        .with("pointer-events", "stroke"),
                );
            routed_out.push(g);
            if is_selected {
                let at = route_anchor(&line, explorer.and_then(|e| e.selected_at));
                tag = Some(cross_feeding_tag(at, &what, &a.color, cx.dark));
            }
            continue;
        }
        let ca = Point::new(a.rect.x + a.rect.w / 2.0, a.rect.y + a.rect.h / 2.0);
        let cb = Point::new(b.rect.x + b.rect.w / 2.0, b.rect.y + b.rect.h / 2.0);
        let p1 = closest_perimeter_point(&a.rect, cb, 0.0);
        let p2 = closest_perimeter_point(&b.rect, ca, 0.0);
        let mx = (p1.x + p2.x) / 2.0;
        let my = (p1.y + p2.y) / 2.0;
        let dx = p2.x - p1.x;
        let dy = p2.y - p1.y;
        let len = {
            let l = js::hypot(dx, dy);
            if l == 0.0 || l.is_nan() { 1.0 } else { l }
        };
        let bow = 0.16 * len;
        let qx = mx - (dy / len) * bow;
        let qy = my + (dx / len) * bow;
        let w = js::max(if is_selected { 5.0 } else { 2.0 }, cx.scale.width(link.flux) * if is_selected { 1.5 } else { 0.8 });
        let opacity = if is_selected {
            "0.9"
        } else if explorer.is_some_and(|e| e.enabled) {
            "0.5"
        } else {
            "0.22"
        };
        out.push(
            Element::new("path")
                .with("class", "emc-crossfeed")
                .with("d", format!("M {} {} Q {} {}, {} {}", n(p1.x), n(p1.y), n(qx), n(qy), n(p2.x), n(p2.y)))
                .with("fill", "none")
                .with("stroke", a.color.clone())
                .with("stroke-width", n(w))
                .with("stroke-linecap", "round")
                .with("opacity", opacity)
                .with("data-crossfeed", link.base.clone())
                .with("data-crossfeed-from", from.to_string())
                .with("data-crossfeed-to", to.to_string()),
        );
    }
    (out, routed_out, tag)
}

/// What a cross-feeding line carries: `Acetate (ac) · St → Fp · 0.611`.
fn cross_feeding_text(link: &CrossFeedingLink, name: Option<&str>) -> String {
    let what = match name.filter(|n| !n.is_empty() && *n != link.base) {
        Some(name) => format!("{name} ({})", link.base),
        None => link.base.clone(),
    };
    format!(
        "{what} · {} → {} · {}",
        link.from.as_deref().unwrap_or("null"),
        link.to.as_deref().unwrap_or("null"),
        format_flux(Some(link.flux), 3)
    )
}

/// The name tag of the picked line, pinned where it was clicked. It keeps one
/// size on screen at any zoom, like the cells' name plates: the group's origin
/// is the pin, and the stylesheet scales it about that corner.
fn cross_feeding_tag(at: Point, text: &str, color: &str, dark: bool) -> Element {
    let width = js::round(utf16_len(text) as f64 * 7.4 + 22.0);
    let bg = if dark { "#0b1220" } else { "#ffffff" };
    let fg = if dark { "#f8fafc" } else { "#111827" };
    Element::paired("g")
        .with("class", "emc-layer emc-layer-crossfeed-tag")
        .with("transform", format!("translate({} {})", n(at.x), n(at.y)))
        .child(
            Element::paired("g")
                .with("class", "emc-frame-label emc-crossfeed-tag")
                .child(Element::new("circle").with("r", "4.5").with("fill", color.to_string()).with("stroke", bg).with("stroke-width", "1.5"))
                .child(
                    Element::new("rect")
                        .with("x", "10")
                        .with("y", "-34")
                        .with("width", js::number_to_string(width))
                        .with("height", "26")
                        .with("rx", "7")
                        .with("fill", bg)
                        .with("fill-opacity", "0.94")
                        .with("stroke", color.to_string())
                        .with("stroke-width", "2"),
                )
                .child(
                    Element::paired("text")
                        .with("x", "21")
                        .with("y", "-16.5")
                        .with("font-size", "13")
                        .with("font-weight", "700")
                        .with("fill", fg)
                        .with("textLength", js::number_to_string(width - 22.0))
                        .with("lengthAdjust", "spacingAndGlyphs")
                        .text(text.to_string()),
                ),
        )
}

/// Triangle with its tip at `tip`, pointing into the node along the end's normal.
fn arrow_head(tip: Point, end: &Endpoint, length: f64, width: f64) -> String {
    let bx = tip.x + end.nx * length;
    let by = tip.y + end.ny * length;
    let px = -end.ny * width / 2.0;
    let py = end.nx * width / 2.0;
    format!("{},{} {},{} {},{}", n(tip.x), n(tip.y), n(bx + px), n(by + py), n(bx - px), n(by - py))
}

/// `esc(s)`, for callers that build markup of their own.
pub fn escape(s: &str) -> String {
    esc(s)
}
