//! The logic behind the Reactions page, kept free of any GUI so it can be
//! tested as it is (port of `src/ui/reactions-explorer.js`).
//!
//! ```text
//!   types      what kind of reaction a row is (exchange, transport, ...)
//!   sign-log   the slider scale every bound control uses
//!   rows       search, quick filters, sorting and grouping of table rows
//!   history    undo/redo over constraint snapshots, apart from the map's
//!   CSV        bounds files in and out
//! ```
//!
//! `virtualWindow` is not ported: egui's `ScrollArea::show_rows` lays out
//! only the rows in view.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, LINK_COMPARTMENT, ReactionRole, SHARED_COMPARTMENT, COMMUNITY_GROWTH, split_id};
use coxeter_core::js;
use coxeter_core::model::{Metabolite, Reaction, ReactionKind};
use indexmap::{IndexMap, IndexSet};

use crate::omics::to_precision;
use crate::types::{Bounds, CustomObjective};
use crate::util::normalize_search;

/// `Number(x.toPrecision(digits))`: `x` rounded to so many significant digits.
pub fn round_precision(x: f64, digits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    js::string_to_number(&to_precision(x, digits))
}

// Types

/// The kinds of reaction the table tells apart, in the order groups are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReactionType {
    Exchange,
    Uptake,
    Transport,
    Internal,
    Biomass,
    Demand,
}

impl ReactionType {
    pub const ALL: [ReactionType; 6] = [
        ReactionType::Exchange,
        ReactionType::Uptake,
        ReactionType::Transport,
        ReactionType::Internal,
        ReactionType::Biomass,
        ReactionType::Demand,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ReactionType::Exchange => "exchange",
            ReactionType::Uptake => "uptake",
            ReactionType::Transport => "transport",
            ReactionType::Internal => "internal",
            ReactionType::Biomass => "biomass",
            ReactionType::Demand => "demand",
        }
    }

    /// `reactionTypeLabel(type)`: the table's name for it.
    pub fn label(self) -> &'static str {
        match self {
            ReactionType::Exchange => "Exchange",
            ReactionType::Uptake => "Uptake/secretion",
            ReactionType::Transport => "Transport",
            ReactionType::Internal => "Internal",
            ReactionType::Biomass => "Biomass",
            ReactionType::Demand => "Demand/Sink",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ReactionType::Exchange => "Boundary of the community or of a model: supply and washout",
            ReactionType::Uptake => "A species takes from or gives to the shared medium",
            ReactionType::Transport => "Moves metabolites between compartments of one cell",
            ReactionType::Internal => "Takes place inside one compartment",
            ReactionType::Biomass => "Growth: a biomass or community growth reaction",
            ReactionType::Demand => "A demand or sink: drains or supplies one metabolite",
        }
    }

    /// Where its group is listed (`TYPE_ORDER`).
    pub fn order(self) -> usize {
        ReactionType::ALL.iter().position(|t| *t == self).unwrap_or(99)
    }

    pub fn from_id(id: &str) -> Option<ReactionType> {
        ReactionType::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// `classifyReaction(community, rid)`: what kind of reaction `rid` is.
///
/// Read from how the community was put together, not from the id alone: a
/// species' exchange became a transfer to the shared medium (uptake/secretion),
/// the community's own boundary is an exchange, and the biomass reactions are
/// the ones the community grows by. DM_/SK_ prefixes mark demands and sinks.
pub fn classify_reaction(community: &CommunityModel, rid: &str) -> ReactionType {
    let reaction = community.merged.reactions.get(rid);
    let owner = community.reaction_owner.get(rid);
    let local_id = owner.map_or(rid, |o| o.original_id.as_str());
    if rid == COMMUNITY_GROWTH
        || owner.is_some_and(|o| o.role == ReactionRole::Biomass)
        || reaction.is_some_and(|r| r.kind == ReactionKind::Biomass)
        || community.biomass_ids.values().any(|id| id == rid)
    {
        return ReactionType::Biomass;
    }
    // A reaction the community did not place: only the model's own kind is known.
    let Some(owner) = owner else {
        return match reaction.map(|r| r.kind) {
            Some(ReactionKind::Exchange) => ReactionType::Exchange,
            Some(ReactionKind::Sink) => ReactionType::Demand,
            Some(ReactionKind::Transport) => ReactionType::Transport,
            _ => ReactionType::Internal,
        };
    };
    if owner.species_id.is_none() {
        return ReactionType::Exchange;
    }
    if owner.role == ReactionRole::Transfer {
        return ReactionType::Uptake;
    }
    let prefix = local_id.get(..3).map(str::to_ascii_uppercase);
    if owner.role == ReactionRole::Sink || matches!(prefix.as_deref(), Some("DM_" | "SK_")) {
        return ReactionType::Demand;
    }
    if owner.role == ReactionRole::Exchange {
        return ReactionType::Exchange;
    }
    let compartments = reaction.map_or(0, |r| r.compartments.iter().filter(|c| *c != LINK_COMPARTMENT).count());
    if owner.role == ReactionRole::Transport || compartments > 1 {
        return ReactionType::Transport;
    }
    ReactionType::Internal
}

// Sign-log slider

/// The scale of a sign-log slider (`SIGN_LOG`).
///
/// Bounds span six orders of magnitude in practice (a trace uptake of 0.001,
/// an unbounded 1000), and a linear slider over ±1000 cannot set anything
/// below a few units. The sign-log scale gives every decade the same travel,
/// both signs mirrored, with the middle position (and its neighbours) exactly
/// 0. The number box beside it stays exact; the slider is for moving fast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignLogScale {
    pub min: f64,
    pub max: f64,
    pub steps_per_decade: f64,
    pub dead_zone: i64,
}

/// ±0.001 to ±1000, 40 steps a decade, the step either side of 0 is 0.
pub const SIGN_LOG: SignLogScale = SignLogScale { min: 1e-3, max: 1e3, steps_per_decade: 40.0, dead_zone: 1 };

/// `signLogSteps(scale)`: the largest slider position; positions run from
/// `-steps` to `+steps`.
pub fn sign_log_steps(scale: &SignLogScale) -> i64 {
    scale.dead_zone + 1 + js::round(js::log10(scale.max / scale.min) * scale.steps_per_decade) as i64
}

/// `signLogValue(position, scale)`: 0 within the dead zone, then ±min to ±max
/// logarithmically, rounded to three significant digits so a drag lands on
/// numbers worth reading.
pub fn sign_log_value(position: f64, scale: &SignLogScale) -> f64 {
    let steps = sign_log_steps(scale);
    let p = if position.is_nan() { 0 } else { js::round(position) as i64 };
    let a = p.abs().min(steps);
    if a <= scale.dead_zone {
        return 0.0;
    }
    let t = (a - scale.dead_zone - 1) as f64 / (steps - scale.dead_zone - 1) as f64;
    let value = round_precision(scale.min * js::pow(scale.max / scale.min, t), 3);
    p.signum() as f64 * value
}

/// `signLogPosition(value, scale)`: the slider position for a value. Beyond
/// ±max (1e6 for "unbounded") it sits at the end; a non-zero value below min
/// sits on the first step off zero, so the slider never claims a non-zero
/// bound is 0.
pub fn sign_log_position(value: f64, scale: &SignLogScale) -> i64 {
    let steps = sign_log_steps(scale);
    if value.is_nan() || value == 0.0 {
        return 0;
    }
    let sign = if value < 0.0 { -1 } else { 1 };
    let a = value.abs();
    if a >= scale.max {
        return sign * steps;
    }
    if a <= scale.min {
        return sign * (scale.dead_zone + 1);
    }
    let t = js::log10(a / scale.min) / js::log10(scale.max / scale.min);
    sign * (scale.dead_zone + 1 + js::round(t * (steps - scale.dead_zone - 1) as f64) as i64)
}

// Bounds

const TOLERANCE: f64 = 1e-9;

/// `sameBounds(a, b)`: whether two pairs are the same (both absent counts as
/// the same).
pub fn same_bounds(a: Option<&Bounds>, b: Option<&Bounds>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a.lower - b.lower).abs() <= TOLERANCE && (a.upper - b.upper).abs() <= TOLERANCE,
        (None, None) => true,
        _ => false,
    }
}

/// `isKnockedOut(current, original)`: closed by the user, both bounds 0
/// where the model had it open.
pub fn is_knocked_out(current: &Bounds, original: &Bounds) -> bool {
    let closed = |b: &Bounds| b.lower.abs() <= TOLERANCE && b.upper.abs() <= TOLERANCE;
    closed(current) && !closed(original)
}

// Catalogue

/// One reaction as the table lists it, before any solve: its names, where
/// it is, what it is, and the text search runs over.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub id: String,
    pub index: usize,
    pub local_id: String,
    pub label: String,
    pub name: String,
    pub species_id: Option<String>,
    /// The species id, or `Community`.
    pub species: String,
    pub species_name: String,
    pub compartments: Vec<String>,
    pub compartment_label: String,
    pub subsystem: String,
    pub pathway: String,
    pub kind: ReactionType,
    pub equation: String,
    pub genes: String,
    pub ec: String,
    pub search: String,
}

/// How a catalogue names things (`names` of `buildReactionCatalog`): the
/// label the map would show, a compartment of a species, a species' full name.
pub struct CatalogNames<'a> {
    pub label: Box<dyn Fn(&str) -> String + 'a>,
    pub compartment: Box<dyn Fn(Option<&str>, &str) -> String + 'a>,
    pub species: Box<dyn Fn(Option<&str>) -> String + 'a>,
}

impl Default for CatalogNames<'_> {
    fn default() -> Self {
        CatalogNames {
            label: Box::new(|rid| rid.to_string()),
            compartment: Box::new(|_, id| id.to_string()),
            species: Box::new(|id| id.unwrap_or("Community").to_string()),
        }
    }
}

/// `buildReactionCatalog(community, names)`: one entry per merged reaction,
/// in model order.
pub fn build_reaction_catalog(community: &CommunityModel, names: &CatalogNames) -> Vec<CatalogEntry> {
    let merged = &community.merged;
    let mut rows = Vec::with_capacity(merged.reactions.len());
    for (index, (id, reaction)) in merged.reactions.iter().enumerate() {
        let owner = community.reaction_owner.get(id);
        let species_id = owner.and_then(|o| o.species_id.clone());
        let local_id = owner.map_or_else(|| id.clone(), |o| o.original_id.clone());
        let label = (names.label)(id);
        let metabolites: Vec<(&String, Option<&Metabolite>)> = reaction
            .metabolites
            .keys()
            .map(|mid| (mid, merged.metabolites.get(mid)))
            .filter(|(_, m)| m.is_none_or(|m| m.compartment != LINK_COMPARTMENT))
            .collect();
        // A cell may call one of its own compartments `s` too, so the shared
        // medium is told apart by who owns the metabolite, not by the id.
        let mut places: IndexMap<String, String> = IndexMap::new();
        for (mid, metabolite) in &metabolites {
            let mowner = community.metabolite_owner.get(*mid);
            let shared = match mowner {
                Some(o) => o.species_id.is_none(),
                None => metabolite.is_some_and(|m| m.compartment == SHARED_COMPARTMENT),
            };
            let cid = if shared { SHARED_COMPARTMENT.to_string() } else { metabolite.map_or_else(String::new, |m| m.compartment.clone()) };
            if !places.contains_key(&cid) {
                let name = if shared {
                    "Shared medium".to_string()
                } else {
                    let sp = mowner.and_then(|o| o.species_id.as_deref()).or(species_id.as_deref());
                    (names.compartment)(sp, &cid)
                };
                places.insert(cid, name);
            }
        }
        let mut compartments: Vec<String> = places.keys().cloned().collect();
        compartments.sort_by(|a, b| js::code_unit_compare(a, b));
        let compartment_label = compartments.iter().map(|c| places[c].as_str()).collect::<Vec<_>>().join(" ↔ ");
        let ec = reaction.label_sources.get("ec").map(|v| v.join(" ")).unwrap_or_default();
        let genes = reaction.gene_reaction_rule.clone();
        let metabolite_text = metabolites
            .iter()
            .map(|(mid, m)| format!("{} {}", metabolite_local_id(mid), m.map_or("", |m| m.name.as_str())))
            .collect::<Vec<_>>()
            .join(" ");
        let search = normalize_search(&[id.as_str(), &local_id, &label, &reaction.name, &genes, &ec, &metabolite_text].join(" "));
        rows.push(CatalogEntry {
            id: id.clone(),
            index,
            species: species_id.clone().unwrap_or_else(|| "Community".into()),
            species_name: (names.species)(species_id.as_deref()),
            species_id,
            local_id,
            label,
            name: reaction.name.clone(),
            compartments,
            compartment_label,
            subsystem: reaction.subsystem.clone(),
            pathway: reaction.pathway.clone(),
            kind: classify_reaction(community, id),
            equation: readable_equation(reaction, &metabolites),
            genes,
            ec,
            search,
        });
    }
    rows
}

fn metabolite_local_id(mid: &str) -> String {
    match split_id(mid) {
        (Some("M"), id) => format!("{id}[medium]"),
        (_, id) => id.to_string(),
    }
}

/// `a + 2 b --> c`, with the cell prefix taken off and medium metabolites marked.
fn readable_equation(reaction: &Reaction, metabolites: &[(&String, Option<&Metabolite>)]) -> String {
    let side = |sign: f64| {
        let terms: Vec<String> = metabolites
            .iter()
            .filter_map(|(mid, _)| reaction.metabolites.get(*mid).map(|c| (*mid, *c)))
            .filter(|(_, c)| js::sign(*c) == sign)
            .map(|(mid, c)| {
                if c.abs() == 1.0 {
                    metabolite_local_id(mid)
                } else {
                    format!("{} {}", js::number_to_string(round_precision(c.abs(), 6)), metabolite_local_id(mid))
                }
            })
            .collect();
        if terms.is_empty() { "∅".to_string() } else { terms.join(" + ") }
    };
    let (lower, upper) = (reaction.lower_bound, reaction.upper_bound);
    let arrow = if lower < 0.0 && upper > 0.0 {
        "<=>"
    } else if upper > 0.0 {
        "-->"
    } else {
        "<--"
    };
    format!("{} {arrow} {}", side(-1.0), side(1.0))
}

// Rows

/// What a row says about its bounds and solve, in order of precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowStatus {
    Culprit,
    KnockedOut,
    Modified,
    Blocked,
    Original,
}

impl RowStatus {
    pub fn id(self) -> &'static str {
        match self {
            RowStatus::Culprit => "culprit",
            RowStatus::KnockedOut => "knocked-out",
            RowStatus::Modified => "modified",
            RowStatus::Blocked => "blocked",
            RowStatus::Original => "original",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RowStatus::Culprit => "Infeasible culprit",
            RowStatus::KnockedOut => "Knocked out",
            RowStatus::Modified => "Modified",
            RowStatus::Blocked => "Blocked",
            RowStatus::Original => "Original",
        }
    }

    fn order(self) -> usize {
        self as usize
    }
}

/// A table row: the catalogue entry with its bounds in force and as
/// opened, flux, FVA range, reduced cost, objective coefficient, whether it
/// is drawn, and its status. Numbers it does not have are NaN.
#[derive(Debug, Clone)]
pub struct ReactionRow {
    pub entry: Arc<CatalogEntry>,
    pub lower: f64,
    pub upper: f64,
    pub original_lower: f64,
    pub original_upper: f64,
    pub modified: bool,
    pub knocked_out: bool,
    pub blocked: bool,
    pub culprit: bool,
    pub flux: f64,
    pub fva_min: f64,
    pub fva_max: f64,
    pub reduced_cost: f64,
    pub objective: f64,
    pub on_map: bool,
    pub held_out: bool,
    pub status: RowStatus,
}

impl ReactionRow {
    pub fn id(&self) -> &str {
        &self.entry.id
    }
}

/// The quick filters, each a test on a table row. Several together all have
/// to hold (`REACTION_FILTERS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReactionFilter {
    Exchanges,
    Modified,
    Active,
    Blocked,
    OnMap,
    OffMap,
    KnockedOut,
}

impl ReactionFilter {
    pub const ALL: [ReactionFilter; 7] = [
        ReactionFilter::Exchanges,
        ReactionFilter::Modified,
        ReactionFilter::Active,
        ReactionFilter::Blocked,
        ReactionFilter::OnMap,
        ReactionFilter::OffMap,
        ReactionFilter::KnockedOut,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ReactionFilter::Exchanges => "exchanges",
            ReactionFilter::Modified => "modified",
            ReactionFilter::Active => "active",
            ReactionFilter::Blocked => "blocked",
            ReactionFilter::OnMap => "on-map",
            ReactionFilter::OffMap => "off-map",
            ReactionFilter::KnockedOut => "knocked-out",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ReactionFilter::Exchanges => "Exchanges only",
            ReactionFilter::Modified => "Modified",
            ReactionFilter::Active => "Active",
            ReactionFilter::Blocked => "Blocked",
            ReactionFilter::OnMap => "On map",
            ReactionFilter::OffMap => "Not on map",
            ReactionFilter::KnockedOut => "Knocked out",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ReactionFilter::Exchanges => "Exchanges and uptake/secretion",
            ReactionFilter::Modified => "Bounds differ from the model’s",
            ReactionFilter::Active => "|flux| > 1e-9 in the solution",
            ReactionFilter::Blocked => "FVA range is 0 to 0",
            ReactionFilter::OnMap => "Drawn on the map",
            ReactionFilter::OffMap => "In the model, not drawn",
            ReactionFilter::KnockedOut => "Both bounds 0 where the model had it open",
        }
    }

    pub fn from_id(id: &str) -> Option<ReactionFilter> {
        ReactionFilter::ALL.into_iter().find(|f| f.id() == id)
    }

    fn test(self, row: &ReactionRow) -> bool {
        match self {
            ReactionFilter::Exchanges => matches!(row.entry.kind, ReactionType::Exchange | ReactionType::Uptake),
            ReactionFilter::Modified => row.modified,
            ReactionFilter::Active => row.flux.abs() > 1e-9,
            ReactionFilter::Blocked => row.blocked,
            ReactionFilter::OnMap => row.on_map,
            ReactionFilter::OffMap => !row.on_map,
            ReactionFilter::KnockedOut => row.knocked_out,
        }
    }
}

/// `toggleReactionFilter(filters, id)`: one quick filter on or off. On map
/// and Not on map exclude each other.
pub fn toggle_reaction_filter(filters: &[ReactionFilter], filter: ReactionFilter) -> Vec<ReactionFilter> {
    let mut next: HashSet<ReactionFilter> = filters.iter().copied().collect();
    if !next.remove(&filter) {
        next.insert(filter);
        match filter {
            ReactionFilter::OnMap => {
                next.remove(&ReactionFilter::OffMap);
            }
            ReactionFilter::OffMap => {
                next.remove(&ReactionFilter::OnMap);
            }
            _ => {}
        }
    }
    ReactionFilter::ALL.into_iter().filter(|f| next.contains(f)).collect()
}

/// `filterReactionRows(rows, { query, filters })`: the rows a search and the
/// quick filters leave. Every word of the query has to appear in the row's
/// id, name, genes, metabolites or EC numbers.
pub fn filter_reaction_rows(rows: &[ReactionRow], query: &str, filters: &[ReactionFilter]) -> Vec<ReactionRow> {
    let normalized = normalize_search(query);
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.is_empty() && filters.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|row| words.iter().all(|w| row.entry.search.contains(w)) && filters.iter().all(|f| f.test(row)))
        .cloned()
        .collect()
}

/// The columns the table sorts by (`REACTION_SORT_KEYS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortKey {
    Reaction,
    Species,
    Compartment,
    Subsystem,
    Type,
    Lower,
    Upper,
    Flux,
    Fva,
    Rc,
    Objective,
    Status,
}

impl SortKey {
    /// A key compared as text (the others are numbers).
    fn text(self, row: &ReactionRow) -> Option<&str> {
        match self {
            SortKey::Reaction => Some(&row.entry.label),
            SortKey::Species => Some(&row.entry.species),
            SortKey::Compartment => Some(&row.entry.compartment_label),
            SortKey::Subsystem => Some(&row.entry.subsystem),
            _ => None,
        }
    }

    fn number(self, row: &ReactionRow) -> f64 {
        match self {
            SortKey::Type => row.entry.kind.order() as f64,
            SortKey::Lower => row.lower,
            SortKey::Upper => row.upper,
            SortKey::Flux => row.flux,
            SortKey::Fva => row.fva_max - row.fva_min,
            SortKey::Rc => row.reduced_cost,
            SortKey::Objective => row.objective,
            SortKey::Status => row.status.order() as f64,
            _ => f64::NAN,
        }
    }

    /// A column whose first click sorts largest first: the numbers other
    /// than type and status.
    pub fn descends_first(self) -> bool {
        !matches!(self, SortKey::Reaction | SortKey::Species | SortKey::Compartment | SortKey::Subsystem | SortKey::Type | SortKey::Status)
    }
}

/// How the table is sorted: a column and `dir` (1 ascending, -1 descending).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub key: SortKey,
    pub dir: i32,
}

impl Default for SortSpec {
    fn default() -> Self {
        SortSpec { key: SortKey::Reaction, dir: 1 }
    }
}

/// `sortReactionRows(rows, { key, dir })`. Missing numbers go last either
/// way; ties go by label.
pub fn sort_reaction_rows(mut rows: Vec<ReactionRow>, sort: SortSpec) -> Vec<ReactionRow> {
    let sign = if sort.dir < 0 { -1.0 } else { 1.0 };
    let tie = |a: &ReactionRow, b: &ReactionRow| {
        js::locale_compare(&a.entry.label, &b.entry.label).then_with(|| js::locale_compare(&a.entry.id, &b.entry.id))
    };
    if is_text(sort.key) {
        rows.sort_by(|a, b| {
            let o = js::locale_compare(sort.key.text(a).unwrap_or(""), sort.key.text(b).unwrap_or(""));
            (if sign < 0.0 { o.reverse() } else { o }).then_with(|| tie(a, b))
        });
        return rows;
    }
    rows.sort_by(|a, b| {
        let (x, y) = (sort.key.number(a), sort.key.number(b));
        match (x.is_finite(), y.is_finite()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => (sign * (x - y)).partial_cmp(&0.0).unwrap_or(Ordering::Equal).then_with(|| tie(a, b)),
            (false, false) => tie(a, b),
        }
    });
    rows
}

fn is_text(key: SortKey) -> bool {
    matches!(key, SortKey::Reaction | SortKey::Species | SortKey::Compartment | SortKey::Subsystem)
}

/// How the table can be grouped (`REACTION_GROUPINGS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Grouping {
    #[default]
    Type,
    Compartment,
    Species,
    Subsystem,
    Pathway,
    None,
}

impl Grouping {
    pub const ALL: [Grouping; 6] =
        [Grouping::Type, Grouping::Compartment, Grouping::Species, Grouping::Subsystem, Grouping::Pathway, Grouping::None];

    pub fn id(self) -> &'static str {
        match self {
            Grouping::Type => "type",
            Grouping::Compartment => "compartment",
            Grouping::Species => "species",
            Grouping::Subsystem => "subsystem",
            Grouping::Pathway => "pathway",
            Grouping::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Grouping::Type => "Type",
            Grouping::Compartment => "Compartment",
            Grouping::Species => "Species",
            Grouping::Subsystem => "Subsystem",
            Grouping::Pathway => "Pathway",
            Grouping::None => "None",
        }
    }
}

/// A group header of the table: its rows' count, how many are modified and
/// ticked, whether it is folded, and its reaction ids.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupItem {
    pub key: String,
    pub label: String,
    pub count: usize,
    pub modified: usize,
    pub selected: usize,
    pub collapsed: bool,
    pub ids: Vec<String>,
}

/// What the table lists, one line each: a group header or a row.
#[derive(Debug, Clone)]
pub enum TableItem {
    Group(GroupItem),
    Row(ReactionRow),
}

/// `groupReactionRows(rows, groupBy, { collapsed, selection, pathwayOf })`:
/// a header item per group, followed by its rows unless the group is
/// collapsed. Without grouping, just the rows. `rows` are already filtered
/// and sorted; `pathway_of` is the pathway manager's group of a reaction.
pub fn group_reaction_rows(
    rows: Vec<ReactionRow>,
    group_by: Grouping,
    collapsed: &HashSet<String>,
    selection: &IndexSet<String>,
    pathway_of: Option<&dyn Fn(&str) -> Option<String>>,
) -> Vec<TableItem> {
    if group_by == Grouping::None {
        return rows.into_iter().map(TableItem::Row).collect();
    }
    let key_of = |row: &ReactionRow| -> (String, String) {
        let e = &row.entry;
        match group_by {
            Grouping::Compartment => {
                let some = !e.compartment_label.is_empty();
                (
                    if some { e.compartment_label.clone() } else { "—".into() },
                    if some { e.compartment_label.clone() } else { "No compartment".into() },
                )
            }
            Grouping::Species => match &e.species_id {
                Some(id) => (id.clone(), e.species.clone()),
                None => (String::new(), "Community".into()),
            },
            Grouping::Subsystem => {
                (e.subsystem.clone(), if e.subsystem.is_empty() { "No subsystem".into() } else { e.subsystem.clone() })
            }
            Grouping::Pathway => {
                let name = pathway_of.and_then(|f| f(&e.id)).filter(|n| !n.is_empty()).unwrap_or_else(|| e.pathway.clone());
                let label = if name.is_empty() { "No pathway".to_string() } else { name.clone() };
                (name, label)
            }
            Grouping::Type | Grouping::None => (e.kind.id().to_string(), e.kind.label().to_string()),
        }
    };
    let mut groups: IndexMap<String, (String, Vec<ReactionRow>)> = IndexMap::new();
    for row in rows {
        let (key, label) = key_of(&row);
        groups.entry(key).or_insert_with(|| (label, Vec::new())).1.push(row);
    }
    let mut ordered: Vec<(String, String, Vec<ReactionRow>)> = groups.into_iter().map(|(k, (l, r))| (k, l, r)).collect();
    ordered.sort_by(|a, b| {
        if group_by == Grouping::Type {
            let order = |k: &str| ReactionType::from_id(k).map_or(99, ReactionType::order);
            return order(&a.0).cmp(&order(&b.0));
        }
        // The group of rows with nothing to group by goes last.
        match (a.0.is_empty(), b.0.is_empty()) {
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            _ => js::locale_compare(&a.1, &b.1),
        }
    });
    let mut items = Vec::new();
    for (key, label, rows) in ordered {
        let ids: Vec<String> = rows.iter().map(|r| r.entry.id.clone()).collect();
        let folded = collapsed.contains(&key);
        items.push(TableItem::Group(GroupItem {
            count: rows.len(),
            modified: rows.iter().filter(|r| r.modified).count(),
            selected: ids.iter().filter(|id| selection.contains(*id)).count(),
            collapsed: folded,
            key,
            label,
            ids,
        }));
        if !folded {
            items.extend(rows.into_iter().map(TableItem::Row));
        }
    }
    items
}

// Constraint history

/// The objective part of a constraint snapshot: the preset and an edit on top.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveSnapshot {
    pub kind: Option<String>,
    pub product_target: Option<String>,
    pub custom: Option<CustomObjective>,
}

/// What the LP is asked, as plain data in a fixed order (`constraintSnapshot()`):
/// overrides and medium sorted by id, the objective.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintSnapshot {
    pub overrides: Vec<(String, Bounds)>,
    pub medium: Vec<(String, f64)>,
    pub objective: ObjectiveSnapshot,
}

/// `JSON.stringify` writes every non-finite number as `null`.
fn same_number(a: f64, b: f64) -> bool {
    if a.is_finite() || b.is_finite() { a == b } else { true }
}

/// `sameConstraints(a, b)`: whether two snapshots ask the LP the same thing.
pub fn same_constraints(a: &ConstraintSnapshot, b: &ConstraintSnapshot) -> bool {
    let bounds = |x: &Bounds, y: &Bounds| same_number(x.lower, y.lower) && same_number(x.upper, y.upper);
    let custom = |x: &Option<CustomObjective>, y: &Option<CustomObjective>| match (x, y) {
        (Some(x), Some(y)) => {
            x.sense == y.sense
                && x.coefficients.len() == y.coefficients.len()
                && x.coefficients.iter().zip(&y.coefficients).all(|((i, u), (j, v))| i == j && same_number(*u, *v))
        }
        (None, None) => true,
        _ => false,
    };
    a.overrides.len() == b.overrides.len()
        && a.overrides.iter().zip(&b.overrides).all(|((i, x), (j, y))| i == j && bounds(x, y))
        && a.medium.len() == b.medium.len()
        && a.medium.iter().zip(&b.medium).all(|((i, x), (j, y))| i == j && same_number(*x, *y))
        && a.objective.kind == b.objective.kind
        && a.objective.product_target == b.objective.product_target
        && custom(&a.objective.custom, &b.objective.custom)
}

/// `changedReactionIds(before, after, mediumReactionId)`: the reactions whose
/// constraints differ between two snapshots: overridden bounds, medium
/// limits (through `medium_reaction_id`, the exchange a medium metabolite
/// opens) and objective coefficients. These are the suspects when a change
/// leaves the LP without a solution.
pub fn changed_reaction_ids(
    before: &ConstraintSnapshot,
    after: &ConstraintSnapshot,
    medium_reaction_id: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut ids: IndexSet<String> = IndexSet::new();
    let a: IndexMap<&str, &Bounds> = before.overrides.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let b: IndexMap<&str, &Bounds> = after.overrides.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let keys: IndexSet<&str> = a.keys().chain(b.keys()).copied().collect();
    for id in keys {
        if !same_bounds(a.get(id).copied(), b.get(id).copied()) {
            ids.insert(id.to_string());
        }
    }
    let ma: IndexMap<&str, f64> = before.medium.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let mb: IndexMap<&str, f64> = after.medium.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let bases: IndexSet<&str> = ma.keys().chain(mb.keys()).copied().collect();
    for base in bases {
        // `Number(x ?? NaN) !== Number(y ?? NaN)`: NaN differs from itself.
        let (x, y) = (ma.get(base).copied().unwrap_or(f64::NAN), mb.get(base).copied().unwrap_or(f64::NAN));
        if x != y {
            if let Some(id) = medium_reaction_id(base) {
                ids.insert(id);
            }
        }
    }
    let empty = IndexMap::new();
    let ca = before.objective.custom.as_ref().map_or(&empty, |c| &c.coefficients);
    let cb = after.objective.custom.as_ref().map_or(&empty, |c| &c.coefficients);
    let keys: IndexSet<&String> = ca.keys().chain(cb.keys()).collect();
    for id in keys {
        if ca.get(id).copied().unwrap_or(0.0) != cb.get(id).copied().unwrap_or(0.0) {
            ids.insert(id.clone());
        }
    }
    ids.into_iter().collect()
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    label: String,
    key: Option<String>,
    at: f64,
    snapshot: ConstraintSnapshot,
}

/// A step [`ConstraintHistory::undo`] or [`ConstraintHistory::redo`] gives
/// back: what it was called and the constraints to restore.
#[derive(Debug, Clone)]
pub struct HistoryStep {
    pub label: String,
    pub snapshot: ConstraintSnapshot,
}

/// Undo and redo for what the LP is asked: reaction bounds, the medium and
/// the objective. The map keeps its own history (`MapHistory`); the two
/// never mix, so the map's Ctrl+Z cannot take back a knockout, nor this a
/// drag.
///
/// Entries hold the snapshot taken before a change. A run of changes to the
/// same thing within `coalesce_ms` (one slider drag, typing in a box) is one
/// entry, so Undo goes back to before the drag rather than one tick of it.
#[derive(Debug, Clone)]
pub struct ConstraintHistory {
    limit: usize,
    coalesce_ms: f64,
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
}

impl Default for ConstraintHistory {
    fn default() -> Self {
        ConstraintHistory::new(100, 1500.0)
    }
}

impl ConstraintHistory {
    pub fn new(limit: usize, coalesce_ms: f64) -> ConstraintHistory {
        ConstraintHistory { limit: limit.max(1), coalesce_ms, undo_stack: Vec::new(), redo_stack: Vec::new() }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
    pub fn undo_label(&self) -> &str {
        self.undo_stack.last().map_or("", |e| e.label.as_str())
    }
    pub fn redo_label(&self) -> &str {
        self.redo_stack.last().map_or("", |e| e.label.as_str())
    }
    /// How many steps Undo can take.
    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Record the snapshot from before a change, made at `at` (ms). Returns
    /// the snapshot of the entry the change belongs to: a new one, or the
    /// last one when it continues the same edit.
    pub fn record(&mut self, before: &ConstraintSnapshot, label: &str, key: Option<&str>, at: f64) -> ConstraintSnapshot {
        let coalesce = self.redo_stack.is_empty() && self.coalesce_ms > 0.0;
        if let (Some(key), Some(last)) = (key, self.undo_stack.last_mut()) {
            if coalesce && last.key.as_deref() == Some(key) && at - last.at <= self.coalesce_ms {
                last.at = at;
                last.label = label.to_string();
                return last.snapshot.clone();
            }
        }
        self.undo_stack.push(HistoryEntry { label: label.to_string(), key: key.map(str::to_string), at, snapshot: before.clone() });
        if self.undo_stack.len() > self.limit {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        before.clone()
    }

    /// Step back: what to restore, with `current` kept for redo; `None` when
    /// there is nothing.
    pub fn undo(&mut self, current: &ConstraintSnapshot) -> Option<HistoryStep> {
        let entry = self.undo_stack.pop()?;
        self.redo_stack.push(HistoryEntry { label: entry.label.clone(), key: None, at: 0.0, snapshot: current.clone() });
        Some(HistoryStep { label: entry.label, snapshot: entry.snapshot })
    }

    /// Step forward again, with `current` back on the undo stack; `None` when
    /// there is nothing.
    pub fn redo(&mut self, current: &ConstraintSnapshot) -> Option<HistoryStep> {
        let entry = self.redo_stack.pop()?;
        self.undo_stack.push(HistoryEntry { label: entry.label.clone(), key: None, at: 0.0, snapshot: current.clone() });
        Some(HistoryStep { label: entry.label, snapshot: entry.snapshot })
    }
}

// Bounds CSV

/// One line of a bounds table to write (`formatBoundsCSV`'s entries).
#[derive(Debug, Clone, PartialEq)]
pub struct BoundsCsvEntry {
    pub id: String,
    pub lower: f64,
    pub upper: f64,
    pub species: String,
    pub original_id: String,
    pub name: String,
}

/// `formatBoundsCSV(entries)`: `id, lower, upper, species, original_id,
/// name`, one line per entry. Only the first three are read back; the rest
/// say what each line is.
pub fn format_bounds_csv(entries: &[BoundsCsvEntry]) -> String {
    let mut lines = vec!["id,lower,upper,species,original_id,name".to_string()];
    for e in entries {
        lines.push(
            [csv_field(&e.id), csv_number(e.lower), csv_number(e.upper), csv_field(&e.species), csv_field(&e.original_id), csv_field(&e.name)]
                .join(","),
        );
    }
    format!("{}\n", lines.join("\n"))
}

fn csv_field(text: &str) -> String {
    if text.contains(['"', ',', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

fn csv_number(value: f64) -> String {
    if value.is_finite() { js::number_to_string(round_precision(value, 12)) } else { String::new() }
}

/// A line of a bounds CSV as read: either side `None` leaves it as it is.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundsCsvRow {
    pub line: usize,
    pub id: String,
    pub species: String,
    pub original_id: String,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
}

/// A line that could not be read, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvError {
    pub line: usize,
    pub message: String,
}

/// What [`parse_bounds_csv`] read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedBoundsCsv {
    pub rows: Vec<BoundsCsvRow>,
    pub errors: Vec<CsvError>,
}

const CSV_ID: &[&str] = &["id", "reaction", "reaction_id", "rxn", "reactionid"];
const CSV_LOWER: &[&str] = &["lower", "lower_bound", "lb", "lowerbound", "min"];
const CSV_UPPER: &[&str] = &["upper", "upper_bound", "ub", "upperbound", "max"];
const CSV_SPECIES: &[&str] = &["species", "species_id", "organism", "cell"];
const CSV_ORIGINAL: &[&str] = &["original_id", "local_id", "model_id", "originalid"];

/// `parseBoundsCSV(text)`: read a bounds CSV (comma, semicolon or tab
/// separated; quoted fields allowed). The first line names the columns: an
/// id column and a lower and/or upper column, optionally species and
/// original_id. An empty bound leaves that side as it is.
pub fn parse_bounds_csv(text: &str) -> ParsedBoundsCsv {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.split("\r\n").flat_map(|l| l.split(['\n', '\r'])).collect();
    let mut out = ParsedBoundsCsv::default();
    let blank = |l: &str| l.trim().is_empty() || l.trim().starts_with('#');
    let Some(header_at) = lines.iter().position(|l| !blank(l)) else {
        out.errors.push(CsvError { line: 0, message: "The file is empty.".into() });
        return out;
    };
    let header_line = lines[header_at];
    let delimiter = if header_line.contains('\t') {
        '\t'
    } else if header_line.contains(';') && !header_line.contains(',') {
        ';'
    } else {
        ','
    };
    let header: Vec<String> = split_csv_line(header_line, delimiter)
        .iter()
        .map(|n| n.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("_"))
        .collect();
    let column = |aliases: &[&str]| header.iter().position(|n| aliases.contains(&n.as_str()));
    let (id_col, lower_col, upper_col) = (column(CSV_ID), column(CSV_LOWER), column(CSV_UPPER));
    let (species_col, original_col) = (column(CSV_SPECIES), column(CSV_ORIGINAL));
    if id_col.is_none() || (lower_col.is_none() && upper_col.is_none()) {
        out.errors.push(CsvError { line: header_at + 1, message: "The first line must name the columns: id, lower, upper.".into() });
        return out;
    }
    for (i, raw) in lines.iter().enumerate().skip(header_at + 1) {
        if blank(raw) {
            continue;
        }
        let line = i + 1;
        let fields = split_csv_line(raw, delimiter);
        let field = |col: Option<usize>| col.and_then(|c| fields.get(c)).map_or_else(String::new, |f| f.trim().to_string());
        let id = field(id_col);
        let original_id = field(original_col);
        if id.is_empty() && original_id.is_empty() {
            out.errors.push(CsvError { line, message: "No reaction id.".into() });
            continue;
        }
        let named = if id.is_empty() { original_id.clone() } else { id.clone() };
        let (lower, upper) = match (bound_value(&field(lower_col)), bound_value(&field(upper_col))) {
            (Ok(l), Ok(u)) => (l, u),
            _ => {
                out.errors.push(CsvError { line, message: format!("{named}: a bound is not a number.") });
                continue;
            }
        };
        if lower.is_none() && upper.is_none() {
            out.errors.push(CsvError { line, message: format!("{named}: no bound given.") });
            continue;
        }
        if let (Some(l), Some(u)) = (lower, upper) {
            if l > u {
                out.errors.push(CsvError { line, message: format!("{named}: the lower bound is above the upper bound.") });
                continue;
            }
        }
        out.rows.push(BoundsCsvRow { line, id, species: field(species_col), original_id, lower, upper });
    }
    out
}

/// A bound cell: `None` when empty, an error when it is not a finite number.
fn bound_value(text: &str) -> Result<Option<f64>, ()> {
    if text.is_empty() {
        return Ok(None);
    }
    let n = js::string_to_number(text);
    if n.is_finite() { Ok(Some(n)) } else { Err(()) }
}

fn split_csv_line(line: &str, delimiter: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' && chars.peek() == Some(&'"') {
                field.push('"');
                chars.next();
            } else if ch == '"' {
                quoted = false;
            } else {
                field.push(ch);
            }
        } else if ch == '"' {
            quoted = true;
        } else if ch == delimiter {
            out.push(std::mem::take(&mut field));
        } else {
            field.push(ch);
        }
    }
    out.push(field);
    out
}

/// Why a CSV line names no single reaction.
#[derive(Debug, Clone, PartialEq)]
pub enum Unresolved {
    Unknown,
    /// Several species have it: the merged ids it could be.
    Ambiguous(Vec<String>),
}

/// `resolveBoundsReaction(row, community)`: the merged reaction a CSV line
/// means: the merged id itself, or the original id with its species, or an
/// original id that only one species (or the community) has.
pub fn resolve_bounds_reaction(row: &BoundsCsvRow, community: &CommunityModel) -> Result<String, Unresolved> {
    let reactions = &community.merged.reactions;
    if !row.id.is_empty() && reactions.contains_key(&row.id) {
        return Ok(row.id.clone());
    }
    let local = if row.original_id.is_empty() { row.id.as_str() } else { row.original_id.as_str() };
    if !row.species.is_empty() {
        let id = format!("{}@{local}", row.species);
        return if reactions.contains_key(&id) { Ok(id) } else { Err(Unresolved::Unknown) };
    }
    if local.is_empty() || local.contains('@') {
        return Err(Unresolved::Unknown);
    }
    let mut candidates = Vec::new();
    if reactions.contains_key(local) {
        candidates.push(local.to_string());
    }
    for sp in community.active_species() {
        let id = format!("{}@{local}", sp.id);
        if reactions.contains_key(&id) {
            candidates.push(id);
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(Unresolved::Unknown),
        _ => Err(Unresolved::Ambiguous(candidates)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_log_slider_spans_a_thousandth_to_a_thousand_with_a_snap_to_zero() {
        let s = &SIGN_LOG;
        let steps = sign_log_steps(s);
        assert_eq!(steps, 242);
        assert_eq!(sign_log_value(0.0, s), 0.0);
        assert_eq!(sign_log_value(1.0, s), 0.0, "the positions next to 0 snap to 0");
        assert_eq!(sign_log_value(2.0, s), 0.001);
        assert_eq!(sign_log_value(-2.0, s), -0.001);
        assert_eq!(sign_log_value(steps as f64, s), 1000.0);
        assert_eq!(sign_log_value((-steps - 50) as f64, s), -1000.0);
        assert_eq!(sign_log_position(1e6, s), steps, "an unbounded 1e6 sits at the end");
        assert_eq!(sign_log_position(-1e-6, s), -2, "a tiny bound is not shown as 0");
        for v in [0.001, 0.0042, 0.37, 1.0, 12.5, -80.0, 999.0] {
            let back = sign_log_value(sign_log_position(v, s) as f64, s);
            assert!(((back - v) / v).abs() < 0.06, "{v} came back as {back}");
        }
        for p in -steps..steps {
            assert!(sign_log_value(p as f64, s) <= sign_log_value((p + 1) as f64, s));
        }
    }

    #[test]
    fn a_bounds_csv_is_read_with_its_errors_and_written_back() {
        let parsed = parse_bounds_csv("\u{feff}# note\nReaction;LB;UB\nR1;-1;2\nR2;;\n\"R;3\";x;1\nR4;5;1\n");
        assert_eq!(parsed.rows, vec![BoundsCsvRow { line: 3, id: "R1".into(), species: String::new(), original_id: String::new(), lower: Some(-1.0), upper: Some(2.0) }]);
        let messages: Vec<&str> = parsed.errors.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["R2: no bound given.", "R;3: a bound is not a number.", "R4: the lower bound is above the upper bound."]);
        assert_eq!(parse_bounds_csv("name,flux\n").errors[0].message, "The first line must name the columns: id, lower, upper.");
        let entry = BoundsCsvEntry { id: "a,b".into(), lower: -0.1 - 0.2, upper: f64::NAN, species: String::new(), original_id: "x".into(), name: "say \"hi\"".into() };
        assert_eq!(format_bounds_csv(&[entry]), "id,lower,upper,species,original_id,name\n\"a,b\",-0.3,,,x,\"say \"\"hi\"\"\"\n");
    }

    #[test]
    fn edits_to_the_same_control_in_quick_succession_are_one_undo_step() {
        let snap = |upper: f64| ConstraintSnapshot {
            overrides: vec![("R".into(), Bounds { lower: 0.0, upper })],
            medium: Vec::new(),
            objective: ObjectiveSnapshot { kind: None, product_target: None, custom: None },
        };
        let mut h = ConstraintHistory::default();
        h.record(&snap(1000.0), "Bounds of R", Some("bounds:R"), 0.0);
        h.record(&snap(5.0), "Bounds of R", Some("bounds:R"), 1000.0);
        h.record(&snap(3.0), "Bounds of S", Some("bounds:S"), 1200.0);
        h.record(&snap(2.0), "Bounds of S", Some("bounds:S"), 5000.0);
        assert_eq!(h.undo_len(), 3);
        h.undo(&snap(1.0));
        h.undo(&snap(2.0));
        let first = h.undo(&snap(3.0)).expect("the drag of R");
        assert_eq!(first.snapshot, snap(1000.0), "the whole drag is undone at once");
        assert!(!h.can_undo() && h.can_redo());
        assert_eq!(h.redo(&snap(1000.0)).map(|s| s.snapshot), Some(snap(3.0)));
        assert!(same_constraints(&snap(f64::NAN), &snap(f64::INFINITY)), "JSON writes both as null");
        assert_eq!(changed_reaction_ids(&snap(1.0), &snap(2.0), &|_| None), ["R"]);
    }
}
