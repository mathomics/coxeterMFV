//! Port of `test/align-and-text.test.mjs`: Escher-style Align vertical /
//! horizontal for reactions, and the size and font of free map text.
//!
//! The JavaScript tests drive the map through `Store` (`src/ui/store.js`).
//! [`Session`] below holds what those store methods touch — the map, its
//! undo history, the multi-selection and the style of the next text — and
//! each of its methods makes the same `EditableMap` / `MapHistory` calls, with
//! the same arguments, as the store method it is named after. The store
//! builds its map with `EditableMap.fromCommunity(community,
//! layoutEngineOptions(layoutPreset('spacious')))`; so does `ecoli_store()`.
//!
//! Not ported: "the reaction inspector offers Vertical and Horizontal" — every
//! assertion is on the HTML `renderInspector` returns (UI panels).
//! Not ported: "the text inspector offers a size and a font" — every
//! assertion is on the HTML `renderInspector` returns (UI panels).
//!
//! Assertions left out of tests that are ported:
//! - "one selected reaction is enough to align, and the panel offers it": the
//!   two `assert.match` on the HTML of `renderWorkspace`. What is kept is
//!   `store.canArrangeSelection`, a predicate of the store's own over
//!   `selectionSize` and `map.selectionHasLineReactions`, mirrored in
//!   `Session::can_arrange`.
//! - "map text keeps a size and a font through saving and drawing": the three
//!   assertions on the SVG string `renderMap` returns (crate
//!   `coxeter-render`). `styleText(id, {fontSize: ''})` — an empty size
//!   changes nothing — is `style_text(id, None, ..)` here: the typed API has
//!   no empty string to hand over.
//! - "restyling a text is one Undo step and becomes the style of the next
//!   text": `assert.deepEqual(store.view.textStyle, …)`, the store's own
//!   state. The style is still carried to the next text as the store does it.
//! - "a metabolite a membrane stops is reported, and spacing follows where it
//!   landed": `store.status.state` and `store.status.message`, the store's
//!   status line (it is set from `result.held`, which is asserted).

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, Objective, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{MapText, Node, NodeKind, Point};
use coxeter_core::text_style::{DEFAULT_TEXT_FONT, TEXT_SIZE, clamp_text_size, normalize_text_font, text_font_stack};
use coxeter_map::EditableMap;
use coxeter_map::editable_map::TextOptions;
use coxeter_map::history::MapHistory;
use coxeter_map::layout_settings::{DEFAULT_LAYOUT_PRESET, RawLayoutSettings, layout_engine_options, layout_preset};
use coxeter_map::multi_selection::{Arranged, LineAlignment, Selection, empty_selection, selection_size, text_box, text_center};

const PGI: &str = "eco@PGI";
const PFK: &str = "eco@PFK";
const TPI: &str = "eco@TPI";
const G6P: &str = "met:eco@g6p_c";
const F6P: &str = "met:eco@f6p_c";
const FDP: &str = "met:eco@fdp_c";

/// `near(a, b, eps = 1e-6)`.
fn near(a: f64, b: f64) -> bool {
    near_eps(a, b, 1e-6)
}

fn near_eps(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

/// What `map.alignSelectionInLine` or `map.alignSelection` /
/// `map.distributeSelection` gave `store.arrangeMultiSelection`.
enum Outcome {
    /// `{nodes, reactions, texts, held}`, or null.
    Line(Option<LineAlignment>),
    /// `{moved, count}`.
    Plain(Arranged),
}

/// `store.arrangeMultiSelection(action)`'s `{...result, changed}`.
struct Arrangement {
    outcome: Outcome,
    changed: bool,
}

impl Arrangement {
    /// `result.held`.
    fn held(&self) -> &[String] {
        match &self.outcome {
            Outcome::Line(Some(line)) => &line.held,
            _ => &[],
        }
    }
}

/// The part of `Store` these tests use.
struct Session {
    /// `store.map`.
    map: EditableMap,
    /// `store.mapHistory`.
    history: MapHistory,
    /// `store.multiSelection`.
    selection: Selection,
    /// `store.view.textStyle`.
    text_style: (f64, String),
}

/// `ecoliStore()`: the store after `useScenario('ecoli')`.
fn ecoli_store() -> Session {
    // `Store.mediumSpec()`: `{ lower: -Math.abs(Number(limit) || 0), upper: 1000 }`.
    let medium = [("glc__D", 10.0_f64), ("o2", 20.0)]
        .into_iter()
        .map(|(base, limit)| (base.to_string(), MediumLimit { lower: Some(-limit.abs()), upper: Some(1000.0) }))
        .collect();
    let community = CommunityModel::new(CommunitySpec {
        name: Some("E. coli".into()),
        objective: Some(Objective::parse("equal-growth").unwrap()),
        product_target: None,
        species: vec![SpeciesSpec {
            id: Some("eco".into()),
            model: Arc::new(load_model("ecoli_core_mc")),
            abundance: Some(1.0),
            enabled: Some(true),
            color: None,
        }],
        medium,
    })
    .unwrap();
    let layout_settings = layout_preset(Some(DEFAULT_LAYOUT_PRESET));
    let map = EditableMap::from_community(&community, &layout_engine_options(&RawLayoutSettings::from(&layout_settings)));
    let mut history = MapHistory::default();
    history.clear();
    Session { map, history, selection: empty_selection(), text_style: (TEXT_SIZE.default, DEFAULT_TEXT_FONT.to_string()) }
}

impl Session {
    /// `select(store, { reactions, nodes })`.
    fn select(&mut self, reactions: &[&str], nodes: &[&str]) {
        self.selection = empty_selection();
        self.selection.reactions.extend(reactions.iter().map(|id| id.to_string()));
        self.selection.nodes.extend(nodes.iter().map(|id| id.to_string()));
    }

    /// `store.canArrangeSelection(action)`: two objects, or one reaction
    /// (Escher-style line).
    fn can_arrange(&self, action: &str) -> bool {
        if selection_size(&self.selection) >= 2 {
            return true;
        }
        (action == "align-horizontal" || action == "align-vertical") && self.map.selection_has_line_reactions(&self.selection)
    }

    /// `store.arrangeMultiSelection(action)`.
    fn arrange(&mut self, action: &str) -> Option<Arrangement> {
        if !self.can_arrange(action) {
            return None;
        }
        let label = match action {
            "align-horizontal" => "Align horizontally",
            "align-vertical" => "Align vertically",
            "align-left" => "Align left",
            "align-right" => "Align right",
            "align-top" => "Align top",
            "align-bottom" => "Align bottom",
            "distribute-horizontal" => "Distribute horizontally",
            "distribute-vertical" => "Distribute vertically",
            _ => return None,
        };

        let before = self.map.to_json();
        let mode = &action[action.find('-').unwrap() + 1..];
        // With reactions selected, Horizontal and Vertical work as in Escher:
        // each reaction comes with its metabolites and they all end on one
        // line. Transporters stay on their membranes, so a selection with no
        // other reaction is aligned as plain objects, as it always was.
        let as_line = (mode == "horizontal" || mode == "vertical")
            && action.starts_with("align-")
            && self.map.selection_has_line_reactions(&self.selection);
        let outcome = if as_line {
            Outcome::Line(self.map.align_selection_in_line(&self.selection, mode))
        } else if action.starts_with("align-") {
            Outcome::Plain(self.map.align_selection(&self.selection, mode))
        } else {
            Outcome::Plain(self.map.distribute_selection(&self.selection, mode))
        };

        let changed = self.history.record(&before, &self.map.to_json(), label);
        Some(Arrangement { outcome, changed })
    }

    /// `store.undoMap()`.
    fn undo(&mut self) -> bool {
        let Some(entry) = self.history.undo(&self.map.to_json()) else { return false };
        self.map = EditableMap::from_json(&entry.doc).unwrap();
        true
    }

    /// `store.selectedLineEnds(reactionId)`: the metabolites of this reaction
    /// that are selected on the map, one substrate and one product.
    fn selected_line_ends(&self, reaction_id: &str) -> (Option<String>, Option<String>) {
        let picked = &self.selection.nodes;
        if picked.is_empty() {
            return (None, None);
        }
        let (substrates, products) = self.map.reaction_sides(reaction_id);
        let one = |entries: &[coxeter_map::reaction_edit::SideEntry]| {
            let chosen: Vec<&str> = entries.iter().filter(|e| picked.contains(&e.node_id)).map(|e| e.node_id.as_str()).collect();
            (chosen.len() == 1).then(|| chosen[0].to_string())
        };
        (one(&substrates), one(&products))
    }

    /// `store.alignMapReactionAxis(reactionId, axis)`.
    fn align_reaction_axis(&mut self, reaction_id: &str, axis: &str) -> bool {
        if axis != "vertical" && axis != "horizontal" {
            return false;
        }
        // `{ ...map.reactionLineEnds(id), ...this.selectedLineEnds(id) }`.
        let (line_from, line_to) = self.map.reaction_line_ends(reaction_id);
        let (picked_from, picked_to) = self.selected_line_ends(reaction_id);
        let from = picked_from.or(line_from).and_then(|id| self.map.node(&id).cloned());
        let to = picked_to.or(line_to).and_then(|id| self.map.node(&id).cloned());
        let (Some(from), Some(to)) = (from, to) else { return false };
        let direction = if axis == "vertical" {
            if to.y >= from.y { "S" } else { "N" }
        } else if to.x >= from.x {
            "E"
        } else {
            "W"
        };
        // `store.alignMapReaction(reactionId, direction)`.
        let (from, to) = self.selected_line_ends(reaction_id);
        let before = self.map.to_json();
        if !self.map.align_reaction(reaction_id, direction, from.as_deref(), to.as_deref()) {
            return false;
        }
        self.history.record(&before, &self.map.to_json(), &format!("Align reaction {direction}"));
        true
    }

    /// `store.addMapText(text, x, y, style = this.view.textStyle)`.
    fn add_text(&mut self, text: &str, x: f64, y: f64) -> Option<MapText> {
        let before = self.map.to_json();
        let style = TextOptions { id: None, font_size: Some(self.text_style.0), font_family: Some(self.text_style.1.clone()) };
        let annotation = self.map.add_text(text, x, y, &style).cloned()?;
        self.history.record(&before, &self.map.to_json(), "Add text");
        Some(annotation)
    }

    /// `store.styleMapText(id, { fontSize, fontFamily })`.
    fn style_text(&mut self, id: &str, font_size: Option<f64>, font_family: Option<&str>) -> Option<MapText> {
        let before = self.map.to_json();
        let annotation = self.map.style_text(id, font_size, font_family).cloned()?;
        // `store.setTextStyle({ fontSize: annotation.fontSize, fontFamily: annotation.fontFamily })`.
        if let Some(size) = annotation.font_size {
            self.text_style.0 = clamp_text_size(size);
        }
        if let Some(family) = &annotation.font_family {
            self.text_style.1 = normalize_text_font(family).to_string();
        }
        self.history.record(&before, &self.map.to_json(), "Text style");
        Some(annotation)
    }

    /// `store.updateMapText(id, text, { fontSize, fontFamily })`.
    fn update_text(&mut self, id: &str, text: &str, font_size: Option<f64>, font_family: Option<&str>) -> Option<MapText> {
        let before = self.map.to_json();
        self.map.update_text(id, text)?;
        self.map.style_text(id, font_size, font_family);
        self.history.record(&before, &self.map.to_json(), "Edit text");
        self.map.text(id).cloned()
    }

    fn node(&self, id: &str) -> &Node {
        self.map.node(id).unwrap_or_else(|| panic!("no node {id}"))
    }

    /// `(x, y)` of a reaction's marker.
    fn marker(&self, id: &str) -> Point {
        let r = self.map.reaction(id).unwrap_or_else(|| panic!("no reaction {id}"));
        Point::new(r.x, r.y)
    }

    /// `map.reactionLineEnds(id)`, both ends present.
    fn line_ends(&self, id: &str) -> (String, String) {
        let (from, to) = self.map.reaction_line_ends(id);
        (from.unwrap(), to.unwrap())
    }

    /// The first reaction drawn on a membrane.
    fn transporter(&self) -> Option<String> {
        self.map.reactions().values().find(|r| r.on_membrane).map(|r| r.id.clone())
    }

    /// `[...ids].sort((a, b) => y(a) - y(b))`.
    fn sorted_by_y(&self, ids: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        out.sort_by(|a, b| self.node(a).y.partial_cmp(&self.node(b).y).unwrap());
        out
    }
}

// Escher-style line alignment

/// JS: "Align vertical puts selected reactions and their metabolites on one straight line"
#[test]
fn align_vertical_puts_selected_reactions_and_their_metabolites_on_one_straight_line() {
    let mut store = ecoli_store();
    let reactions = [PGI, PFK];
    let line = [G6P, F6P, FDP];
    let order_before = store.sorted_by_y(&line);
    let cofactor = store
        .map
        .nodes()
        .values()
        .find(|n| n.kind == NodeKind::Currency && n.reaction_id.as_deref() == Some(PFK))
        .map(|n| n.id.clone());
    store.select(&reactions, &[]);

    let result = store.arrange("align-vertical");
    assert!(result.is_some_and(|r| r.changed), "the map changed and was recorded");
    let x = store.node(line[0]).x;
    for id in line {
        assert!(near(store.node(id).x, x), "{id} on the line");
    }
    for id in reactions {
        assert!(near(store.marker(id).x, x), "{id} marker on the line");
    }
    let order_after = store.sorted_by_y(&line);
    assert!(order_before.len() == 3);
    let reversed: Vec<&str> = line.iter().rev().copied().collect();
    assert!(
        order_after == line || order_after == reversed,
        "the pathway reads in its own order along the line, not folded back ({})",
        order_after.join(" → ")
    );
    for i in 1..order_after.len() {
        assert!(
            store.node(&order_after[i]).y - store.node(&order_after[i - 1]).y >= 80.0 - 1e-6,
            "neighbours are spread to leave room for a marker between them"
        );
    }
    for id in reactions {
        let (from, to) = store.line_ends(id);
        let mid = (store.node(&from).y + store.node(&to).y) / 2.0;
        assert!(near_eps(store.marker(id).y, mid, 1e-3), "{id} marker half way between its metabolites");
    }
    for id in reactions {
        assert!(store.map.reaction(id).unwrap().linear.is_some(), "{id} is kept straight");
        for seg in store
            .map
            .segments()
            .iter()
            .filter(|s| s.reaction_id == id && s.node_id.as_deref().is_some_and(|n| line.contains(&n)))
        {
            for point in [seg.b1, seg.b2].into_iter().flatten() {
                assert!(near_eps(point.x, x, 1e-3), "{id} branch to {:?} is straight", seg.node_id);
            }
        }
    }
    if let Some(cofactor) = &cofactor {
        let placed = store.node(cofactor);
        let pfk = store.marker(PFK);
        assert!(!near_eps(placed.x, x, 0.5), "cofactors are placed beside the line, not stacked on it");
        assert!(js::hypot(placed.x - pfk.x, placed.y - pfk.y) < 120.0, "and stay by their reaction");
    }

    store.undo();
    assert!(!near(store.node(line[0]).x, x) || !near(store.node(line[2]).x, x), "one Undo puts it back");
}

/// JS: "Align horizontal does the same along a horizontal line"
#[test]
fn align_horizontal_does_the_same_along_a_horizontal_line() {
    let mut store = ecoli_store();
    store.select(&[PGI], &[]);
    store.arrange("align-horizontal");
    let y = store.marker(PGI).y;
    assert!(near(store.node(G6P).y, y));
    assert!(near(store.node(F6P).y, y));
}

/// JS: "one selected reaction is enough to align, and the panel offers it"
#[test]
fn one_selected_reaction_is_enough_to_align_and_the_panel_offers_it() {
    let mut store = ecoli_store();
    store.select(&[PGI], &[]);
    assert!(store.can_arrange("align-vertical"));
    assert!(!store.can_arrange("align-left"), "edge alignment still needs two objects");
}

/// JS: "a selection without reactions keeps the plain alignment of centres"
#[test]
fn a_selection_without_reactions_keeps_the_plain_alignment_of_centres() {
    let mut store = ecoli_store();
    store.select(&[], &[G6P, FDP]);
    let pgi_before = store.map.reaction(PGI).unwrap().clone();
    store.arrange("align-vertical");
    assert!(near(store.node(G6P).x, store.node(FDP).x));
    assert_eq!(store.map.reaction(PGI).unwrap().linear, pgi_before.linear, "no reaction was made straight");
}

/// JS: "transporters stay on their membranes when a selection is aligned"
#[test]
fn transporters_stay_on_their_membranes_when_a_selection_is_aligned() {
    let mut store = ecoli_store();
    let transporter = store.transporter();
    assert!(transporter.is_some(), "the fixture has a transporter");
    let transporter = transporter.unwrap();
    let before = store.marker(&transporter);
    store.select(&[&transporter, PGI], &[]);
    store.arrange("align-vertical");
    let after = store.marker(&transporter);
    assert!(near(after.x, before.x) && near(after.y, before.y));
}

/// JS: "a reaction aligned on its own keeps the way it runs"
#[test]
fn a_reaction_aligned_on_its_own_keeps_the_way_it_runs() {
    let mut store = ecoli_store();
    let (from, to) = store.line_ends(PGI);
    let was_below = store.node(&to).y >= store.node(&from).y;
    store.align_reaction_axis(PGI, "vertical");
    let marker = store.marker(PGI);
    assert!(
        near(store.node(&from).x, marker.x) && near(store.node(&to).x, marker.x),
        "on one vertical line through the marker"
    );
    assert_eq!(store.node(&to).y >= store.node(&from).y, was_below, "the product stays on the side it was");

    store.align_reaction_axis(PGI, "horizontal");
    let marker = store.marker(PGI);
    assert!(near(store.node(&from).y, marker.y) && near(store.node(&to).y, marker.y));
}

// Text size and font

/// JS: "map text keeps a size and a font through saving and drawing"
#[test]
fn map_text_keeps_a_size_and_a_font_through_saving_and_drawing() {
    let mut map = EditableMap::blank(Some(800.0), Some(600.0));
    let text = map
        .add_text("Glycolysis", 100.0, 100.0, &TextOptions { id: None, font_size: Some(30.0), font_family: Some("serif".into()) })
        .cloned()
        .unwrap();
    assert_eq!(text.font_size, Some(30.0));
    assert_eq!(text.font_family.as_deref(), Some("serif"));
    assert_eq!(
        map.add_text("x", 1.0, 1.0, &TextOptions { font_family: Some("comic".into()), ..Default::default() })
            .unwrap()
            .font_family
            .as_deref(),
        Some("sans"),
        "an unknown font falls back"
    );
    map.style_text(&text.id, Some(500.0), None);
    assert_eq!(map.text(&text.id).unwrap().font_size, Some(TEXT_SIZE.max), "sizes are kept within range");
    map.style_text(&text.id, None, Some("mono"));
    assert_eq!(map.text(&text.id).unwrap().font_size, Some(TEXT_SIZE.max), "an empty size changes nothing");
    assert_eq!(map.text(&text.id).unwrap().font_family.as_deref(), Some("mono"));

    let restored = EditableMap::from_json(&map.to_json()).unwrap();
    assert_eq!(restored.text(&text.id).unwrap().font_family.as_deref(), Some("mono"));
    assert!(text_font_stack("serif").contains("Georgia"));
}

/// JS: "restyling a text is one Undo step and becomes the style of the next text"
#[test]
fn restyling_a_text_is_one_undo_step_and_becomes_the_style_of_the_next_text() {
    let mut store = ecoli_store();
    let first = store.add_text("Pathway", 50.0, 50.0).unwrap();
    assert_eq!(first.font_size, Some(TEXT_SIZE.default));
    store.style_text(&first.id, Some(40.0), Some("times"));
    assert_eq!(store.map.text(&first.id).unwrap().font_family.as_deref(), Some("times"));

    let second = store.add_text("Next", 60.0, 60.0).unwrap();
    assert_eq!(second.font_size, Some(40.0), "the next text uses the last size");
    assert_eq!(second.font_family.as_deref(), Some("times"), "and the last font");

    store.undo(); // undoes adding the second text
    store.undo(); // undoes the restyle
    assert_eq!(store.map.text(&first.id).unwrap().font_size, Some(TEXT_SIZE.default));

    store.update_text(&first.id, "Renamed", Some(18.0), Some("courier"));
    let edited = store.map.text(&first.id).unwrap();
    assert_eq!(edited.text, "Renamed");
    assert_eq!(edited.font_family.as_deref(), Some("courier"));
    store.undo();
    assert_eq!(store.map.text(&first.id).unwrap().text, "Pathway", "words and style are one step");
}

/// JS: "a pathway drawn across the line is not folded back on itself"
#[test]
fn a_pathway_drawn_across_the_line_is_not_folded_back_on_itself() {
    let mut store = ecoli_store();
    // Lay g6p, f6p and fdp side by side, f6p a little below g6p: projected onto a
    // vertical line by position alone, g6p would land between f6p and fdp.
    store.map.move_node(FDP, 600.0, 300.0);
    store.map.move_node(G6P, 400.0, 500.0);
    store.map.move_node(F6P, 500.0, 520.0);
    store.select(&[PGI, PFK], &[]);
    store.arrange("align-vertical");
    let y = |id: &str| store.node(id).y;
    let ordered = store.sorted_by_y(&[G6P, F6P, FDP]);
    assert_eq!(ordered[1], F6P, "f6p stays between g6p and fdp");
    for id in [PGI, PFK] {
        let (from, to) = store.line_ends(id);
        let (low, high) = if y(&from) <= y(&to) { (y(&from), y(&to)) } else { (y(&to), y(&from)) };
        let marker = store.marker(id).y;
        assert!(marker > low && marker < high, "{id} sits between its own metabolites");
        for other in [G6P, F6P, FDP].into_iter().filter(|n| *n != from && *n != to) {
            assert!(!(y(other) > low && y(other) < high), "{id}'s line does not run over {other}");
        }
    }
}

// Alignment edge cases

/// JS: "a metabolite a membrane stops is reported, and spacing follows where it landed"
#[test]
fn a_metabolite_a_membrane_stops_is_reported_and_spacing_follows_where_it_landed() {
    let mut store = ecoli_store();
    let cell = &store.map.cells()[0];
    let cytosol = cell.compartments.iter().find(|c| c.id == "c").map(|c| c.rect);
    let periplasm = cell.compartments.iter().find(|c| c.id == "p").map(|c| c.rect);
    let outside = store
        .map
        .nodes()
        .values()
        .find(|n| n.compartment.as_deref() == Some("p") && n.kind == NodeKind::Metabolite && n.primary != Some(false))
        .map(|n| n.id.clone());
    assert!(
        cytosol.is_some() && periplasm.is_some() && outside.is_some(),
        "fixture has a cytosol, a periplasm and a periplasmic metabolite"
    );
    let (cytosol, periplasm, outside) = (cytosol.unwrap(), periplasm.unwrap(), outside.unwrap());
    // Beside the cytosol, inside the periplasm band, at the cytosol's height.
    store.map.move_node(&outside, (periplasm.x + cytosol.x) / 2.0, cytosol.y + cytosol.h / 2.0);
    store.select(&[PGI], &[&outside]);
    let result = store.arrange("align-vertical").unwrap();
    assert!(result.held().contains(&outside), "the periplasmic metabolite could not cross into the cytosol");
    let landed = store.node(&outside);
    assert!(landed.x < cytosol.x, "it stayed outside the cytosol");
}

/// JS: "a cofactor helper is never put on the line"
#[test]
fn a_cofactor_helper_is_never_put_on_the_line() {
    let mut store = ecoli_store();
    let (from, to) = store.line_ends(TPI);
    assert_eq!(store.node(&to).kind, NodeKind::Currency, "TPI runs to its own g3p helper in this map");
    store.select(&[TPI], &[]);
    store.arrange("align-vertical");
    let x = store.marker(TPI).x;
    assert!(near(store.node(&from).x, x), "dhap is on the line");
    assert!(!near_eps(store.node(&to).x, x, 0.5), "the helper is placed beside it");
    assert!(store.map.reaction(TPI).unwrap().linear.is_none(), "no straight line is drawn to a helper");
}

/// JS: "a transporter-only selection is aligned as plain objects, as before"
#[test]
fn a_transporter_only_selection_is_aligned_as_plain_objects_as_before() {
    let mut store = ecoli_store();
    let transporter = store.transporter().unwrap();
    store.select(&[&transporter], &[G6P]);
    assert!(!store.map.selection_has_line_reactions(&store.selection));
    let result = store.arrange("align-vertical").unwrap();
    // `'count' in result && !('held' in result)`.
    assert!(matches!(result.outcome, Outcome::Plain(_)), "the plain centre alignment ran");
    store.select(&[&transporter], &[]);
    assert!(!store.can_arrange("align-vertical"), "one transporter alone offers nothing to align");
}

/// JS: "a selected title joins the line instead of pulling the pathway to it"
#[test]
fn a_selected_title_joins_the_line_instead_of_pulling_the_pathway_to_it() {
    let mut store = ecoli_store();
    let (from, to) = store.line_ends(PGI);
    let anchors = [store.node(&from).x, store.node(&to).x, store.marker(PGI).x];
    let expected = anchors.iter().fold(0.0, |sum, x| sum + x) / anchors.len() as f64;
    let title = store.add_text("Glycolysis", expected - 2000.0, 100.0).unwrap();
    store.select(&[PGI], &[]);
    store.selection.texts.insert(title.id.clone());
    store.arrange("align-vertical");
    assert!(near_eps(store.marker(PGI).x, expected, 1e-6), "the line is where the pathway was");
    assert!(
        near_eps(text_center(store.map.text(&title.id).unwrap()).x, expected, 1e-6),
        "and the title is centred on it"
    );
}

/// JS: "unselected reactions drawn to a moved metabolite are put in order where they stand"
#[test]
fn unselected_reactions_drawn_to_a_moved_metabolite_are_put_in_order_where_they_stand() {
    let mut store = ecoli_store();
    let neighbour = "eco@G6PDH2r";
    // A curve shaped by hand earlier, its handles now far from both its ends.
    store.map.edit_graph(|g| {
        for seg in g.segments.iter_mut().filter(|s| s.reaction_id == neighbour && s.node_id.as_deref() == Some(G6P)) {
            seg.b1 = Some(Point::new(0.0, 0.0));
            seg.b2 = Some(Point::new(5.0, 5.0));
        }
    });
    store.select(&[PGI], &[]);
    store.arrange("align-vertical");
    let rxn = store.marker(neighbour);
    let g6p = store.node(G6P);
    let (left, right) = (js::min(rxn.x, g6p.x) - 90.0, js::max(rxn.x, g6p.x) + 90.0);
    let (top, bottom) = (js::min(rxn.y, g6p.y) - 90.0, js::max(rxn.y, g6p.y) + 90.0);
    for seg in store.map.segments().iter().filter(|s| s.reaction_id == neighbour && s.node_id.as_deref() == Some(G6P)) {
        for p in [seg.b1, seg.b2].into_iter().flatten() {
            assert!(
                p.x >= left && p.x <= right && p.y >= top && p.y <= bottom,
                "{neighbour}'s curve to g6p stays between its marker and g6p"
            );
        }
    }
}

/// JS: "text boxes follow the font they are drawn in"
#[test]
fn text_boxes_follow_the_font_they_are_drawn_in() {
    // `{ text: 'Glycolysis pathway', fontSize: 40, x: 0, y: 0 }`, with a font or without.
    let base = |font_family: Option<&str>| MapText {
        id: String::new(),
        text: "Glycolysis pathway".into(),
        x: 0.0,
        y: 0.0,
        font_size: Some(40.0),
        font_family: font_family.map(String::from),
        extra: serde_json::Map::new(),
    };
    let courier = text_box(&base(Some("courier")));
    let times = text_box(&base(Some("times")));
    let plain = text_box(&base(None));
    assert!(courier.right > plain.right && plain.right > times.right, "a monospace label is wider than a Times one");
}
