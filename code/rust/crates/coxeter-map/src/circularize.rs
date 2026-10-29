//! Lay a cycle of selected reactions out as a circle (`src/map/circularize.js`).
//!
//! Selected reactions close a cycle when their main metabolites join them end
//! to end and back to the start, as the TCA and Calvin cycles do. Metabolites
//! are matched by what they are (`mergedId`), not by which copy is drawn, so a
//! cycle whose metabolites are drawn more than once is still found; the copies
//! the cycle runs through are joined into one where the circle needs it.
//!
//! The circle puts the cycle's metabolites and reactions in turn at equal
//! steps, so it grows with the cycle. Metabolite names are written outside it,
//! reaction names inside, and every selected reaction's curves are tidied, the
//! cycle's own running along the circle. Geometry only: the model is untouched.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind, Point, Segment};
use coxeter_core::metabolite_label::{is_secondary_node, metabolite_label_angle};
use coxeter_core::reaction_label::set_reaction_label_offset;
use indexmap::IndexMap;

use crate::drag::{Placeable, clamp_node_position, node_fits_at, update_connected_segments};
use crate::editable_map::EditableMap;
use crate::multi_selection::Selection;
use crate::reaction_edit::{move_reaction, set_reaction_angle, set_reaction_linear, straighten_linear_reactions, tidy_reaction};

/// The arc between two neighbours on the circle, a metabolite and a reaction:
/// what a hand-drawn TCA cycle of nine steps uses at a radius of 240.
const RING_STEP: f64 = 84.0;
/// The smallest circle, so a short cycle still has room inside for its names.
const RING_MIN_RADIUS: f64 = 140.0;
/// How far outside the circle a second reaction between the same two
/// metabolites stands, and the next one twice as far: clear of the first
/// one's cofactors.
const PARALLEL_OUT: f64 = 90.0;
/// How far out from its metabolite a spoke's marker stands; its other metabolites stand twice as far.
const SPOKE: f64 = 90.0;
/// How far outside the circle a cycle reaction's other main metabolites stand.
const EXTRA_OUT: f64 = 90.0;
/// How much further out each further one of those stands, a metabolite and its name apart.
const EXTRA_STEP: f64 = 44.0;
/// How far inside the circle a reaction's name is written: clear of its inner cofactors.
const NAME_IN: f64 = 48.0;
/// How far out from its marker the name of a reaction outside the circle is written.
const NAME_OUT: f64 = 28.0;
/// Room kept between the circle and its compartment's edge when the circle has to be moved to fit.
const ROOM_PAD: f64 = 30.0;
/// The grid the circle's middle is tried on when it has to be moved: half a metabolite's name apart.
const ROOM_GRID: f64 = 40.0;
/// A selection with more reactions is not searched for a cycle: the search grows fast with their number.
const MAX_REACTIONS: usize = 80;
/// How many steps the cycle search takes at most, about a tenth of a second, before it keeps the longest found.
const SEARCH_BUDGET: i64 = 200_000;

/// `CIRCLE_REFUSAL`: why a selection was not laid out as a circle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircleRefusal {
    /// The selected reactions close no cycle.
    Cycle,
    /// The circle fits nowhere in its compartment.
    Room,
}

impl CircleRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            CircleRefusal::Cycle => "cycle",
            CircleRefusal::Room => "room",
        }
    }
}

/// The longest cycle the selected reactions close ([`selection_cycle`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionCycle {
    /// `reactions[i]` runs between `metabolites[i]` and `metabolites[i + 1]`
    /// (the last back to the first).
    pub reactions: Vec<String>,
    /// `mergedId`s.
    pub metabolites: Vec<String>,
    /// Whether `metabolites[i]` is on the substrate side of `reactions[i]`.
    pub forward: Vec<bool>,
}

/// What [`circularize_selection`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Circularized {
    Circled { reactions: usize, radius: f64, centre: Point },
    Refused(CircleRefusal),
}

/// `selectionCycle(map, selection)`: the longest cycle the selected reactions
/// close, or None. Of two as long, the one drawn rounder now: a cycle already
/// drawn as a ring keeps its ring. Transporters are left out.
pub fn selection_cycle(map: &EditableMap, selection: &Selection) -> Option<SelectionCycle> {
    let mut ids: Vec<&String> = selection.reactions.iter().filter(|id| ringable(map, id)).collect();
    ids.sort_by(|a, b| js::code_unit_compare(a, b));
    if ids.len() < 3 || ids.len() > MAX_REACTIONS {
        return None;
    }

    // Each reaction joins every main substrate to every main product, both ways.
    let mut adjacency: IndexMap<String, Vec<(String, String)>> = IndexMap::new();
    for id in &ids {
        let (substrates, products) = main_ends(map, id);
        for a in substrates.keys() {
            for b in products.keys() {
                if a == b {
                    continue;
                }
                adjacency.entry(a.clone()).or_default().push((b.clone(), (*id).clone()));
                adjacency.entry(b.clone()).or_default().push((a.clone(), (*id).clone()));
            }
        }
    }
    for list in adjacency.values_mut() {
        list.sort_by(|p, q| js::code_unit_compare(&p.0, &q.0).then_with(|| js::code_unit_compare(&p.1, &q.1)));
    }

    // Every simple cycle is met once from its smallest metabolite, so the
    // search from each start goes only through larger ones. It stops after a
    // fixed number of steps with the longest found so far.
    let mut starts: Vec<&String> = adjacency.keys().collect();
    starts.sort_by(|a, b| js::code_unit_compare(a, b));
    let mut search = Search {
        map,
        adjacency: &adjacency,
        budget: SEARCH_BUDGET,
        best: None,
        path: Vec::new(),
        reactions: Vec::new(),
        on_path: HashSet::new(),
        used: HashSet::new(),
    };
    for start in starts {
        if search.budget <= 0 {
            break;
        }
        search.path.push(start.clone());
        search.on_path.insert(start.clone());
        search.walk(start, start);
        search.path.pop();
        search.on_path.remove(start);
    }
    let best = search.best?;

    let mut forward = Vec::new();
    for (i, id) in best.reactions.iter().enumerate() {
        forward.push(main_ends(map, id).0.contains_key(&best.metabolites[i]));
    }
    Some(SelectionCycle { reactions: best.reactions, metabolites: best.metabolites, forward })
}

/// The longest cycle found so far, and how round it is drawn.
struct Found {
    metabolites: Vec<String>,
    reactions: Vec<String>,
    round: f64,
}

/// The state of the cycle search: the path walked so far and what it uses.
struct Search<'a> {
    map: &'a EditableMap,
    adjacency: &'a IndexMap<String, Vec<(String, String)>>,
    budget: i64,
    best: Option<Found>,
    path: Vec<String>,
    reactions: Vec<String>,
    on_path: HashSet<String>,
    used: HashSet<String>,
}

impl Search<'_> {
    fn walk(&mut self, start: &str, at: &str) {
        let adjacency = self.adjacency;
        let Some(list) = adjacency.get(at) else { return };
        for (to, reaction) in list {
            self.budget -= 1;
            if self.budget < 0 {
                return;
            }
            if self.used.contains(reaction) {
                continue;
            }
            if to == start {
                let length = self.reactions.len() + 1;
                let longest = match &self.best {
                    None => true,
                    Some(best) => length >= best.reactions.len(),
                };
                if length >= 3 && longest {
                    let metabolites = self.path.clone();
                    let mut reactions = self.reactions.clone();
                    reactions.push(reaction.clone());
                    let round = roundness(self.map, &metabolites, &reactions);
                    let better = match &self.best {
                        None => true,
                        Some(best) => length > best.reactions.len() || round < best.round - 1e-9,
                    };
                    if better {
                        self.best = Some(Found { metabolites, reactions, round });
                    }
                }
                continue;
            }
            if self.on_path.contains(to) || js::code_unit_compare(to, start) == Ordering::Less {
                continue;
            }
            self.path.push(to.clone());
            self.reactions.push(reaction.clone());
            self.on_path.insert(to.clone());
            self.used.insert(reaction.clone());
            self.walk(start, to);
            self.path.pop();
            self.reactions.pop();
            self.on_path.remove(to);
            self.used.remove(reaction);
        }
    }
}

/// How far a cycle is drawn from a circle now: the spread of its reactions'
/// and metabolites' distances from their middle, against the mean distance.
/// A metabolite drawn more than once counts where its first copy is.
fn roundness(map: &EditableMap, metabolites: &[String], reactions: &[String]) -> f64 {
    let mut points = Vec::new();
    for (i, id) in reactions.iter().enumerate() {
        let rxn = map.reaction(id).expect("a selected reaction is drawn");
        points.push(Point::new(rxn.x, rxn.y));
        let copies = nodes_for(map, id, &metabolites[i]);
        if let Some(node) = copies.first().and_then(|n| map.node(n)) {
            points.push(node.pos());
        }
    }
    let n = points.len() as f64;
    let mut cx = 0.0;
    let mut cy = 0.0;
    for p in &points {
        cx += p.x;
        cy += p.y;
    }
    cx /= n;
    cy /= n;
    let mut mean = 0.0;
    for p in &points {
        mean += js::hypot(p.x - cx, p.y - cy);
    }
    mean /= n;
    if mean < 1e-9 {
        return 0.0;
    }
    let mut variance = 0.0;
    for p in &points {
        let d = js::hypot(p.x - cx, p.y - cy);
        variance += (d - mean) * (d - mean);
    }
    variance /= n;
    variance.sqrt() / mean
}

/// `circularizeSelection(map, selection)`: lay the cycle the selected
/// reactions close out as a circle, where it is drawn now: about its middle
/// ([`ring_middle`]), running the way its metabolites and reactions run round
/// it, each as near its present place as it can be. If that does not fit its
/// compartment, the nearest place that does is used.
pub fn circularize_selection(map: &mut EditableMap, selection: &Selection) -> Circularized {
    let Some(cycle) = selection_cycle(map, selection) else { return Circularized::Refused(CircleRefusal::Cycle) };

    let k = cycle.reactions.len();
    let slots = 2 * k;
    let step = (2.0 * PI) / slots as f64;
    let radius = RING_MIN_RADIUS.max((slots as f64 * RING_STEP) / (2.0 * PI));

    // One drawn node per metabolite of the cycle: the copy most drawn to
    // among those the two reactions beside it reach. The others are joined
    // into it.
    let mut ring_nodes: Vec<String> = Vec::new();
    for i in 0..k {
        let key = &cycle.metabolites[i];
        let before = &cycle.reactions[(i + k - 1) % k];
        let after = &cycle.reactions[i];
        let mut copies: Vec<String> = Vec::new();
        for copy in nodes_for(map, before, key).into_iter().chain(nodes_for(map, after, key)) {
            if !copies.contains(&copy) {
                copies.push(copy);
            }
        }
        copies.sort_by(|a, b| segment_count(map, b).cmp(&segment_count(map, a)).then_with(|| js::code_unit_compare(a, b)));
        ring_nodes.push(copies.swap_remove(0));
    }

    // The circle in order: metabolite, reaction, metabolite, ... and where each is now.
    let mut items: Vec<Point> = Vec::new();
    for i in 0..k {
        items.push(map.node(&ring_nodes[i]).expect("a ring node").pos());
        let rxn = map.reaction(&cycle.reactions[i]).expect("a ring reaction");
        items.push(Point::new(rxn.x, rxn.y));
    }
    let middle = ring_middle(&items, radius);
    let mut angles = Vec::new();
    for p in &items {
        angles.push(js::atan2(p.y - middle.y, p.x - middle.x));
    }
    let mut turning = 0.0;
    for j in 0..slots {
        turning += wrap(angles[(j + 1) % slots] - angles[j]);
    }
    let dir = if turning < 0.0 { -1.0 } else { 1.0 };
    let mut sin = 0.0;
    let mut cos = 0.0;
    for j in 0..slots {
        sin += js::sin(angles[j] - dir * j as f64 * step);
        cos += js::cos(angles[j] - dir * j as f64 * step);
    }
    let offset = if sin.abs() + cos.abs() > 1e-9 { js::atan2(sin, cos) } else { -PI / 2.0 };
    let slot_angle = |j: f64| offset + dir * j * step;

    // Where the circle goes: each metabolite where it may stand, each marker
    // where a small metabolite of its compartment could.
    let fits = |c: Point| {
        for j in 0..slots {
            let at = polar(c, radius, slot_angle(j as f64));
            let node = map.node(&ring_nodes[j / 2]).expect("a ring node");
            let mut stand = Placeable::from(node);
            if j % 2 == 1 {
                stand.r = 10.0;
            }
            if !node_fits_at(map, stand, at.x, at.y, None) {
                return false;
            }
        }
        true
    };
    let first = map.node(&ring_nodes[0]).expect("a ring node").clone();
    let Some(centre) = place_circle(map, &first, middle, radius, &fits) else {
        return Circularized::Refused(CircleRefusal::Room);
    };

    // The other selected reactions. A second reaction between two neighbours
    // of the circle stands outside it, beside the first. One that reaches a
    // single metabolite of the circle stands out from it like a spoke. The
    // rest stay where they are.
    let mut ring_index: HashMap<String, usize> = HashMap::new();
    let mut ring_pairs: HashMap<String, usize> = HashMap::new();
    for i in 0..k {
        ring_index.insert(cycle.metabolites[i].clone(), i);
        ring_pairs.insert(pair_key(&cycle.metabolites[i], &cycle.metabolites[(i + 1) % k]), i);
    }
    let ring_reactions: HashSet<String> = cycle.reactions.iter().cloned().collect();
    let mut parallels: Vec<Beside> = Vec::new();
    let mut spokes: Vec<Beside> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    let mut selected: Vec<&String> = selection.reactions.iter().collect();
    selected.sort_by(|a, b| js::code_unit_compare(a, b));
    for id in selected {
        if ring_reactions.contains(id) || !map.has_reaction(id) {
            continue;
        }
        if !ringable(map, id) {
            others.push(id.clone());
            continue;
        }
        let (substrates, products) = main_ends(map, id);
        if let Some(i) = parallel_step(&substrates, &products, &ring_pairs) {
            let forward = substrates.contains_key(&cycle.metabolites[i]);
            parallels.push(Beside { id: id.clone(), i, forward });
            continue;
        }
        let mut touched: Vec<&String> = Vec::new();
        for key in substrates.keys().chain(products.keys()) {
            if ring_index.contains_key(key) && !touched.contains(&key) {
                touched.push(key);
            }
        }
        if touched.len() == 1 {
            let forward = substrates.contains_key(touched[0]);
            spokes.push(Beside { id: id.clone(), i: ring_index[touched[0]], forward });
        } else {
            others.push(id.clone());
        }
    }

    // Reactions kept straight between two metabolites stop being so: they run along the circle now.
    let mut placed_reactions: Vec<String> = cycle.reactions.clone();
    for p in &parallels {
        placed_reactions.push(p.id.clone());
    }
    for s in &spokes {
        placed_reactions.push(s.id.clone());
    }
    for id in &placed_reactions {
        if map.reaction(id).is_some_and(|r| r.linear.is_some()) {
            set_reaction_linear(map, id, false, None, None);
        }
    }

    // The copies joined into the circle's nodes: those its reactions, the
    // second reactions beside them and the spokes reach.
    for i in 0..k {
        let mut reach = vec![cycle.reactions[(i + k - 1) % k].clone(), cycle.reactions[i].clone()];
        for p in &parallels {
            if p.i == i || (p.i + 1) % k == i {
                reach.push(p.id.clone());
            }
        }
        for s in &spokes {
            if s.i == i {
                reach.push(s.id.clone());
            }
        }
        for id in &reach {
            for copy in nodes_for(map, id, &cycle.metabolites[i]) {
                if copy != ring_nodes[i] {
                    map.merge_metabolite_nodes(&copy, &ring_nodes[i]);
                }
            }
        }
    }

    // The metabolites on the circle, named outside it.
    let ring_set: HashSet<String> = ring_nodes.iter().cloned().collect();
    for (i, node_id) in ring_nodes.iter().enumerate() {
        let angle = slot_angle((2 * i) as f64);
        set_node(map, node_id, polar(centre, radius, angle));
        let node = map.g.nodes.get_mut(node_id).expect("a ring node");
        node.hidden = Some(false);
        node.label_angle = Some(metabolite_label_angle(Some(angle)));
    }

    // Each reaction along the circle, from the metabolite on its substrate
    // side to the one on its product side, named inside the circle, or
    // outside it when it stands outside.
    let place_on_circle = |map: &mut EditableMap, id: &str, slot: f64, forward: bool, out: f64| {
        let angle = slot_angle(slot);
        let at = polar(centre, radius + out, angle);
        move_reaction(map, id, at.x, at.y);
        let travel = angle + dir * PI / 2.0;
        set_reaction_angle(map, id, if forward { travel } else { travel + PI });
        let name = if out > 0.0 { NAME_OUT } else { -NAME_IN };
        let rxn = map.g.reactions.get_mut(id).expect("a placed reaction");
        set_reaction_label_offset(rxn, Some(Point::new(js::cos(angle) * name, js::sin(angle) * name)));
    };
    for i in 0..k {
        place_on_circle(map, &cycle.reactions[i], (2 * i + 1) as f64, cycle.forward[i], 0.0);
    }
    let mut stacked: HashMap<usize, usize> = HashMap::new();
    for p in &parallels {
        let n = *stacked.get(&p.i).unwrap_or(&0);
        stacked.insert(p.i, n + 1);
        place_on_circle(map, &p.id, (2 * p.i + 1) as f64, p.forward, PARALLEL_OUT * (n + 1) as f64);
    }

    // A cycle reaction's other main metabolites stand outside the circle, on
    // the side of the reaction they are on, each further one further out.
    let mut placed: HashSet<String> = ring_set.clone();
    let mut extras: Vec<String> = Vec::new();
    let mut outside = |map: &mut EditableMap, id: &str, slot: f64, out: f64, forward: bool| {
        let (substrates, products) = main_ends(map, id);
        let toward_substrates = if forward { -1.0 } else { 1.0 };
        let sides = [(substrates, toward_substrates), (products, -toward_substrates)];
        for (ends, toward) in sides {
            let mut n = 0.0;
            let angle = slot_angle(slot + toward * 0.5);
            for copies in ends.values() {
                for node_id in copies {
                    if !placed.insert(node_id.clone()) {
                        continue;
                    }
                    let want = polar(centre, radius + out + EXTRA_OUT + EXTRA_STEP * n, angle);
                    let node = map.node(node_id).expect("a drawn node");
                    let at = clamp_node_position(map, Placeable::from(node), want.x, want.y);
                    set_node(map, node_id, at);
                    let node = map.g.nodes.get_mut(node_id).expect("a drawn node");
                    node.label_angle = Some(metabolite_label_angle(Some(angle)));
                    extras.push(node_id.clone());
                    n += 1.0;
                }
            }
        }
    };
    for i in 0..k {
        outside(map, &cycle.reactions[i], (2 * i + 1) as f64, 0.0, cycle.forward[i]);
    }
    stacked.clear();
    for p in &parallels {
        let n = *stacked.get(&p.i).unwrap_or(&0);
        stacked.insert(p.i, n + 1);
        outside(map, &p.id, (2 * p.i + 1) as f64, PARALLEL_OUT * (n + 1) as f64, p.forward);
    }

    // A spoke runs straight out from its metabolite, its other main
    // metabolites further along it, and a second spoke on one metabolite
    // beside the first. A metabolite with a spoke straight out writes its
    // name inside the circle.
    stacked.clear();
    for s in &spokes {
        let n = *stacked.get(&s.i).unwrap_or(&0);
        stacked.insert(s.i, n + 1);
        let mut spread = (n / 2) as f64;
        if n % 2 == 1 {
            spread = -(((n + 1) / 2) as f64);
        }
        let angle = slot_angle((2 * s.i) as f64) + dir * step * 0.5 * spread;
        let at = polar(centre, radius + SPOKE, angle);
        move_reaction(map, &s.id, at.x, at.y);
        set_reaction_angle(map, &s.id, if s.forward { angle } else { angle + PI });
        let rxn = map.g.reactions.get_mut(&s.id).expect("a spoke");
        set_reaction_label_offset(rxn, Some(Point::new(-js::sin(angle) * NAME_OUT, js::cos(angle) * NAME_OUT)));
        let (substrates, products) = main_ends(map, &s.id);
        let mut j = 0.0;
        for copies in substrates.values().chain(products.values()) {
            for node_id in copies {
                if !placed.insert(node_id.clone()) {
                    continue;
                }
                let want = polar(centre, radius + 2.0 * SPOKE + EXTRA_STEP * j, angle);
                let node = map.node(node_id).expect("a drawn node");
                let at = clamp_node_position(map, Placeable::from(node), want.x, want.y);
                set_node(map, node_id, at);
                let node = map.g.nodes.get_mut(node_id).expect("a drawn node");
                node.label_angle = Some(metabolite_label_angle(Some(angle)));
                extras.push(node_id.clone());
                j += 1.0;
            }
        }
        if n == 0 {
            let inward = slot_angle((2 * s.i) as f64) + PI;
            let node = map.g.nodes.get_mut(&ring_nodes[s.i]).expect("a ring node");
            node.label_angle = Some(metabolite_label_angle(Some(inward)));
        }
    }

    // Every selected reaction's curves tidied, the cycle's own along the circle.
    let mut tidied = placed_reactions;
    tidied.extend(others);
    for id in &tidied {
        tidy_reaction(map, id);
    }
    for seg in map.g.segments.iter_mut() {
        let branch = seg.is_substrate() || seg.is_product();
        let on_ring = seg.node_id.as_ref().is_some_and(|n| ring_set.contains(n));
        if branch && on_ring && ring_reactions.contains(&seg.reaction_id) {
            bend_along(seg, centre, radius);
        }
    }
    // Straight reactions drawn to anything that moved follow it.
    let mut moved = ring_nodes.clone();
    moved.extend(extras);
    straighten_linear_reactions(map, Some(&moved));

    Circularized::Circled { reactions: k, radius, centre }
}

/// A selected reaction beside the circle: a second one between two
/// neighbours of it, or a spoke. `i` is the step, or the metabolite, it is at.
struct Beside {
    id: String,
    i: usize,
    forward: bool,
}

/// The middle of a cycle as drawn now. A cycle already drawn round (whole,
/// or as an arc) keeps the middle of its circle, the least-squares one
/// through its points, so what is written in it stays in the middle. A
/// drawing that is not round, whose circle would be far off or too large,
/// uses its centroid.
fn ring_middle(points: &[Point], radius: f64) -> Point {
    let n = points.len() as f64;
    let mut cx = 0.0;
    let mut cy = 0.0;
    for p in points {
        cx += p.x;
        cy += p.y;
    }
    cx /= n;
    cy /= n;
    let mut suu = 0.0;
    let mut svv = 0.0;
    let mut suv = 0.0;
    let mut suz = 0.0;
    let mut svz = 0.0;
    let mut sz = 0.0;
    let mut spread = 0.0;
    for p in points {
        let u = p.x - cx;
        let v = p.y - cy;
        let z = u * u + v * v;
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suz += u * z;
        svz += v * z;
        sz += z;
        spread += z.sqrt();
    }
    spread /= n;
    let det = suu * svv - suv * suv;
    if !(det > 1e-9 * (suu + svv) * (suu + svv)) {
        return Point::new(cx, cy);
    }
    let a = (suz * svv - svz * suv) / (2.0 * det);
    let b = (svz * suu - suz * suv) / (2.0 * det);
    let fitted = (a * a + b * b + sz / n).sqrt();
    if js::hypot(a, b) > spread || fitted > 2.0 * radius {
        return Point::new(cx, cy);
    }
    Point::new(cx + a, cy + b)
}

/// A reaction that can stand on the circle: drawn, with a backbone, and not a transporter.
fn ringable(map: &EditableMap, id: &str) -> bool {
    map.reaction(id).is_some_and(|r| r.axis.is_some() && !r.on_membrane)
}

/// Drawn copies of each metabolite, by `mergedId`, in the order they are drawn.
type Ends = IndexMap<String, Vec<String>>;

/// The main metabolites drawn on each side of a reaction, as `mergedId` →
/// node ids, in the order its branches are drawn. Cofactors are left out.
fn main_ends(map: &EditableMap, id: &str) -> (Ends, Ends) {
    let mut substrates = Ends::new();
    let mut products = Ends::new();
    for seg in map.segments() {
        if seg.reaction_id != id || !(seg.is_substrate() || seg.is_product()) {
            continue;
        }
        let Some(node_id) = seg.node_id.as_deref() else { continue };
        let Some(node) = map.node(node_id) else { continue };
        if is_secondary_node(node) {
            continue;
        }
        let ends = if seg.is_substrate() { &mut substrates } else { &mut products };
        let key = node.merged_id.clone().unwrap_or_else(|| node.id.clone());
        let list = ends.entry(key).or_default();
        if !list.contains(&node.id) {
            list.push(node.id.clone());
        }
    }
    (substrates, products)
}

/// The drawn copies of metabolite `key` a reaction reaches, substrates first.
fn nodes_for(map: &EditableMap, reaction_id: &str, key: &str) -> Vec<String> {
    let (substrates, products) = main_ends(map, reaction_id);
    let mut copies = Vec::new();
    if let Some(list) = substrates.get(key) {
        copies.extend(list.iter().cloned());
    }
    if let Some(list) = products.get(key) {
        copies.extend(list.iter().cloned());
    }
    copies
}

/// How many curves are drawn to a node.
fn segment_count(map: &EditableMap, node_id: &str) -> usize {
    let mut n = 0;
    for seg in map.segments() {
        if seg.node_id.as_deref() == Some(node_id) {
            n += 1;
        }
    }
    n
}

/// One key for two metabolites, whichever way round they are given.
fn pair_key(a: &str, b: &str) -> String {
    if js::code_unit_compare(a, b) == Ordering::Less {
        return format!("{a}\u{0}{b}");
    }
    format!("{b}\u{0}{a}")
}

/// The step of the circle a reaction runs alongside (both its ends are that step's metabolites).
fn parallel_step(substrates: &Ends, products: &Ends, ring_pairs: &HashMap<String, usize>) -> Option<usize> {
    for a in substrates.keys() {
        for b in products.keys() {
            if let Some(&i) = ring_pairs.get(&pair_key(a, b)) {
                return Some(i);
            }
        }
    }
    None
}

/// Where the circle goes: about `middle` if everything on it fits there,
/// else the nearest point of a grid over its compartment where it does, the
/// circle kept inside the compartment. None when there is none.
fn place_circle(map: &EditableMap, node: &Node, middle: Point, radius: f64, fits: &dyn Fn(Point) -> bool) -> Option<Point> {
    if fits(middle) {
        return Some(middle);
    }
    let rect = if node.kind == NodeKind::Medium {
        map.environment().rect
    } else {
        let cell = map.cell(node.cell_id.as_deref()?)?;
        let compartment = cell.compartments.iter().find(|c| Some(c.id.as_str()) == node.compartment.as_deref())?;
        compartment.rect
    };
    let reach = radius + ROOM_PAD;
    let mut candidates: Vec<(f64, f64, f64)> = Vec::new();
    let mut y = rect.y + reach;
    while y <= rect.y + rect.h - reach + 1e-9 {
        let mut x = rect.x + reach;
        while x <= rect.x + rect.w - reach + 1e-9 {
            candidates.push((x, y, js::hypot(x - middle.x, y - middle.y)));
            x += ROOM_GRID;
        }
        y += ROOM_GRID;
    }
    candidates.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.total_cmp(&b.1)).then(a.0.total_cmp(&b.0)));
    for (x, y, _) in candidates {
        if fits(Point::new(x, y)) {
            return Some(Point::new(x, y));
        }
    }
    None
}

/// Give a branch to a metabolite on the circle the shape of the circle's arc between its two ends.
fn bend_along(seg: &mut Segment, centre: Point, radius: f64) {
    let (node, anchor) = if seg.is_substrate() { (seg.from, seg.to) } else { (seg.to, seg.from) };
    let a = js::atan2(node.y - centre.y, node.x - centre.x);
    let b = js::atan2(anchor.y - centre.y, anchor.x - centre.x);
    let sweep = wrap(b - a);
    // The handle length that makes a cubic Bézier follow a circular arc.
    let k = (4.0 / 3.0) * js::tan(sweep / 4.0) * radius;
    seg.b1 = Some(Point::new(node.x - js::sin(a) * k, node.y + js::cos(a) * k));
    seg.b2 = Some(Point::new(anchor.x + js::sin(b) * k, anchor.y - js::cos(b) * k));
    seg.extra.shift_remove("control");
}

/// Put a node at a point and bring the curves drawn to it along.
fn set_node(map: &mut EditableMap, node_id: &str, point: Point) {
    let node = map.g.nodes.get_mut(node_id).expect("a drawn node");
    let delta = (point.x - node.x, point.y - node.y);
    node.x = point.x;
    node.y = point.y;
    update_connected_segments(map, node_id, Some(delta));
}

/// The point `r` from `c` in direction `angle`.
fn polar(c: Point, r: f64, angle: f64) -> Point {
    Point::new(c.x + r * js::cos(angle), c.y + r * js::sin(angle))
}

/// An angle in (-π, π].
fn wrap(angle: f64) -> f64 {
    let mut a = angle % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    }
    if a <= -PI {
        a += 2.0 * PI;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
    use coxeter_core::model::Model;
    use std::sync::Arc;

    use crate::LayoutOptions;
    use crate::multi_selection::empty_selection;

    const TCA: [&str; 8] = ["CS", "ACONT", "ICDHyr", "AKGDH", "SUCOAS", "SUCDi", "FUM", "MDH"];

    /// The E. coli core model, laid out in full: its TCA cycle, with FRD7 running beside SUCDi.
    fn ecoli_map() -> EditableMap {
        let path = coxeter_core::testkit::repo_root().join("models/ecoli_core_mc.json");
        let model = Model::from_json_str(&std::fs::read_to_string(path).expect("model file")).expect("model");
        let spec = CommunitySpec { species: vec![SpeciesSpec::new("ecoli", Arc::new(model))], ..Default::default() };
        EditableMap::from_community(&CommunityModel::new(spec).expect("community"), &LayoutOptions::default())
    }

    fn pick(locals: &[&str]) -> Selection {
        let mut selection = empty_selection();
        for local in locals {
            selection.reactions.insert(format!("ecoli@{local}"));
        }
        selection
    }

    /// The main metabolite nodes a reaction is drawn to.
    fn main_nodes(map: &EditableMap, rid: &str) -> Vec<String> {
        let (substrates, products) = main_ends(map, rid);
        let mut nodes = Vec::new();
        for list in substrates.values().chain(products.values()) {
            nodes.extend(list.iter().cloned());
        }
        nodes
    }

    fn with_frd7() -> Vec<&'static str> {
        let mut locals = TCA.to_vec();
        locals.push("FRD7");
        locals
    }

    #[test]
    fn the_tca_cycle_is_found_among_the_selected_reactions_and_a_chain_is_not_a_cycle() {
        let mut map = ecoli_map();
        let cycle = selection_cycle(&map, &pick(&with_frd7())).expect("a cycle");
        assert_eq!(cycle.reactions.len(), 8);
        assert_eq!(cycle.metabolites.iter().collect::<HashSet<_>>().len(), 8);

        let chain = pick(&["PGI", "PFK", "FBA"]);
        assert_eq!(selection_cycle(&map, &chain), None);
        let before = map.to_json();
        assert_eq!(circularize_selection(&mut map, &chain), Circularized::Refused(CircleRefusal::Cycle));
        assert_eq!(map.to_json(), before, "a refusal changes nothing");
    }

    #[test]
    fn circularize_lays_the_cycle_on_one_circle_in_equal_steps_that_grow_with_it() {
        let mut map = ecoli_map();
        let selection = pick(&with_frd7());
        let cycle = selection_cycle(&map, &selection).expect("a cycle");
        let Circularized::Circled { reactions, radius, centre } = circularize_selection(&mut map, &selection) else {
            panic!("refused")
        };
        assert_eq!(reactions, 8);
        assert!((radius - 16.0 * 84.0 / (2.0 * PI)).abs() < 1e-9, "sixteen steps of 84 px round");

        // Metabolite, reaction, metabolite, ...: each on the circle, a sixteenth of a turn apart.
        let mut ring: Vec<Point> = Vec::new();
        let mut ring_nodes = HashSet::new();
        for (i, rid) in cycle.reactions.iter().enumerate() {
            let mut on_circle = None;
            for n in main_nodes(&map, rid) {
                if map.node(&n).and_then(|node| node.merged_id.as_deref()) == Some(cycle.metabolites[i].as_str()) {
                    on_circle = Some(n);
                }
            }
            let node = on_circle.expect("its metabolite on the circle");
            ring.push(map.node(&node).expect("node").pos());
            ring_nodes.insert(node);
            let rxn = map.reaction(rid).expect("reaction");
            ring.push(Point::new(rxn.x, rxn.y));
            // Its name is written inside the circle, its backbone runs along it.
            let offset = coxeter_core::reaction_label::reaction_label_offset(rxn).expect("a name offset");
            assert!(offset.x * (rxn.x - centre.x) + offset.y * (rxn.y - centre.y) < 0.0, "{rid} named inside");
            let dir = rxn.axis.expect("a backbone").dir;
            assert!((dir.x * (rxn.x - centre.x) + dir.y * (rxn.y - centre.y)).abs() < 1e-6 * radius, "{rid} along the circle");
        }
        let step = 2.0 * PI / 16.0;
        for (j, p) in ring.iter().enumerate() {
            assert!((js::hypot(p.x - centre.x, p.y - centre.y) - radius).abs() < 1e-6, "item {j} on the circle");
            let next = ring[(j + 1) % ring.len()];
            let turn = wrap(js::atan2(next.y - centre.y, next.x - centre.x) - js::atan2(p.y - centre.y, p.x - centre.x));
            assert!((turn.abs() - step).abs() < 1e-6, "item {j} one step from the next");
        }

        // FRD7 and SUCDi both run between fumarate and succinate: the one not
        // on the circle stands outside it, beside the other, drawn to the same
        // two nodes, 90 px out or as far as its compartment lets it.
        let beside = ["ecoli@FRD7", "ecoli@SUCDi"].into_iter().find(|r| !cycle.reactions.iter().any(|c| c == r)).expect("one beside");
        let rxn = map.reaction(beside).expect("reaction");
        let out = js::hypot(rxn.x - centre.x, rxn.y - centre.y);
        assert!(out > radius + 30.0 && out < radius + 90.0 + 1e-6);
        let drawn_to = main_nodes(&map, beside);
        assert_eq!(drawn_to.iter().filter(|n| ring_nodes.contains(*n)).count(), 2, "drawn to the circle's own nodes");
    }

    #[test]
    fn copies_of_a_cycle_metabolite_are_joined_into_one_node_on_the_circle() {
        let mut map = ecoli_map();
        let mut citrate = None;
        for (i, seg) in map.segments().iter().enumerate() {
            let merged = seg.node_id.as_deref().and_then(|n| map.node(n)).and_then(|n| n.merged_id.as_deref());
            if seg.reaction_id == "ecoli@CS" && merged.is_some_and(|m| m.ends_with("cit_c")) {
                citrate = Some(i);
            }
        }
        let citrate = citrate.expect("CS draws citrate");
        let source = map.segments()[citrate].node_id.clone().expect("node");
        let copy = map.duplicate_metabolite_node(&source, &Default::default()).expect("a copy").id.clone();
        map.edit_graph(|g| g.segments[citrate].node_id = Some(copy.clone()));
        let merged = map.node(&copy).and_then(|n| n.merged_id.clone()).expect("merged id");
        let before = map.nodes_for_metabolite(&merged).len();

        assert!(matches!(circularize_selection(&mut map, &pick(&TCA)), Circularized::Circled { .. }));
        assert_eq!(map.nodes_for_metabolite(&merged).len(), before - 1, "one copy fewer");
        let acont = main_nodes(&map, "ecoli@ACONT");
        let shared = main_nodes(&map, "ecoli@CS").into_iter().filter(|n| acont.contains(n)).count();
        assert_eq!(shared, 1, "CS and ACONT meet at one citrate");
    }

    #[test]
    fn a_circle_with_no_room_in_its_compartment_is_refused_and_changes_nothing() {
        let mut map = ecoli_map();
        let node = map.node(&main_nodes(&map, "ecoli@CS")[0]).expect("node").clone();
        map.edit_graph(|g| {
            let cell = g.cells.iter_mut().find(|c| Some(&c.id) == node.cell_id.as_ref()).expect("its cell");
            let compartment = cell.compartments.iter_mut().find(|c| Some(&c.id) == node.compartment.as_ref()).expect("its compartment");
            compartment.rect.w = 200.0;
            compartment.rect.h = 200.0;
        });
        let before = map.to_json();
        assert_eq!(circularize_selection(&mut map, &pick(&TCA)), Circularized::Refused(CircleRefusal::Room));
        assert_eq!(map.to_json(), before);
    }
}
