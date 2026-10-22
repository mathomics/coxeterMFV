//! Reactions and bounds (the "Reactions and bounds", "Constraint history",
//! "Objective", "Bulk actions", "Bounds CSV" and "Reactions table" sections
//! of `store.js`): which reactions the medium sets, the bounds in force and
//! as opened, undo/redo of constraints apart from the map's, an objective
//! edited on top of the preset, bulk edits, bounds files, and the rows of
//! the Reactions page.
//!
//! The pure logic (types, sign-log scale, filters, history, CSV) is in
//! [`crate::reactions_explorer`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use coxeter_core::community::community_exchange;
use coxeter_core::compartments::cell_compartment_name;
use coxeter_core::js;
use coxeter_solver::fba::{FluxResult, default_objective};
use coxeter_solver::lp::LpStatus;
use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

use crate::error::{Result, StoreError};
use crate::reactions_explorer::*;
use crate::store::Store;
use crate::types::*;

// State

/// The side of an exchange's bounds the medium owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediumSide {
    /// The community exchange: the lower bound is the supply.
    Lower,
    /// A joint community's uptake reaction: the upper bound is the supply.
    Upper,
}

/// A reaction whose bounds the medium sets: its medium metabolite and side.
#[derive(Debug, Clone, PartialEq)]
pub struct MediumEntry {
    pub base: String,
    pub side: MediumSide,
}

/// `mediumReactions()`: per reaction id its medium metabolite and side,
/// `by_base` the other way, and `transfer_base` the metabolite each
/// species' exchange moves.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediumReactions {
    pub by_reaction: IndexMap<String, MediumEntry>,
    pub by_base: IndexMap<String, String>,
    pub transfer_base: IndexMap<String, String>,
}

/// `{ ids, label, status }`: the reactions flagged once a change left the
/// LP without an optimum, the change, and the status it left.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintCulprits {
    pub ids: IndexSet<String>,
    pub label: String,
    pub status: String,
}

/// What the next solve compares against (`_culpritCheck`).
#[derive(Debug, Clone)]
pub(crate) struct CulpritCheck {
    before: ConstraintSnapshot,
    label: String,
}

/// `store.reactionTable`: how the Reactions page lists rows, and the rows
/// ticked for bulk actions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReactionTableState {
    pub query: String,
    pub filters: Vec<ReactionFilter>,
    pub group_by: Grouping,
    pub sort: SortSpec,
    pub collapsed: HashSet<String>,
    pub selection: IndexSet<String>,
    /// The Genes control's search.
    pub gene_query: String,
}

/// A change to [`ReactionTableState`] (`setReactionTable(patch)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReactionTablePatch {
    pub query: Option<String>,
    pub group_by: Option<Grouping>,
    pub filters: Option<Vec<ReactionFilter>>,
    /// One quick filter on or off.
    pub toggle_filter: Option<ReactionFilter>,
    /// A column; the same one again reverses it.
    pub sort_by: Option<SortKey>,
    /// Fold or unfold a group.
    pub toggle_group: Option<String>,
    /// The Genes control's search.
    pub gene_query: Option<String>,
}

/// One reaction whose bounds differ from the ones it was opened with.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintChange {
    pub id: String,
    pub label: String,
    pub species_id: Option<String>,
    pub current: Bounds,
    pub original: Bounds,
    pub knocked_out: bool,
    /// The medium metabolite, when the change is to its supply.
    pub medium: Option<String>,
    /// The knocked-out genes that close it.
    pub genes: Vec<String>,
}

/// One action on many reactions (`bulkReactionAction`).
#[derive(Debug, Clone, PartialEq)]
pub enum BulkAction {
    /// An empty side is left as it is.
    SetBounds { lower: UserNumber, upper: UserNumber },
    /// Finite bounds are multiplied; ±1000 (unbounded by convention) stays.
    Scale(UserNumber),
    Knockout,
    /// Back to the bounds as opened.
    Reset,
    /// Every exchange's uptake closed except those given (an
    /// uptake/secretion row keeps its metabolite open).
    CloseUptakesExcept,
}

/// What a bulk action did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BulkOutcome {
    pub changed: usize,
    pub skipped: usize,
}

/// What a bounds CSV import did (`importBoundsCSV`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoundsImport {
    pub applied: usize,
    pub unchanged: usize,
    pub unknown: Vec<String>,
    pub ambiguous: Vec<String>,
    pub errors: Vec<CsvError>,
    pub rows: usize,
}

/// Whether the FVA and reduced-cost columns have anything to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableColumns {
    pub fva: bool,
    pub rc: bool,
}

/// `reactionTableModel()`: the table as the Reactions page lists it.
#[derive(Debug, Clone, Default)]
pub struct ReactionTableModel {
    pub rows: Vec<ReactionRow>,
    /// Filtered and sorted.
    pub visible: Vec<ReactionRow>,
    pub items: Vec<TableItem>,
    pub total: usize,
    pub shown: usize,
    pub modified: usize,
    pub selected: usize,
    pub columns: TableColumns,
}

/// Per built problem: which reactions the medium controls, and the table's
/// catalogue (the JS keeps both in WeakMaps on the merged model).
#[derive(Default)]
pub(crate) struct ReactionCaches {
    medium: Mutex<Option<(u64, Arc<MediumReactions>)>>,
    catalog: Mutex<Option<(u64, String, Arc<Vec<Arc<CatalogEntry>>>)>>,
}

/// How far apart two bounds may be and still be the same.
const EPSILON: f64 = 1e-9;

fn same(a: &Bounds, b: &Bounds) -> bool {
    same_bounds(Some(a), Some(b))
}

/// A result solved as the maximum of the negated objective, read back as the
/// minimum of the objective: the objective value and every dual change sign.
pub fn minimised_result(mut res: FluxResult) -> FluxResult {
    if res.objective_value.is_finite() {
        res.objective_value = -res.objective_value;
    }
    for map in [&mut res.reduced_costs, &mut res.scaled_reduced_costs, &mut res.shadow_prices] {
        for v in map.values_mut() {
            *v = -*v;
        }
    }
    res
}

impl Store {
    // Medium and bounds

    /// `mediumReactions()`: the reactions whose bounds the medium sets. That
    /// is the community exchange of each medium metabolite (the lower bound
    /// is the supply) or, in a joint community, its uptake reaction (the
    /// upper bound is the supply).
    pub fn medium_reactions(&self) -> Arc<MediumReactions> {
        let Some(community) = self.community.as_ref().filter(|_| !self.is_zuniga_study()) else {
            return Arc::new(MediumReactions::default());
        };
        let mut cache = self.reaction_caches.medium.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, value)) = cache.as_ref().filter(|(serial, _)| *serial == self.problem_serial) {
            return value.clone();
        }
        let merged = &community.merged;
        let mut value = MediumReactions::default();
        for (base, ids) in self.community_exchange_ids.iter().flatten() {
            if ids.uptake.is_empty() || !merged.reactions.contains_key(&ids.uptake) {
                continue;
            }
            value.by_reaction.insert(ids.uptake.clone(), MediumEntry { base: base.clone(), side: MediumSide::Upper });
            value.by_base.insert(base.clone(), ids.uptake.clone());
        }
        for (base, transfers) in &community.transfers_by_base {
            let id = community_exchange(base);
            if !value.by_base.contains_key(base) && merged.reactions.contains_key(&id) {
                value.by_reaction.insert(id.clone(), MediumEntry { base: base.clone(), side: MediumSide::Lower });
                value.by_base.insert(base.clone(), id);
            }
            for transfer in transfers {
                value.transfer_base.insert(transfer.reaction_id.clone(), base.clone());
            }
        }
        let value = Arc::new(value);
        *cache = Some((self.problem_serial, value.clone()));
        value
    }

    /// `mediumReactionBounds(rid, medium, { absentClosed })`: the bounds
    /// `medium` gives a reaction the medium controls, as `apply_overrides`
    /// sets them, or `None` for any other reaction. A metabolite the medium
    /// does not list keeps the bounds of the last rebuild, or, with
    /// `absent_closed`, is closed to uptake as a freshly built community has it.
    pub fn medium_reaction_bounds(&self, rid: &str, medium: &IndexMap<String, f64>, absent_closed: bool) -> Option<Bounds> {
        let media = self.medium_reactions();
        let entry = media.by_reaction.get(rid)?;
        let defaults = self.default_reaction_bounds(rid);
        let listed = medium.get(&entry.base);
        let limit = listed.map_or(0.0, |v| if v.is_nan() { 0.0 } else { v.abs() });
        Some(match (entry.side, listed.is_some()) {
            (MediumSide::Upper, true) => Bounds { lower: defaults.lower, upper: limit },
            (MediumSide::Upper, false) => defaults,
            (MediumSide::Lower, true) => Bounds { lower: -limit, upper: 1000.0 },
            (MediumSide::Lower, false) if absent_closed => Bounds { lower: 0.0, upper: 1000.0 },
            (MediumSide::Lower, false) => defaults,
        })
    }

    /// `originalReactionBounds(rid)`: the bounds a reaction had when the
    /// scenario was opened: the model's, with the medium it was opened with.
    pub fn original_reaction_bounds(&self, rid: &str) -> Bounds {
        self.medium_reaction_bounds(rid, &self.base_medium, true).unwrap_or_else(|| self.default_reaction_bounds(rid))
    }

    /// `reactionKnockedOut(rid)`: whether the user closed a reaction (both
    /// bounds 0) that was open as opened.
    pub fn reaction_knocked_out(&self, rid: &str) -> bool {
        is_knocked_out(&self.current_reaction_bounds(rid), &self.original_reaction_bounds(rid))
    }

    /// `_writeReactionBounds(rid, next)`: store a reaction's new bounds where
    /// they belong. An exchange the medium controls takes its supply from the
    /// medium, so the supply goes there; what the medium cannot say (a cap on
    /// secretion, a forced flux) stays an override on top of it. Everything
    /// else is an override. No history, no solve.
    pub(crate) fn write_reaction_bounds(&mut self, rid: &str, next: Bounds) {
        let media = self.medium_reactions();
        let Some(entry) = media.by_reaction.get(rid) else {
            self.overrides.insert(rid.to_string(), next);
            return;
        };
        let listed = self.medium.contains_key(&entry.base);
        let defaults = self.default_reaction_bounds(rid);
        if entry.side == MediumSide::Upper {
            if listed || (next.upper - defaults.upper).abs() > EPSILON {
                self.medium.insert(entry.base.clone(), js::max(0.0, next.upper));
            }
            if (next.lower - defaults.lower).abs() <= EPSILON {
                self.overrides.shift_remove(rid);
            } else {
                self.overrides.insert(rid.to_string(), next);
            }
            return;
        }
        let supply = js::max(0.0, -next.lower);
        if listed || supply > 0.0 {
            self.medium.insert(entry.base.clone(), supply);
        }
        if (next.upper - 1000.0).abs() <= EPSILON && next.lower <= 0.0 {
            self.overrides.shift_remove(rid);
        } else {
            self.overrides.insert(rid.to_string(), next);
        }
    }

    /// `_resetReactionBounds(rid)`: one reaction's bounds back as opened, its
    /// medium supply included. No history, no solve.
    pub(crate) fn reset_reaction_bounds(&mut self, rid: &str) {
        self.overrides.shift_remove(rid);
        let media = self.medium_reactions();
        let Some(entry) = media.by_reaction.get(rid) else { return };
        if let Some(v) = self.base_medium.get(&entry.base) {
            self.medium.insert(entry.base.clone(), *v);
        } else if self.medium.contains_key(&entry.base) {
            self.medium.insert(entry.base.clone(), 0.0);
        }
    }

    /// `reactionTitle(rid)`: a reaction as history entries and messages name
    /// it: its model id, and its species when there are several.
    pub fn reaction_title(&self, rid: &str) -> String {
        let owner = self.community.as_ref().and_then(|c| c.reaction_owner.get(rid));
        let id = owner.map_or(rid, |o| o.original_id.as_str());
        match owner.and_then(|o| o.species_id.as_deref()) {
            Some(sp) if self.species.len() > 1 => format!("{id} ({sp})"),
            _ => id.to_string(),
        }
    }

    /// `reactionHeldOut(rid)`: whether a reaction belongs to a species held
    /// out of the simulation, whose bounds cannot be edited now.
    pub fn reaction_held_out(&self, rid: &str) -> bool {
        self.community
            .as_ref()
            .and_then(|c| c.reaction_owner.get(rid))
            .and_then(|o| o.species_id.as_deref())
            .is_some_and(|sp| self.species_held_out(sp))
    }

    /// `constraintChanges()`: every reaction whose bounds differ from the
    /// ones it was opened with, by label, with the knocked-out genes that
    /// close it.
    pub fn constraint_changes(&self) -> Vec<ConstraintChange> {
        let Some(community) = self.community.as_ref() else { return Vec::new() };
        let media = self.medium_reactions();
        let genes_closing = self.gene_closed_reactions();
        let mut ids: IndexSet<&str> = self.overrides.keys().map(String::as_str).collect();
        ids.extend(genes_closing.keys().map(String::as_str));
        for (rid, entry) in &media.by_reaction {
            if self.medium.contains_key(&entry.base) || self.base_medium.contains_key(&entry.base) {
                ids.insert(rid);
            }
        }
        let mut changes = Vec::new();
        for id in ids {
            if !community.merged.reactions.contains_key(id) {
                continue;
            }
            let current = self.current_reaction_bounds(id);
            let original = self.original_reaction_bounds(id);
            if same(&current, &original) {
                continue;
            }
            changes.push(ConstraintChange {
                id: id.to_string(),
                label: self.entity_label("reaction", id).label,
                species_id: community.reaction_owner.get(id).and_then(|o| o.species_id.clone()),
                current,
                original,
                knocked_out: is_knocked_out(&current, &original),
                medium: media.by_reaction.get(id).map(|e| e.base.clone()),
                genes: genes_closing.get(id).cloned().unwrap_or_default(),
            });
        }
        changes.sort_by(|a, b| js::locale_compare(&a.label, &b.label).then_with(|| js::locale_compare(&a.id, &b.id)));
        changes
    }

    // Constraint history

    /// `constraintSnapshot()`: what the LP is asked, as plain data in a fixed
    /// order: overrides, gene knockouts, medium and objective.
    pub fn constraint_snapshot(&self) -> ConstraintSnapshot {
        let mut overrides: Vec<(String, Bounds)> = self.overrides.iter().map(|(k, b)| (k.clone(), *b)).collect();
        overrides.sort_by(|a, b| js::code_unit_compare(&a.0, &b.0));
        let mut medium: Vec<(String, f64)> = self.medium.iter().map(|(k, v)| (k.clone(), *v)).collect();
        medium.sort_by(|a, b| js::code_unit_compare(&a.0, &b.0));
        let custom = self.custom_objective.as_ref().map(|c| {
            let mut coefficients: Vec<(String, f64)> = c.coefficients.iter().map(|(k, v)| (k.clone(), *v)).collect();
            coefficients.sort_by(|a, b| js::code_unit_compare(&a.0, &b.0));
            CustomObjective { sense: c.sense, coefficients: coefficients.into_iter().collect() }
        });
        ConstraintSnapshot {
            overrides,
            gene_knockouts: self.sorted_gene_knockouts(),
            medium,
            objective: ObjectiveSnapshot { kind: Some(self.objective.clone()), product_target: self.product_target.clone(), custom },
        }
    }

    /// `_changeConstraints(label, key, mutate)`: run `mutate` as one
    /// constraint edit: if it changed anything, the state before it goes on
    /// the undo stack under `label`. Edits with the same `key` in quick
    /// succession (a slider drag) are one entry.
    pub(crate) fn change_constraints<R>(&mut self, label: &str, key: Option<&str>, mutate: impl FnOnce(&mut Store) -> R) -> R {
        let before = self.constraint_snapshot();
        let result = mutate(self);
        if !same_constraints(&before, &self.constraint_snapshot()) {
            let entry = self.constraint_history.record(&before, label, key, now_ms());
            let before = self.culprit_check.take().map_or(entry, |c| c.before);
            self.culprit_check = Some(CulpritCheck { before, label: label.to_string() });
        }
        result
    }

    pub fn can_undo_constraints(&self) -> bool {
        self.constraint_history.can_undo()
    }

    pub fn can_redo_constraints(&self) -> bool {
        self.constraint_history.can_redo()
    }

    /// `undoConstraints()`: take back the last constraint edit and solve.
    /// The label of what was undone, or `None` when there was nothing.
    pub fn undo_constraints(&mut self) -> Result<Option<String>> {
        self.assert_published_study_constraints()?;
        let current = self.constraint_snapshot();
        let Some(step) = self.constraint_history.undo(&current) else { return Ok(None) };
        self.restore_constraint_snapshot(&step.snapshot)?;
        self.report_constraint_step(&format!("Undone: {}", step.label));
        Ok(Some(step.label))
    }

    /// `redoConstraints()`: apply again the last constraint edit undone, and solve.
    pub fn redo_constraints(&mut self) -> Result<Option<String>> {
        self.assert_published_study_constraints()?;
        let current = self.constraint_snapshot();
        let Some(step) = self.constraint_history.redo(&current) else { return Ok(None) };
        self.restore_constraint_snapshot(&step.snapshot)?;
        self.report_constraint_step(&format!("Redone: {}", step.label));
        Ok(Some(step.label))
    }

    fn report_constraint_step(&mut self, message: &str) {
        self.add_log(StatusState::Ok, message, LogKind::Message);
        if self.solution.as_ref().is_some_and(|s| s.is_optimal()) {
            let status = self.status_mut();
            status.state = StatusState::Ok;
            status.message = message.to_string();
        }
        self.emit("constraints");
    }

    /// `restoreConstraintSnapshot(snapshot)`: make a snapshot the constraints
    /// in force and solve. A different preset objective changes the
    /// community's structure, so that rebuilds.
    pub fn restore_constraint_snapshot(&mut self, snapshot: &ConstraintSnapshot) -> Result<()> {
        self.invalidate_steady_com();
        let objective = &snapshot.objective;
        let structural = objective.kind.as_deref() != Some(self.objective.as_str()) || objective.product_target != self.product_target;
        self.overrides = snapshot.overrides.iter().cloned().collect();
        self.gene_knockouts = snapshot.gene_knockouts.iter().cloned().collect();
        self.medium = snapshot.medium.iter().cloned().collect();
        if let Some(kind) = &objective.kind {
            self.objective = kind.clone();
        }
        self.product_target = objective.product_target.clone();
        self.custom_objective = objective.custom.clone();
        // What is flagged belonged to the state being left.
        self.constraint_culprits = None;
        self.culprit_check = None;
        if structural {
            return self.rebuild(RebuildOptions { relayout: false, ..RebuildOptions::default() });
        }
        self.apply_overrides();
        self.solve_with(None, true);
        Ok(())
    }

    /// `noteConstraintOutcome(res)`: after a solve, an optimum clears the
    /// flagged culprits; no optimum right after a constraint edit flags the
    /// reactions that edit touched, which is what "Revert last change" offers
    /// to undo. Edits made while the LP already had no optimum join the
    /// flagged ones, and the message keeps naming the edit that lost it.
    pub(crate) fn note_constraint_outcome(&mut self, status: LpStatus) {
        let check = self.culprit_check.take();
        if status == LpStatus::Optimal {
            self.constraint_culprits = None;
            return;
        }
        let Some(check) = check else { return };
        let media = self.medium_reactions();
        let ids = changed_reaction_ids(&check.before, &self.constraint_snapshot(), &|base| media.by_base.get(base).cloned());
        let earlier = self.constraint_culprits.take();
        let mut all = earlier.as_ref().map(|e| e.ids.clone()).unwrap_or_default();
        all.extend(ids);
        self.constraint_culprits =
            Some(ConstraintCulprits { ids: all, label: earlier.map_or(check.label, |e| e.label), status: status.as_str().to_string() });
    }

    // Objective

    /// `presetObjectiveCoefficients()`: the coefficients of the preset
    /// objective, as the LP was built with them.
    pub fn preset_objective_coefficients(&self) -> IndexMap<String, f64> {
        if self.is_zuniga_study() {
            return self.problem.as_ref().map(|p| p.objective().clone()).unwrap_or_default();
        }
        self.community.as_ref().map(|c| default_objective(&c.merged)).unwrap_or_default()
    }

    /// `objectiveCoefficients()`: the objective in force, the edited one or
    /// the preset's.
    pub fn objective_coefficients(&self) -> IndexMap<String, f64> {
        match &self.custom_objective {
            Some(c) => c.coefficients.clone(),
            None => self.preset_objective_coefficients(),
        }
    }

    /// `objectiveSense()`: presets maximise.
    pub fn objective_sense(&self) -> ObjectiveSense {
        self.custom_objective.as_ref().map_or(ObjectiveSense::Max, |c| c.sense)
    }

    /// `applyObjective()`: give the LP the objective in force. A minimised
    /// objective goes in negated, so every analysis (FBA, pFBA, FVA,
    /// loopless) keeps maximising; the solve turns the signs of its result
    /// back (see [`minimised_result`]).
    pub(crate) fn apply_objective(&mut self) {
        if self.problem.is_none() || self.is_zuniga_study() {
            return;
        }
        let sign = if self.objective_sense() == ObjectiveSense::Min { -1.0 } else { 1.0 };
        let coefficients: IndexMap<String, f64> = self.objective_coefficients().into_iter().map(|(id, c)| (id, sign * c)).collect();
        if let Some(problem) = self.problem_mut() {
            problem.set_objective(&coefficients);
        }
    }

    /// Install an edited objective; one equal to the preset, maximised, is
    /// the preset again.
    fn set_custom_objective(&mut self, sense: ObjectiveSense, coefficients: IndexMap<String, f64>) {
        let preset = self.preset_objective_coefficients();
        let keys: IndexSet<&String> = preset.keys().chain(coefficients.keys()).collect();
        let same = sense == ObjectiveSense::Max
            && keys.iter().all(|id| preset.get(*id).copied().unwrap_or(0.0) == coefficients.get(*id).copied().unwrap_or(0.0));
        self.custom_objective = if same { None } else { Some(CustomObjective { sense, coefficients }) };
    }

    /// `setObjectiveCoefficient(rid, value)`: one reaction's objective
    /// coefficient (0 takes it out), then a solve. Any reaction, any number
    /// of them.
    pub fn set_objective_coefficient(&mut self, rid: &str, value: impl Into<UserNumber>) -> Result<Option<Arc<Solution>>> {
        self.assert_published_study_constraints()?;
        let Some(coefficient) = finite_input(&value.into()) else { return Ok(None) };
        if !self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(rid)) {
            return Ok(None);
        }
        let mut coefficients = self.objective_coefficients();
        if coefficients.get(rid).copied().unwrap_or(0.0) == coefficient {
            return Ok(self.solution.clone());
        }
        self.invalidate_steady_com();
        let label = format!("Objective coefficient of {}", self.reaction_title(rid));
        self.change_constraints(&label, Some(&format!("objective:{rid}")), |s| {
            if coefficient == 0.0 {
                coefficients.shift_remove(rid);
            } else {
                coefficients.insert(rid.to_string(), coefficient);
            }
            let sense = s.objective_sense();
            s.set_custom_objective(sense, coefficients);
        });
        self.apply_overrides();
        Ok(self.solve())
    }

    /// `setObjectiveSense(sense)`: maximise or minimise the objective in
    /// force, and solve.
    pub fn set_objective_sense(&mut self, sense: ObjectiveSense) -> Result<Option<Arc<Solution>>> {
        self.assert_published_study_constraints()?;
        if sense == self.objective_sense() {
            return Ok(self.solution.clone());
        }
        self.invalidate_steady_com();
        let label = if sense == ObjectiveSense::Min { "Minimise the objective" } else { "Maximise the objective" };
        self.change_constraints(label, None, |s| {
            let coefficients = s.objective_coefficients();
            s.set_custom_objective(sense, coefficients);
        });
        self.apply_overrides();
        Ok(self.solve())
    }

    /// `resetObjective()`: back to the preset's own objective, maximised,
    /// and solve.
    pub fn reset_objective(&mut self) -> Result<Option<Arc<Solution>>> {
        self.assert_published_study_constraints()?;
        if self.custom_objective.is_none() {
            return Ok(self.solution.clone());
        }
        self.invalidate_steady_com();
        self.change_constraints("Objective back to the preset", None, |s| s.custom_objective = None);
        self.apply_overrides();
        Ok(self.solve())
    }

    /// `customObjectiveFrom(raw)`: an edited objective read from a file:
    /// finite, non-zero coefficients of reactions this community has, and a
    /// sense. `None` when the file has none, or when every reaction it names
    /// is missing here (the preset then stands).
    pub fn custom_objective_from(&mut self, raw: Option<&Value>) -> Option<CustomObjective> {
        let raw = raw.filter(|v| v.is_object())?;
        let mut coefficients = IndexMap::new();
        let mut dropped = 0;
        if let Some(Value::Object(entries)) = raw.get("coefficients") {
            for (id, value) in entries {
                let coefficient = js::to_number(value);
                let known = self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(id));
                if !known || !coefficient.is_finite() {
                    dropped += 1;
                    continue;
                }
                if coefficient != 0.0 {
                    coefficients.insert(id.clone(), coefficient);
                }
            }
        }
        if dropped > 0 {
            self.add_log(
                StatusState::Warn,
                &format!("{dropped} objective coefficient(s) in the file name reactions this model does not have; they were left out."),
                LogKind::Message,
            );
            if coefficients.is_empty() {
                return None;
            }
        }
        let sense = if raw.get("sense").and_then(Value::as_str) == Some("min") { ObjectiveSense::Min } else { ObjectiveSense::Max };
        Some(CustomObjective { sense, coefficients })
    }

    /// `objectiveDescription()`: `max 1 COMMUNITY_GROWTH`, `min 1 EX_glc__D_s + …`.
    pub fn objective_description(&self) -> String {
        let terms: Vec<String> = self
            .objective_coefficients()
            .iter()
            .map(|(id, c)| format!("{} {}", js::number_to_string(round_precision(*c, 6)), self.reaction_title(id)))
            .collect();
        let terms = if terms.is_empty() { "0".to_string() } else { terms.join(" + ") };
        format!("{} {terms}", self.objective_sense().as_str())
    }

    // Bulk actions

    /// `bulkReactionAction(action, params, ids)`: one action on many
    /// reactions (by default the ticked rows), as one undoable edit, then a
    /// solve.
    pub fn bulk_reaction_action(&mut self, action: BulkAction, ids: Option<Vec<String>>) -> Result<BulkOutcome> {
        self.assert_published_study_constraints()?;
        let Some(community) = self.community.clone() else { return Ok(BulkOutcome::default()) };
        let ids = ids.unwrap_or_else(|| self.reaction_table.selection.iter().cloned().collect());
        // A reaction a gene knockout closes stays closed whatever its bounds say.
        let gene_closed = self.gene_closed_reactions();
        let targets: Vec<String> = ids
            .into_iter()
            .filter(|id| community.merged.reactions.contains_key(id) && !self.reaction_held_out(id) && !gene_closed.contains_key(id))
            .collect();
        let n = targets.len();
        let mut outcome = BulkOutcome::default();
        // A bound pair applied unless it would cross or changes nothing.
        let write = |s: &mut Store, outcome: &mut BulkOutcome, id: &str, next: Bounds| {
            if next.lower > next.upper {
                outcome.skipped += 1;
                return;
            }
            if same(&next, &s.current_reaction_bounds(id)) {
                return;
            }
            s.write_reaction_bounds(id, next);
            outcome.changed += 1;
        };
        let label = match &action {
            BulkAction::SetBounds { lower, upper } => {
                if finite_input(lower).is_none() && finite_input(upper).is_none() {
                    return Ok(outcome);
                }
                format!("Set bounds of {n} reactions")
            }
            BulkAction::Scale(factor) => {
                let Some(factor) = finite_input(factor) else { return Ok(outcome) };
                format!("Scale bounds of {n} reactions by {}", js::number_to_string(factor))
            }
            BulkAction::Knockout => format!("Knock out {n} reactions"),
            BulkAction::Reset => format!("Reset {n} reactions"),
            BulkAction::CloseUptakesExcept => format!("Close all uptakes except {n} selected"),
        };
        self.invalidate_steady_com();
        let edit_label = label.clone();
        self.change_constraints(&edit_label, None, |s| match &action {
            BulkAction::SetBounds { lower, upper } => {
                let (lower, upper) = (finite_input(lower), finite_input(upper));
                for id in &targets {
                    let current = s.current_reaction_bounds(id);
                    write(s, &mut outcome, id, Bounds { lower: lower.unwrap_or(current.lower), upper: upper.unwrap_or(current.upper) });
                }
            }
            BulkAction::Scale(factor) => {
                let factor = finite_input(factor).unwrap_or(1.0);
                // 1000 is "unbounded" in COBRA models: scaling it would invent a limit.
                let scale = |v: f64| if v.abs() >= 1000.0 { v } else { v * factor };
                for id in &targets {
                    let current = s.current_reaction_bounds(id);
                    let (a, b) = (scale(current.lower), scale(current.upper));
                    write(s, &mut outcome, id, Bounds { lower: js::min(a, b), upper: js::max(a, b) });
                }
            }
            BulkAction::Knockout => {
                for id in &targets {
                    write(s, &mut outcome, id, Bounds { lower: 0.0, upper: 0.0 });
                }
            }
            BulkAction::Reset => {
                for id in &targets {
                    if same(&s.current_reaction_bounds(id), &s.original_reaction_bounds(id)) {
                        continue;
                    }
                    s.reset_reaction_bounds(id);
                    outcome.changed += 1;
                }
            }
            BulkAction::CloseUptakesExcept => {
                let media = s.medium_reactions();
                let keep: HashSet<&String> = targets.iter().collect();
                let keep_bases: HashSet<String> = targets
                    .iter()
                    .filter_map(|id| media.by_reaction.get(id).map(|e| e.base.clone()).or_else(|| media.transfer_base.get(id).cloned()))
                    .collect();
                for id in community.merged.reactions.keys() {
                    if keep.contains(id) || s.reaction_held_out(id) || gene_closed.contains_key(id) {
                        continue;
                    }
                    let current = s.current_reaction_bounds(id);
                    if let Some(entry) = media.by_reaction.get(id) {
                        if keep_bases.contains(&entry.base) {
                            continue;
                        }
                        let next = match entry.side {
                            MediumSide::Upper => Bounds { lower: js::min(current.lower, 0.0), upper: 0.0 },
                            MediumSide::Lower => Bounds { lower: 0.0, upper: js::max(current.upper, 0.0) },
                        };
                        write(s, &mut outcome, id, next);
                    } else if current.lower < 0.0 && classify_reaction(&community, id) == ReactionType::Exchange {
                        write(s, &mut outcome, id, Bounds { lower: 0.0, upper: js::max(current.upper, 0.0) });
                    }
                }
            }
        });
        if outcome.changed > 0 {
            self.apply_overrides();
            self.solve_with(None, true);
        }
        // A cleared solution is not a lost optimum.
        let lost = outcome.changed > 0 && self.solution.as_ref().is_some_and(|s| !s.is_optimal());
        let mut status = self.status().clone();
        if !lost {
            status.state = if outcome.skipped > 0 { StatusState::Warn } else { StatusState::Ok };
        }
        let skipped = if outcome.skipped > 0 {
            format!(", {} skipped (the lower bound would pass the upper)", outcome.skipped)
        } else {
            String::new()
        };
        let failed = if lost {
            format!(" The LP has no optimum now ({}); Revert last change takes it back.", self.solution_status())
        } else {
            String::new()
        };
        status.message = format!("{label}: {} changed{skipped}.{failed}", outcome.changed);
        self.set_status(status);
        self.emit("constraints");
        Ok(outcome)
    }

    fn solution_status(&self) -> String {
        self.solution.as_ref().map_or_else(|| "undefined".to_string(), |s| s.status.as_str().to_string())
    }

    // Bounds CSV

    /// `boundsCSV(ids)`: every reaction's bounds in force as CSV (see
    /// [`format_bounds_csv`]), or only `ids`.
    pub fn bounds_csv(&self, ids: Option<&[String]>) -> String {
        let Some(community) = self.community.as_ref() else { return format_bounds_csv(&[]) };
        let merged = &community.merged;
        let all: Vec<&String>;
        let ids: Vec<&String> = match ids {
            Some(ids) => ids.iter().collect(),
            None => {
                all = merged.reactions.keys().collect();
                all
            }
        };
        let entries: Vec<BoundsCsvEntry> = ids
            .into_iter()
            .filter_map(|id| merged.reactions.get(id).map(|r| (id, r)))
            .map(|(id, reaction)| {
                let owner = community.reaction_owner.get(id);
                let bounds = self.current_reaction_bounds(id);
                BoundsCsvEntry {
                    id: id.clone(),
                    lower: bounds.lower,
                    upper: bounds.upper,
                    species: owner.and_then(|o| o.species_id.clone()).unwrap_or_default(),
                    original_id: owner.map_or_else(|| id.clone(), |o| o.original_id.clone()),
                    name: reaction.name.clone(),
                }
            })
            .collect();
        format_bounds_csv(&entries)
    }

    /// `importBoundsCSV(text, { name })`: apply a bounds CSV as one undoable
    /// edit and solve. Lines are matched by merged id, or by original id with
    /// species, or by an original id only one species has. Unknown and
    /// ambiguous ids and unreadable lines are reported, never dropped
    /// silently; a line giving the bounds as opened resets them.
    pub fn import_bounds_csv(&mut self, text: &str, name: &str) -> Result<BoundsImport> {
        self.assert_published_study_constraints()?;
        let Some(community) = self.community.clone() else {
            return Err(StoreError::msg("Load a model before importing bounds."));
        };
        let parsed = parse_bounds_csv(text);
        let mut report = BoundsImport { errors: parsed.errors.clone(), rows: parsed.rows.len(), ..BoundsImport::default() };
        let mut changes: Vec<(String, Bounds, Bounds)> = Vec::new();
        for row in &parsed.rows {
            let found = resolve_bounds_reaction(row, &community);
            let as_written = if row.species.is_empty() {
                if row.id.is_empty() { row.original_id.clone() } else { row.id.clone() }
            } else {
                format!("{}@{}", row.species, if row.original_id.is_empty() { &row.id } else { &row.original_id })
            };
            let id = match found {
                Ok(id) => id,
                Err(Unresolved::Ambiguous(_)) => {
                    report.ambiguous.push(as_written);
                    continue;
                }
                Err(Unresolved::Unknown) => {
                    report.unknown.push(as_written);
                    continue;
                }
            };
            if self.reaction_held_out(&id) {
                report.errors.push(CsvError { line: row.line, message: format!("{id}: its species is held out of the simulation.") });
                continue;
            }
            let current = self.current_reaction_bounds(&id);
            let next = Bounds { lower: row.lower.unwrap_or(current.lower), upper: row.upper.unwrap_or(current.upper) };
            if next.lower > next.upper {
                report.errors.push(CsvError { line: row.line, message: format!("{id}: the lower bound would pass the upper one.") });
                continue;
            }
            changes.push((id, next, current));
        }
        report.unchanged = changes.iter().filter(|(_, next, current)| same(next, current)).count();
        if changes.len() > report.unchanged {
            self.invalidate_steady_com();
            let mut applied = 0;
            self.change_constraints(&format!("Import bounds from {name}"), None, |s| {
                for (id, next, current) in &changes {
                    if same(next, current) {
                        continue;
                    }
                    if same(next, &s.original_reaction_bounds(id)) {
                        s.reset_reaction_bounds(id);
                    } else {
                        s.write_reaction_bounds(id, *next);
                    }
                    applied += 1;
                }
            });
            report.applied = applied;
            self.apply_overrides();
            self.solve_with(None, true);
        }
        let list = |ids: &[String]| format!("{}{}", ids.iter().take(5).cloned().collect::<Vec<_>>().join(", "), if ids.len() > 5 { ", …" } else { "" });
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let mut parts = vec![format!("{} changed", plural(report.applied, "reaction")), format!("{} already as given", report.unchanged)];
        if !report.unknown.is_empty() {
            parts.push(format!("{} ({})", plural(report.unknown.len(), "unknown id"), list(&report.unknown)));
        }
        if !report.ambiguous.is_empty() {
            parts.push(format!("{} ambiguous, give the species ({})", report.ambiguous.len(), list(&report.ambiguous)));
        }
        if let Some(first) = report.errors.first() {
            parts.push(format!("{} not read (line {}: {})", plural(report.errors.len(), "line"), first.line, first.message));
        }
        if report.unknown.len() > 5 {
            self.add_log(StatusState::Warn, &format!("Bounds import from {name}: unknown reaction ids {}", report.unknown.join(", ")), LogKind::Message);
        }
        if report.ambiguous.len() > 5 {
            self.add_log(StatusState::Warn, &format!("Bounds import from {name}: ambiguous reaction ids {}", report.ambiguous.join(", ")), LogKind::Message);
        }
        let failed = report.applied > 0 && self.solution.as_ref().is_some_and(|s| !s.is_optimal());
        let state = if failed {
            StatusState::Error
        } else if !report.unknown.is_empty() || !report.ambiguous.is_empty() || !report.errors.is_empty() {
            StatusState::Warn
        } else {
            StatusState::Ok
        };
        let tail = if failed { format!(" The LP has no optimum now ({}); Revert last change takes it back.", self.solution_status()) } else { String::new() };
        self.set_status_message(state, format!("Bounds from {name}: {}.{tail}", parts.join(" · ")));
        self.emit("constraints");
        Ok(report)
    }

    // Reactions table

    /// `reactionCatalog()`: what does not change between solves for every
    /// reaction, cached per built model and reaction label setting.
    pub fn reaction_catalog(&self) -> Arc<Vec<Arc<CatalogEntry>>> {
        let Some(community) = self.community.as_ref() else { return Arc::new(Vec::new()) };
        let label_source = self.map_label_source("reaction");
        let mut cache = self.reaction_caches.catalog.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, rows)) = cache.as_ref().filter(|(s, l, _)| *s == self.problem_serial && *l == label_source) {
            return rows.clone();
        }
        let compartment_names: IndexMap<String, coxeter_core::compartments::Names> =
            self.species.iter().map(|sp| (sp.id.clone(), self.models.get(&sp.file).map(|m| m.compartment_names.clone()).unwrap_or_default())).collect();
        let species_names: IndexMap<String, String> =
            self.species.iter().map(|sp| (sp.id.clone(), self.models.get(&sp.file).map_or_else(|| sp.id.clone(), |m| m.cell.name.clone()))).collect();
        let empty = coxeter_core::compartments::Names::new();
        let names = CatalogNames {
            label: Box::new(|rid| self.entity_label("reaction", rid).label),
            species: Box::new(|id| match id {
                Some(id) => species_names.get(id).cloned().unwrap_or_else(|| id.to_string()),
                None => "Community (shared medium)".to_string(),
            }),
            compartment: Box::new(|sp, cid| cell_compartment_name(cid, sp.and_then(|s| compartment_names.get(s)).unwrap_or(&empty), "en")),
        };
        let rows: Arc<Vec<Arc<CatalogEntry>>> = Arc::new(build_reaction_catalog(community, &names).into_iter().map(Arc::new).collect());
        *cache = Some((self.problem_serial, label_source, rows.clone()));
        rows
    }

    /// `reactionTableRows()`: every reaction as a table row: the catalogue
    /// entry with its bounds in force and as opened, flux, FVA range,
    /// reduced cost, objective coefficient, whether it is drawn, the
    /// knocked-out genes that close it, and its status (culprit, knocked out,
    /// modified, blocked or original, in that order of precedence).
    pub fn reaction_table_rows(&self) -> Vec<ReactionRow> {
        let catalog = self.reaction_catalog();
        let fluxes = self.solution.as_ref().filter(|s| s.is_optimal()).map(|s| &s.fluxes);
        let costs = self.reduced_costs_available().then(|| self.solution.as_ref().and_then(|s| s.result.as_ref())).flatten().map(|r| &r.reduced_costs);
        let objective = self.objective_coefficients();
        let culprits = self.constraint_culprits.as_ref().map(|c| &c.ids);
        let held_out: HashSet<&str> = self.species.iter().filter(|sp| self.species_held_out(&sp.id)).map(|sp| sp.id.as_str()).collect();
        let genes_closing = self.gene_closed_reactions();
        catalog
            .iter()
            .map(|entry| {
                let id = entry.id.as_str();
                let current = self.current_reaction_bounds(id);
                let original = self.original_reaction_bounds(id);
                let range = self.fva_ranges.get(id);
                let modified = !same(&current, &original);
                let knocked_out = is_knocked_out(&current, &original);
                let blocked = range.is_some_and(|r| r.min.abs() <= 1e-9 && r.max.abs() <= 1e-9);
                let culprit = culprits.is_some_and(|c| c.contains(id));
                let status = if culprit {
                    RowStatus::Culprit
                } else if knocked_out {
                    RowStatus::KnockedOut
                } else if modified {
                    RowStatus::Modified
                } else if blocked {
                    RowStatus::Blocked
                } else {
                    RowStatus::Original
                };
                ReactionRow {
                    entry: entry.clone(),
                    lower: current.lower,
                    upper: current.upper,
                    original_lower: original.lower,
                    original_upper: original.upper,
                    modified,
                    knocked_out,
                    blocked,
                    culprit,
                    flux: fluxes.map_or(f64::NAN, |f| f.get(id).copied().unwrap_or(0.0)),
                    fva_min: range.map_or(f64::NAN, |r| r.min),
                    fva_max: range.map_or(f64::NAN, |r| r.max),
                    reduced_cost: costs.and_then(|c| c.get(id).copied()).unwrap_or(f64::NAN),
                    objective: objective.get(id).copied().unwrap_or(0.0),
                    on_map: self.map.as_ref().is_some_and(|m| m.has_reaction(id)),
                    held_out: entry.species_id.as_deref().is_some_and(|sp| held_out.contains(sp)),
                    gene_knockouts: genes_closing.get(id).cloned().unwrap_or_default(),
                    status,
                }
            })
            .collect()
    }

    /// `reducedCostsAvailable()`: whether the solution in force reports
    /// reduced costs worth a column.
    pub fn reduced_costs_available(&self) -> bool {
        let Some(solution) = self.solution.as_ref().filter(|s| s.is_optimal()) else { return false };
        !self.loopless && solution.result.as_ref().is_some_and(|r| r.dual_analysis.available && !r.reduced_costs.is_empty())
    }

    /// `reactionTableModel()`: the table as the Reactions page lists it, for
    /// the current search, filters, sort and grouping.
    pub fn reaction_table_model(&self) -> ReactionTableModel {
        let rows = self.reaction_table_rows();
        let table = &self.reaction_table;
        let visible = sort_reaction_rows(filter_reaction_rows(&rows, &table.query, &table.filters), table.sort);
        let groups = (table.group_by == Grouping::Pathway).then(|| self.managed_pathway_coloring(false).groups);
        let pathway_of = |rid: &str| -> Option<String> {
            groups.as_ref()?.get(rid).filter(|name| !name.eq_ignore_ascii_case("Sin pathway")).cloned()
        };
        let items = group_reaction_rows(
            visible.clone(),
            table.group_by,
            &table.collapsed,
            &table.selection,
            if groups.is_some() { Some(&pathway_of) } else { None },
        );
        ReactionTableModel {
            total: rows.len(),
            shown: visible.len(),
            modified: rows.iter().filter(|r| r.modified).count(),
            selected: table.selection.len(),
            columns: TableColumns { fva: !self.fva_ranges.is_empty() || self.fva_enabled, rc: self.reduced_costs_available() },
            rows,
            visible,
            items,
        }
    }

    /// `setReactionTable(patch)`: change how the Reactions page lists rows.
    /// Emits `reaction-table`, which redraws the tables only.
    pub fn set_reaction_table(&mut self, patch: ReactionTablePatch) {
        let table = &mut self.reaction_table;
        if let Some(query) = patch.query {
            table.query = query;
        }
        if let Some(query) = patch.gene_query {
            table.gene_query = query;
        }
        if let Some(group_by) = patch.group_by {
            table.group_by = group_by;
            table.collapsed.clear();
        }
        if let Some(filters) = patch.filters {
            table.filters = ReactionFilter::ALL.into_iter().filter(|f| filters.contains(f)).collect();
        }
        if let Some(filter) = patch.toggle_filter {
            table.filters = toggle_reaction_filter(&table.filters, filter);
        }
        if let Some(key) = patch.sort_by {
            table.sort = if table.sort.key == key {
                SortSpec { key, dir: -table.sort.dir }
            } else {
                SortSpec { key, dir: if key.descends_first() { -1 } else { 1 } }
            };
        }
        if let Some(group) = patch.toggle_group {
            if !table.collapsed.remove(&group) {
                table.collapsed.insert(group);
            }
        }
        self.emit("reaction-table");
    }

    /// `selectReactionRows(ids, on)`: tick (`on`) or untick reactions for
    /// bulk actions.
    pub fn select_reaction_rows<S: AsRef<str>>(&mut self, ids: &[S], on: bool) {
        let known = |id: &str| self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(id));
        let ticked: Vec<String> = ids.iter().map(|id| id.as_ref().to_string()).filter(|id| on && known(id)).collect();
        let selection = &mut self.reaction_table.selection;
        if on {
            selection.extend(ticked);
        } else {
            for id in ids {
                selection.shift_remove(id.as_ref());
            }
        }
        self.emit("reaction-table");
    }

    /// `clearReactionRowSelection()`.
    pub fn clear_reaction_row_selection(&mut self) {
        if self.reaction_table.selection.is_empty() {
            return;
        }
        self.reaction_table.selection.clear();
        self.emit("reaction-table");
    }

    /// What `settleOpenedWork()` adds: bounds edits start over too, and what
    /// was opened is "original", with its medium.
    pub(crate) fn settle_opened_constraints(&mut self) {
        self.base_medium = self.medium.clone();
        self.constraint_history.clear();
        self.constraint_culprits = None;
        self.culprit_check = None;
        self.reaction_table.selection.clear();
    }

    /// What `loadState` resets before it reads a file: "original" bounds are
    /// the scenario's own, and what the file changes is a change.
    pub(crate) fn reset_opened_constraints(&mut self) {
        if let Some(Value::Object(medium)) = self.scenario.as_ref().and_then(|s| s.get("medium")) {
            self.base_medium = medium.iter().map(|(k, v)| (k.clone(), js::to_number(v))).collect();
        }
        self.constraint_history.clear();
        self.constraint_culprits = None;
        self.culprit_check = None;
    }
}
