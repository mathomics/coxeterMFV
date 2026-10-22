//! Replays the edit scripts `rust/tools/fixtures-map.mjs` recorded, op by op,
//! with the arguments the JavaScript was given, and summarises each result
//! the way the script did.

use coxeter_core::community::CommunityModel;
use coxeter_core::mapdata::{Point, Rect};
use coxeter_map::EditableMap;
use coxeter_map::editable_map::{CellSpec, ColorChange, CompartmentSpec, DuplicateOptions, TextOptions};
use coxeter_core::reaction_label::{normalize_reaction_label_offset, reaction_label_placement_in};
use coxeter_map::multi_selection::{Selection, SelectionRect, SelectionTurn, pointing_angle, spacing_items};
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_map::reaction_edit::Handle;
use serde_json::{Map, Value, json};

fn s(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).map(String::from)
}
fn f(a: &Value, k: &str) -> f64 {
    a.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN)
}
fn of(a: &Value, k: &str) -> Option<f64> {
    a.get(k).and_then(Value::as_f64)
}
fn point(v: &Value) -> Option<Point> {
    Some(Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?))
}
fn rect(v: &Value) -> Option<Rect> {
    Some(Rect::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?, v.get("w")?.as_f64()?, v.get("h")?.as_f64()?))
}
fn yes<T>(v: Option<T>) -> Value {
    if v.is_some() { Value::Bool(true) } else { Value::Null }
}
fn yes_b(v: bool) -> Value {
    if v { Value::Bool(true) } else { Value::Null }
}
fn list(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}
fn selection(v: &Value) -> Selection {
    Selection {
        nodes: list(v, "nodes").into_iter().collect(),
        reactions: list(v, "reactions").into_iter().collect(),
        texts: list(v, "texts").into_iter().collect(),
        compartments: v
            .get("compartments")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|k| Some((k.get(0)?.as_str()?.to_string(), k.get(1)?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
    }
}
fn turned(r: Option<SelectionTurn>) -> Value {
    match r {
        Some(SelectionTurn::Turned { angle, pivot, nodes, reactions, texts }) => json!({
            "turned": true, "angle": angle, "pivot": { "x": pivot.x, "y": pivot.y },
            "nodes": nodes, "reactions": reactions, "texts": texts,
        }),
        Some(SelectionTurn::Refused(reason)) => json!({ "turned": false, "reason": reason.as_str() }),
        None => Value::Null,
    }
}
fn color(v: &Value, k: &str) -> ColorChange {
    match v.get(k) {
        None => ColorChange::Keep,
        Some(Value::Null) => ColorChange::Reset,
        Some(x) => ColorChange::Set(x.as_str().unwrap_or("").to_string()),
    }
}
fn place_options(v: &Value) -> PlaceOptions {
    PlaceOptions {
        anchor_node_id: s(v, "anchorNodeId"),
        boundary_id: s(v, "boundaryId"),
        anchor: v.get("anchor").and_then(point),
        placement_direction: v.get("placementDirection").and_then(point),
        terminal_metabolite_id: s(v, "terminalMetaboliteId"),
    }
}
fn arranged(r: &coxeter_map::multi_selection::Arranged) -> Value {
    let mut out = Map::new();
    out.insert(
        "moved".into(),
        Value::Array(r.moved.iter().map(|(k, id)| json!({ "kind": k.as_str(), "id": id })).collect()),
    );
    out.insert("count".into(), json!(r.count));
    if let Some(step) = r.step {
        out.insert("step".into(), json!(step));
    }
    Value::Object(out)
}

/// The summary the fixture script wrote for `map.stats()`.
pub fn stats_json(map: &EditableMap) -> Value {
    let s = map.stats();
    json!({
        "cells": s.cells, "compartments": s.compartments, "boundaries": s.boundaries, "reactions": s.reactions,
        "nodes": s.nodes, "metabolites": s.metabolites, "currency": s.currency, "medium": s.medium,
        "texts": s.texts, "segments": s.segments,
    })
}

/// Apply one recorded op. Returns the summary; `map` is replaced when the op
/// hands back a new map.
pub fn apply_edit(map: &mut EditableMap, community: &CommunityModel, op: &str, a: &Value) -> Value {
    let id = s(a, "id").unwrap_or_default();
    match op {
        "addReaction" => match map.add_reaction(community, &id, &place_options(a.get("opts").unwrap_or(&Value::Null))) {
            Some(r) => json!({ "nodes": r.nodes, "segments": r.segments, "anchorNode": r.anchor_node }),
            None => Value::Null,
        },
        "removeReaction" => match map.remove_reaction(&id) {
            Some(r) => json!({ "nodes": r.nodes }),
            None => Value::Null,
        },
        "moveNode" => yes(map.move_node(&id, f(a, "x"), f(a, "y"))),
        "moveReaction" => yes(map.move_reaction(&id, f(a, "x"), f(a, "y"))),
        "rotateReaction" => yes_b(map.rotate_reaction(&id, f(a, "angle"))),
        "setReactionAngle" => yes_b(map.set_reaction_angle(&id, f(a, "angle"))),
        "flipReaction" => yes_b(map.flip_reaction(&id)),
        "mirrorReaction" => yes_b(map.mirror_reaction(&id)),
        "alignReaction" => {
            let ends = a.get("ends").cloned().unwrap_or(Value::Null);
            yes_b(map.align_reaction(&id, &s(a, "direction").unwrap_or_default(), s(&ends, "from").as_deref(), s(&ends, "to").as_deref()))
        }
        "tidyReaction" => yes_b(map.tidy_reaction(&id)),
        "setReactionLinear" => {
            let ends = a.get("ends").cloned().unwrap_or(Value::Null);
            let on = a.get("on").and_then(Value::as_bool).unwrap_or(true);
            yes_b(map.set_reaction_linear(&id, on, s(&ends, "from").as_deref(), s(&ends, "to").as_deref()))
        }
        "setReactionLineEnd" => yes_b(map.set_reaction_line_end(&id, &s(a, "nodeId").unwrap_or_default())),
        "reactionLineEnds" => {
            let (from, to) = map.reaction_line_ends(&id);
            json!({ "from": from, "to": to })
        }
        "swapReactionMetabolites" => yes_b(map.swap_reaction_metabolites(&id, &s(a, "a").unwrap_or_default(), &s(a, "b").unwrap_or_default())),
        "moveBranchControl" => {
            let handle = Handle::parse(&s(a, "handle").unwrap_or_default());
            match handle.and_then(|h| map.move_branch_control(&id, &s(a, "nodeId").unwrap_or_default(), h, f(a, "x"), f(a, "y"))) {
                Some(p) => json!({ "x": p.x, "y": p.y }),
                None => Value::Null,
            }
        }
        "reactionControlPoints" => Value::Array(
            map.reaction_control_points(&id)
                .iter()
                .map(|c| json!({ "reactionId": c.reaction_id, "nodeId": c.node_id, "handle": c.handle.as_str(), "role": c.role, "x": c.x, "y": c.y }))
                .collect(),
        ),
        "canRotateReaction" => json!(map.can_rotate_reaction(&id)),
        "canLinearizeReaction" => json!(map.can_linearize_reaction(&id)),
        "reactionAngle" => json!(map.reaction_angle(&id)),
        "reactionSides" => {
            let (subs, prods) = map.reaction_sides(&id);
            let strip = |l: &[coxeter_map::reaction_edit::SideEntry]| {
                l.iter().map(|e| json!({ "nodeId": e.node_id, "label": e.label, "group": e.group })).collect::<Vec<_>>()
            };
            json!({ "substrates": strip(&subs), "products": strip(&prods) })
        }
        "selectableInRect" => {
            let r = rect(&a["rect"]).unwrap();
            let include = a.get("options").and_then(|o| o.get("includeSecondary")).and_then(Value::as_bool) != Some(false);
            let s = map.selectable_in_rect(SelectionRect::from_xywh(r.x, r.y, r.w, r.h), include);
            json!({ "nodes": s.nodes, "reactions": s.reactions, "texts": s.texts })
        }
        "translateSelection" => match map.translate_selection(&selection(&a["selection"]), f(a, "dx"), f(a, "dy")) {
            Some(m) => json!({ "nodes": m.nodes, "reactions": m.reactions, "texts": m.texts }),
            None => Value::Null,
        },
        "alignSelection" => arranged(&map.align_selection(&selection(&a["selection"]), &s(a, "mode").unwrap_or_default())),
        "distributeSelection" => arranged(&map.distribute_selection(&selection(&a["selection"]), &s(a, "axis").unwrap_or_default())),
        "alignSelectionInLine" => match map.align_selection_in_line(&selection(&a["selection"]), &s(a, "mode").unwrap_or_default()) {
            Some(r) => json!({ "nodes": r.nodes, "reactions": r.reactions, "texts": r.texts, "held": r.held }),
            None => Value::Null,
        },
        "spaceSelectionEvenly" => {
            let r = map.space_selection_evenly(&selection(&a["selection"]));
            json!({
                "moved": r.moved.iter().map(|(k, id)| json!({ "kind": k.as_str(), "id": id })).collect::<Vec<_>>(),
                "count": r.count,
                "axis": r.axis.map(|p| json!({ "x": p.x, "y": p.y })),
                "step": r.step,
            })
        }
        "rotateSelection" => {
            let pivot = a.get("pivot").and_then(point);
            turned(map.rotate_selection(&selection(&a["selection"]), f(a, "angle"), pivot))
        }
        "pointSelection" => turned(map.point_selection(&selection(&a["selection"]), &s(a, "direction").unwrap_or_default())),
        "pointingAngle" => json!(pointing_angle(map, &selection(&a["selection"]), &s(a, "direction").unwrap_or_default())),
        "spacingItems" => Value::Array(
            spacing_items(map, &selection(&a["selection"])).iter().map(|(k, id)| json!({ "kind": k.as_str(), "id": id })).collect(),
        ),
        "setReactionLabelOffset" => match a.get("offset").and_then(normalize_reaction_label_offset) {
            Some(offset) => yes(map.set_reaction_label_offset(&id, offset)),
            None => Value::Null,
        },
        "resetReactionLabel" => yes(map.reset_reaction_label(&id)),
        "reactionLabelPlacement" => match map.reaction(&id) {
            Some(rxn) => {
                let p = reaction_label_placement_in(map.graph(), rxn);
                json!({ "x": p.x, "y": p.y, "anchor": p.anchor.as_str(), "placement": p.placement })
            }
            None => Value::Null,
        },
        "editGeometryRect" => {
            let r = rect(&a["rect"]).unwrap();
            match map.edit_geometry_rect(s(a, "cellId").as_deref(), &s(a, "compartmentId").unwrap_or_default(), r) {
                Some(e) => json!({ "cellId": e.cell_id, "compartmentId": e.compartment_id, "rect": e.rect }),
                None => Value::Null,
            }
        }
        "setCanvasSize" => {
            let mv = a.get("move").cloned().unwrap_or(Value::Null);
            let c = map.set_canvas_size(of(a, "width"), of(a, "height"), of(&mv, "shiftX").unwrap_or(0.0), of(&mv, "shiftY").unwrap_or(0.0));
            json!({ "width": c.width, "height": c.height })
        }
        "addCell" => {
            let sp = &a["spec"];
            let spec = CellSpec {
                species_id: s(sp, "speciesId").unwrap_or_default(),
                outer_compartment_id: s(sp, "outerCompartmentId"),
                rect: sp.get("rect").and_then(rect),
                outer_label: s(sp, "outerLabel"),
                outer_membrane: s(sp, "outerMembrane"),
                cell_name: s(sp, "cellName"),
                organism: s(sp, "organism"),
                note: s(sp, "note"),
                color: s(sp, "color"),
                abundance: of(sp, "abundance"),
            };
            match map.add_cell(&spec) {
                Some(r) => json!({ "cell": r.cell.id, "compartment": r.compartment.id, "boundary": r.boundary.map(|b| b.id) }),
                None => Value::Null,
            }
        }
        "addCompartment" => {
            let sp = &a["spec"];
            let spec = CompartmentSpec {
                species_id: s(sp, "speciesId").unwrap_or_default(),
                compartment_id: s(sp, "compartmentId").unwrap_or_default(),
                parent_id: s(sp, "parentId"),
                rect: sp.get("rect").and_then(rect),
                label: s(sp, "label"),
                membrane: s(sp, "membrane"),
                cell_name: s(sp, "cellName"),
                organism: s(sp, "organism"),
                note: s(sp, "note"),
                color: s(sp, "color"),
                abundance: of(sp, "abundance"),
            };
            match map.add_compartment(&spec) {
                Some(r) => json!({ "cell": r.cell.id, "compartment": r.compartment.id }),
                None => Value::Null,
            }
        }
        "addText" => {
            let o = a.get("opts").cloned().unwrap_or(Value::Null);
            let opts = TextOptions { id: s(&o, "id"), font_size: of(&o, "fontSize"), font_family: s(&o, "fontFamily") };
            match map.add_text(&s(a, "text").unwrap_or_default(), f(a, "x"), f(a, "y"), &opts) {
                Some(t) => serde_json::to_value(t).unwrap(),
                None => Value::Null,
            }
        }
        "updateText" => yes(map.update_text(&id, &s(a, "text").unwrap_or_default())),
        "styleText" => {
            let st = a.get("style").cloned().unwrap_or(Value::Null);
            yes(map.style_text(&id, of(&st, "fontSize"), s(&st, "fontFamily").as_deref()))
        }
        "moveText" => yes(map.move_text(&id, f(a, "x"), f(a, "y"))),
        "removeText" => yes(map.remove_text(&id)),
        "duplicateMetaboliteNode" => {
            let o = a.get("opts").cloned().unwrap_or(Value::Null);
            let opts = DuplicateOptions { id: s(&o, "id"), offset: of(&o, "offset"), x: of(&o, "x"), y: of(&o, "y") };
            json!(map.duplicate_metabolite_node(&id, &opts).map(|n| n.id.clone()))
        }
        "mergeMetaboliteNodes" => match map.merge_metabolite_nodes(&s(a, "source").unwrap_or_default(), &s(a, "target").unwrap_or_default()) {
            Some(r) => json!({ "rewired": r.rewired }),
            None => Value::Null,
        },
        "mergeOverlappingMetabolite" => match map.merge_overlapping_metabolite(&id) {
            Some(r) => json!({ "rewired": r.rewired, "target": r.target.id }),
            None => Value::Null,
        },
        "overlappingMetaboliteTarget" => json!(map.overlapping_metabolite_target(&id).map(|n| n.id.clone())),
        "addMetabolite" => json!(map.add_metabolite(community, &s(a, "mergedId").unwrap_or_default(), f(a, "x"), f(a, "y")).map(|n| n.id.clone())),
        "setNodePrimary" => yes(map.set_node_primary(&id, a.get("primary").and_then(Value::as_bool).unwrap_or(true))),
        "setNodeRadius" => yes(map.set_node_radius(&id, f(a, "radius"))),
        "setNodeHidden" => yes(map.set_node_hidden(&id, a.get("hidden").and_then(Value::as_bool).unwrap_or(true))),
        "showAllHiddenNodes" => json!(map.show_all_hidden_nodes()),
        "setMetaboliteLabelAngle" => yes(map.set_metabolite_label_angle(&id, f(a, "angle"))),
        "setCellColors" => {
            let c = &a["colors"];
            yes(map.set_cell_colors(&s(a, "cellId").unwrap_or_default(), &color(c, "fill"), &color(c, "stroke")))
        }
        "setCompartmentColors" => {
            let c = &a["colors"];
            yes(map.set_compartment_colors(&s(a, "cellId").unwrap_or_default(), &s(a, "compartmentId").unwrap_or_default(), &color(c, "fill"), &color(c, "stroke")))
        }
        "setCompartmentLabelPosition" => {
            match map.set_compartment_label_position(s(a, "cellId").as_deref(), &s(a, "compartmentId").unwrap_or_default(), f(a, "x"), f(a, "y")) {
                Some(p) => json!({ "x": p.x, "y": p.y }),
                None => Value::Null,
            }
        }
        "removeNode" => yes(map.remove_node(&id)),
        "stats" => stats_json(map),
        "boundaries" => Value::Array(map.boundaries().iter().map(|b| b.to_json()).collect()),
        "minimumCanvasSize" => {
            let c = map.minimum_canvas_size(40.0);
            json!({ "width": c.width, "height": c.height })
        }
        "reload" => {
            *map = EditableMap::from_json(&map.to_json()).unwrap();
            Value::Null
        }
        "subsetByReactions" => {
            *map = map.subset_by_reactions(&list(a, "ids"));
            Value::Null
        }
        "cleared" => {
            *map = map.cleared();
            Value::Null
        }
        other => panic!("unknown edit op {other}"),
    }
}
