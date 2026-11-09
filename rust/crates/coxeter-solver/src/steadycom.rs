//! SteadyCom: maximum community growth rate at community steady state.
//!
//! Ports `src/core/steadycom.js`.
//!
//! Chan SHJ, Simons MN, Maranas CD (2017) SteadyCom: Predicting microbial
//! abundances while ensuring community stability. PLoS Comput Biol 13(5):
//! e1005539. <https://doi.org/10.1371/journal.pcbi.1005539>
//!
//! The formulation the paper states is
//!
//! ```text
//!     max  mu
//!     s.t. sum_j S^k_ij V^k_j = 0                  for each metabolite i of member k
//!          LB^k_j X^k <= V^k_j <= UB^k_j X^k       for each reaction j of member k
//!          sum_k V^k_ex(i) + u^com_i >= 0          for each community metabolite i
//!          V^k_biomass = X^k mu                    for each member k
//!          sum_k X^k = 1
//!          X^k, mu >= 0
//! ```
//!
//! which is not an LP, because `X^k mu` multiplies two variables. It becomes
//! one the moment mu is fixed, and the maximum total biomass is non-increasing
//! in mu, so the paper solves a sequence of LPs and brackets the largest mu
//! that still supports the required total biomass. That is what this module
//! does: bisection on mu, with each step an ordinary LP.
//!
//! Note on units, because it is the easiest thing to get wrong here: V is an
//! absolute rate (mmol/h) for the whole population of that member, not a
//! specific rate, which is why the bounds are scaled by X^k. Divide by X^k to
//! recover the per-gDW fluxes an individual FBA would report.
//!
//! The JS solves every LP with GLPK, and much of its care (the capped
//! formulation, the floor on the bisection's resolution, the retries without
//! presolve, the row equilibration) exists because of what GLPK does on these
//! problems. All of it is kept: the answers the app reports are defined by this
//! procedure, and HiGHS runs it unchanged (it is also robust where GLPK was
//! not, so the retries simply rarely fire).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use coxeter_core::js;
use coxeter_core::model::Model;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::lp::{HighsBackend, LpBackend, LpProblem, LpStatus, SolveOptions, SparseColumns};

/// How close the capped bisection is allowed to get to the maximum growth
/// rate. See the note in [`steady_com`]: nearer than this the LP could cycle
/// in GLPK and never return.
pub const CAPPED_FLOOR: f64 = 1e-5;

/// Errors a caller can cause (bad specs, bad options), and cancellation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SteadyComError {
    /// The JS throws these, and the message is the JS one.
    #[error("{0}")]
    Invalid(String),
    #[error("SteadyCom was cancelled.")]
    Cancelled,
}

fn invalid<T>(message: impl Into<String>) -> Result<T, SteadyComError> {
    Err(SteadyComError::Invalid(message.into()))
}

/// One member of the community.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub id: String,
    pub name: Option<String>,
    /// Id within that member's model (separate members) or within the joint
    /// reactions.
    pub biomass_reaction: String,
    pub atp_maintenance: Option<String>,
    /// Separate-model communities: the member's own model.
    pub model: Option<Arc<Model>>,
    /// Separate-model communities: base metabolite → that member's exchange.
    pub exchange_for: IndexMap<String, String>,
}

impl Member {
    /// A member of a joint community: no model of its own.
    pub fn joint(id: &str, biomass_reaction: &str) -> Member {
        Member {
            id: id.into(),
            name: None,
            biomass_reaction: biomass_reaction.into(),
            atp_maintenance: None,
            model: None,
            exchange_for: IndexMap::new(),
        }
    }
}

/// `communityExchangeBounds[base]`: an explicit range for a community budget row.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ExchangeBounds {
    pub lower: Option<f64>,
    pub upper: Option<f64>,
}

/// A reaction of a joint community model.
#[derive(Debug, Clone, PartialEq)]
pub struct JointReaction {
    pub id: String,
    pub name: String,
    pub subsystem: String,
    /// Metabolite id → coefficient, in JS key order.
    pub metabolites: IndexMap<String, f64>,
    /// The member it belongs to, or the community tag (`comm`).
    pub species: String,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub objective: f64,
}

/// One term of an extra linear constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintTerm {
    pub reaction: String,
    pub coef: f64,
    /// `part: 'uptake'`: only the uptake side of the flux counts.
    pub uptake_only: bool,
}

/// A term on a member's biomass variable X^k.
#[derive(Debug, Clone, PartialEq)]
pub struct BiomassTerm {
    pub species: String,
    pub coef: f64,
}

/// An extra linear constraint `lo <= Σ terms + Σ biomass <= hi` of a joint
/// community (the Figure 4 fiber limits, disabled members).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Constraint {
    pub name: Option<String>,
    pub terms: Vec<ConstraintTerm>,
    pub biomass: Vec<BiomassTerm>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
}

/// The Figure 4 fiber condition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fiber {
    pub bacteroides: f64,
    pub faecalibacterium: f64,
}

/// What `jointSteadyComSpec` records about the joint model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JointMetadata {
    /// Reaction id → owning member (`None` for community reactions).
    pub reaction_owners: IndexMap<String, Option<String>>,
    /// Shared metabolite id (`x[u]`) → display base (`x`).
    pub bases: IndexMap<String, String>,
    /// `"<metabolite>\0<member>"` → species-to-community exchange reaction.
    pub species_exchange: IndexMap<String, String>,
    /// Shared metabolite → `UT_x(u)`.
    pub community_uptake: IndexMap<String, String>,
    /// Shared metabolite → `EX_x(u)`.
    pub community_export: IndexMap<String, String>,
    pub fiber: Option<Fiber>,
}

/// A community that carries its own shared compartment.
#[derive(Debug, Clone, PartialEq)]
pub struct JointCommunity {
    pub reactions: IndexMap<String, JointReaction>,
    /// Metabolite documents (the member model's JSON plus `species`).
    pub metabolites: IndexMap<String, Map<String, Value>>,
    pub community_tag: String,
    pub constraints: Vec<Constraint>,
    pub metadata: Option<JointMetadata>,
}

/// A SteadyCom problem in either of the two shapes the JS accepts.
///
/// `members[].model` + `exchange_for` is the shape the four-auxotroph community
/// uses: separate single-organism models whose own exchange reactions are
/// summed into one community budget row per metabolite.
///
/// `joint` is a model that already carries its community structure: a shared
/// `[u]` compartment with its own mass balance, species-to-community exchange
/// reactions and community-to-environment uptake and export reactions. The
/// nine-species gut model is distributed that way, and it is also the
/// formulation the paper's own code builds.
#[derive(Debug, Clone, PartialEq)]
pub struct SteadyComSpec {
    pub members: Vec<Member>,
    /// Required total biomass (gDW); 1 unless stated.
    pub total_biomass: f64,
    /// Maximise the total up to this cap instead of fixing it.
    pub biomass_cap: Option<f64>,
    /// Separate members: base metabolite → budget for the whole community
    /// (positive mmol/h per gDW of community).
    pub community_uptake: IndexMap<String, f64>,
    /// Separate members: explicit bounds of a community budget row.
    pub community_exchange_bounds: IndexMap<String, ExchangeBounds>,
    pub joint: Option<JointCommunity>,
}

impl SteadyComSpec {
    /// `metadata.kind`: `"joint"` or `"separate"`.
    pub fn kind(&self) -> &'static str {
        if self.joint.is_some() { "joint" } else { "separate" }
    }

    /// The species tag of community reactions: the joint model's own, else
    /// `comm`.
    pub fn community_tag(&self) -> &str {
        self.joint.as_ref().map_or("comm", |j| j.community_tag.as_str())
    }
}

/// `onProgress({ iterations, lo, hi, mu })`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SteadyComProgress {
    pub iterations: u32,
    pub lo: f64,
    pub hi: f64,
    pub mu: f64,
}

/// Options of [`steady_com`]. The defaults are the JS ones.
pub struct SteadyComOptions<'a> {
    /// Growth-rate resolution of the bisection (1e-6).
    pub tolerance: f64,
    /// Most LPs to solve (60).
    pub max_iterations: u32,
    /// First upper bracket for mu (2.0).
    pub upper_guess: f64,
    /// Maximise the total biomass up to this cap at each step (the paper's
    /// algorithm, much cheaper on large communities) instead of fixing it.
    pub biomass_cap: Option<f64>,
    /// Per-LP time limit (600 s).
    pub time_limit_seconds: f64,
    pub on_progress: Option<Box<dyn FnMut(&SteadyComProgress) + 'a>>,
    /// Set to stop. The solve then returns [`SteadyComError::Cancelled`].
    pub cancel: Option<Arc<AtomicBool>>,
    /// The LP solver: HiGHS unless the caller passes another (the solver the
    /// user chose, or a test's own).
    pub backend: Option<&'a dyn LpBackend>,
}

impl Default for SteadyComOptions<'_> {
    fn default() -> Self {
        SteadyComOptions {
            tolerance: 1e-6,
            max_iterations: 60,
            upper_guess: 2.0,
            biomass_cap: None,
            time_limit_seconds: 600.0,
            on_progress: None,
            cancel: None,
            backend: None,
        }
    }
}

/// Options of [`max_total_biomass_at`] and [`parsimonious_at`].
#[derive(Clone, Default)]
pub struct AtOptions<'a> {
    pub biomass_cap: Option<f64>,
    /// Per-LP time limit; 600 s when `None`.
    pub time_limit_seconds: Option<f64>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub backend: Option<&'a dyn LpBackend>,
}

/// Status of a growth-rate search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SteadyComStatus {
    Optimal,
    Infeasible,
    /// An LP did not come back with an answer.
    SolverError,
    /// The LP budget ran out or no bracket was found.
    IterationLimit,
    /// The range search needs µ = 0 to be feasible and it is not.
    ZeroInfeasible,
}

impl SteadyComStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SteadyComStatus::Optimal => "optimal",
            SteadyComStatus::Infeasible => "infeasible",
            SteadyComStatus::SolverError => "solver-error",
            SteadyComStatus::IterationLimit => "iteration-limit",
            SteadyComStatus::ZeroInfeasible => "zero-infeasible",
        }
    }
}

/// The result of [`steady_com`].
#[derive(Debug, Clone, PartialEq)]
pub struct SteadyComResult {
    pub status: SteadyComStatus,
    pub growth_rate: f64,
    /// The verified range `[0, max]`; only set when optimal.
    pub range_lower: Option<f64>,
    pub range_upper: Option<f64>,
    /// The infeasible end of the final bracket.
    pub upper_bound: Option<f64>,
    pub resolution: Option<f64>,
    /// Whether µ = 0 was found feasible (`None` when the search failed first).
    pub zero_feasible: Option<bool>,
    /// Member id → X^k, summing to the total biomass.
    pub abundances: IndexMap<String, f64>,
    /// `member|reaction` (separate) or reaction id (joint) → absolute flux.
    pub fluxes: IndexMap<String, f64>,
    pub iterations: u32,
    pub message: String,
}

/// The LP at one fixed growth rate (`solveAt`).
#[derive(Debug, Clone, PartialEq)]
pub struct AtResult {
    /// Whether the required total biomass is reached.
    pub feasible: bool,
    /// Σ X^k (`NaN` unless the LP solved).
    pub total: f64,
    pub status: LpStatus,
    pub message: String,
    pub objective: Option<f64>,
    pub abundances: IndexMap<String, f64>,
    pub fluxes: IndexMap<String, f64>,
    /// LPs this answer took (retries count).
    pub lp_evaluations: u32,
    /// [`parsimonious_at`]: the least total member flux.
    pub total_flux: Option<f64>,
}

// Validation

fn positive_number(value: f64, name: &str) -> Result<(), SteadyComError> {
    if !value.is_finite() || value <= 0.0 {
        return invalid(format!("{name} must be a finite positive number."));
    }
    Ok(())
}

fn validate_cap(cap: Option<f64>, total: f64) -> Result<(), SteadyComError> {
    let Some(cap) = cap else { return Ok(()) };
    positive_number(cap, "The biomass cap")?;
    if cap < total {
        return invalid("The biomass cap cannot be below the required total biomass.");
    }
    Ok(())
}

fn validate_spec(spec: &SteadyComSpec) -> Result<(), SteadyComError> {
    if spec.members.is_empty() {
        return invalid("A SteadyCom problem needs at least one member.");
    }
    positive_number(spec.total_biomass, "Total community biomass")?;
    let mut ids = std::collections::HashSet::new();
    for member in &spec.members {
        if member.id.is_empty() || !ids.insert(member.id.as_str()) {
            return invalid("SteadyCom member identifiers must be nonempty and unique.");
        }
        let has = match &spec.joint {
            Some(j) => j.reactions.contains_key(&member.biomass_reaction),
            None => member.model.as_ref().is_some_and(|m| m.reactions.contains_key(&member.biomass_reaction)),
        };
        if !has {
            return invalid(format!("Member {} has no valid biomass reaction {}.", member.id, member.biomass_reaction));
        }
    }
    Ok(())
}

fn validate_mu(mu: f64) -> Result<(), SteadyComError> {
    if !mu.is_finite() || mu < 0.0 {
        return invalid("The fixed growth rate must be a finite number of zero or more.");
    }
    Ok(())
}

// Building

#[derive(Debug, Clone)]
struct Col {
    name: String,
    lb: f64,
    ub: f64,
}

#[derive(Debug, Clone)]
struct Row {
    name: String,
    entries: Vec<(usize, f64)>,
    lo: f64,
    hi: f64,
}

/// The laid-out community: one column per member reaction, one per member
/// biomass variable X^k, and rows for mass balance, the bound coupling, the
/// community budget, the normalisation and the growth coupling. Only the few
/// coefficients that depend on mu are rewritten between bisection steps.
#[derive(Debug, Clone)]
struct Problem {
    member_ids: Vec<String>,
    cols: Vec<Col>,
    rows: Vec<Row>,
    /// `member\0reaction` (separate) or reaction id (joint) → column.
    flux_cols: IndexMap<String, usize>,
    biomass_cols: IndexMap<String, usize>,
    growth_rows: Vec<usize>,
    time_limit_seconds: Option<f64>,
    growth_row_scale: Option<f64>,
    presolve: bool,
}

impl Problem {
    fn push_col(&mut self, name: String, lb: f64, ub: f64) -> usize {
        self.cols.push(Col { name, lb, ub });
        self.cols.len() - 1
    }
    fn push_row(&mut self, name: String, entries: Vec<(usize, f64)>, lo: f64, hi: f64) -> usize {
        self.rows.push(Row { name, entries, lo, hi });
        self.rows.len() - 1
    }
}

fn empty_problem(spec: &SteadyComSpec) -> Problem {
    Problem {
        member_ids: spec.members.iter().map(|m| m.id.clone()).collect(),
        cols: Vec::new(),
        rows: Vec::new(),
        flux_cols: IndexMap::new(),
        biomass_cols: IndexMap::new(),
        growth_rows: Vec::new(),
        time_limit_seconds: None,
        growth_row_scale: None,
        presolve: true,
    }
}

fn build_any(spec: &SteadyComSpec, biomass_cap: Option<f64>) -> Result<Problem, SteadyComError> {
    match &spec.joint {
        Some(joint) => build_joint_problem(spec, joint, biomass_cap),
        None => build_problem(spec, biomass_cap),
    }
}

fn key(member: &str, reaction: &str) -> String {
    format!("{member}\u{0}{reaction}")
}

fn build_problem(spec: &SteadyComSpec, biomass_cap: Option<f64>) -> Result<Problem, SteadyComError> {
    let mut p = empty_problem(spec);
    let models: Vec<&Model> = spec
        .members
        .iter()
        .map(|m| m.model.as_deref().ok_or_else(|| SteadyComError::Invalid(format!("Member {} has no model.", m.id))))
        .collect::<Result<_, _>>()?;

    // Columns: member fluxes, then one biomass variable per member. The
    // absolute rate is unbounded. The LB*X <= V <= UB*X rows do the bounding,
    // which is the whole point of the formulation.
    for (member, model) in spec.members.iter().zip(&models) {
        for rid in model.reactions.keys() {
            let col = p.push_col(format!("{}|{rid}", member.id), f64::NEG_INFINITY, f64::INFINITY);
            p.flux_cols.insert(key(&member.id, rid), col);
        }
    }
    // Bounded by the community total, or by the cap when the total is being
    // maximised instead of fixed. Leaving these free lets the LP drift to a
    // total of ten-odd gDW with fluxes to match, which is where GLPK gave up.
    let total = spec.total_biomass;
    for member in &spec.members {
        let col = p.push_col(format!("X|{}", member.id), 0.0, biomass_cap.unwrap_or(total));
        p.biomass_cols.insert(member.id.clone(), col);
    }

    // Mass balance per member: S^k V^k = 0
    for (member, model) in spec.members.iter().zip(&models) {
        let mut per_met: IndexMap<&str, Vec<(usize, f64)>> = IndexMap::new();
        for (rid, rxn) in &model.reactions {
            let col = p.flux_cols[&key(&member.id, rid)];
            for (mid, coef) in &rxn.metabolites {
                if *coef == 0.0 || coef.is_nan() {
                    continue;
                }
                per_met.entry(mid.as_str()).or_default().push((col, *coef));
            }
        }
        for (mid, entries) in per_met {
            p.push_row(format!("{}|{mid}", member.id), entries, 0.0, 0.0);
        }
    }

    // Bound coupling: V - UB*X <= 0 and V - LB*X >= 0
    for (member, model) in spec.members.iter().zip(&models) {
        let x_col = p.biomass_cols[&member.id];
        for (rid, rxn) in &model.reactions {
            let col = p.flux_cols[&key(&member.id, rid)];
            if rxn.upper_bound.is_finite() {
                p.push_row(format!("{}|{rid}|ub", member.id), vec![(col, 1.0), (x_col, -rxn.upper_bound)], f64::NEG_INFINITY, 0.0);
            }
            if rxn.lower_bound.is_finite() {
                p.push_row(format!("{}|{rid}|lb", member.id), vec![(col, 1.0), (x_col, -rxn.lower_bound)], 0.0, f64::INFINITY);
            }
        }
    }

    // Community exchange space: sum_k V^k_ex(i) + u^com_i >= 0
    //
    // One row for EVERY metabolite any member can exchange, not only the
    // ones the medium supplies. A metabolite the medium does not provide
    // gets a budget of zero, which is exactly what forces cross-feeding: a
    // member can only take it up to the extent that another member secretes
    // it. Without the zero-budget rows each member draws from nowhere and an
    // auxotroph appears to grow alone.
    let mut exchange_bases: indexmap::IndexSet<&str> = indexmap::IndexSet::new();
    for member in &spec.members {
        for base in member.exchange_for.keys() {
            exchange_bases.insert(base);
        }
    }
    for base in exchange_bases {
        let mut entries = Vec::new();
        for member in &spec.members {
            let Some(rid) = member.exchange_for.get(base) else { continue };
            if rid.is_empty() {
                continue;
            }
            if let Some(&col) = p.flux_cols.get(&key(&member.id, rid)) {
                entries.push((col, 1.0));
            }
        }
        if entries.is_empty() {
            continue;
        }
        let budget = spec.community_uptake.get(base).copied().unwrap_or(0.0).abs();
        let bounds = spec.community_exchange_bounds.get(base);
        let lo = bounds.and_then(|b| b.lower).unwrap_or(-budget);
        let hi = bounds.and_then(|b| b.upper).unwrap_or(f64::INFINITY);
        p.push_row(format!("community|{base}"), entries, lo, hi);
    }

    // Normalisation: sum_k X^k = total biomass. Equation (15) of the paper,
    // and the reason the abundances it reports are relative.
    //
    // Fixing the total makes each bisection step a feasibility question.
    // Capping it instead makes the step an optimisation (maximise the
    // total, then see whether it reached the required one), which is the
    // paper's own algorithm and answers the same question far more cheaply,
    // because proving a large LP infeasible costs an order of magnitude more
    // than solving a feasible one. Either way every X stays bounded, which is
    // what keeps the LP conditioned.
    push_normalisation(&mut p, total, biomass_cap);

    // Growth coupling, rewritten per bisection step: V_biomass - mu*X = 0
    for member in &spec.members {
        let bio = p.flux_cols[&key(&member.id, &member.biomass_reaction)];
        let x = p.biomass_cols[&member.id];
        let row = p.push_row(format!("growth|{}", member.id), vec![(bio, 1.0), (x, 0.0)], 0.0, 0.0);
        p.growth_rows.push(row);
    }
    Ok(p)
}

fn push_normalisation(p: &mut Problem, total: f64, cap: Option<f64>) {
    let entries: Vec<(usize, f64)> = p.biomass_cols.values().map(|&c| (c, 1.0)).collect();
    let (lo, hi) = match cap {
        None => (total, total),
        Some(cap) => (0.0, cap),
    };
    p.push_row("normalisation".into(), entries, lo, hi);
}

/// Lay out a community that already carries its own community compartment.
///
/// The difference from [`build_problem`] is where the community lives. There,
/// each member keeps its own exchange reactions and a budget row sums them.
/// Here the shared medium is in the model: community metabolites have their own
/// mass balance, species-to-community exchange reactions move material in and
/// out of it, and community uptake and export reactions connect it to the
/// environment.
///
/// That changes which fluxes scale with abundance. A species reaction is a
/// specific rate and is bounded by LB*X <= V <= UB*X. A community exchange
/// reaction is an absolute community rate (the diet is so many mmol per hour
/// for the whole community, not per gram of any member), so it keeps its own
/// bounds and is coupled to nothing.
fn build_joint_problem(spec: &SteadyComSpec, joint: &JointCommunity, biomass_cap: Option<f64>) -> Result<Problem, SteadyComError> {
    let mut p = empty_problem(spec);
    let total = spec.total_biomass;
    let tag = joint.community_tag.as_str();

    for (id, rxn) in &joint.reactions {
        let col = if rxn.species == tag {
            p.push_col(id.clone(), rxn.lower_bound, rxn.upper_bound)
        } else {
            p.push_col(id.clone(), f64::NEG_INFINITY, f64::INFINITY)
        };
        p.flux_cols.insert(id.clone(), col);
    }
    for member in &spec.members {
        let col = p.push_col(format!("X|{}", member.id), 0.0, biomass_cap.unwrap_or(total));
        p.biomass_cols.insert(member.id.clone(), col);
    }

    // Mass balance, for species and community metabolites alike.
    let mut per_met: IndexMap<&str, Vec<(usize, f64)>> = IndexMap::new();
    for (id, rxn) in &joint.reactions {
        let col = p.flux_cols[id];
        for (mid, coef) in &rxn.metabolites {
            if *coef == 0.0 || coef.is_nan() {
                continue;
            }
            per_met.entry(mid.as_str()).or_default().push((col, *coef));
        }
    }
    for (mid, entries) in per_met {
        p.push_row(mid.to_string(), entries, 0.0, 0.0);
    }

    // Bound coupling, for species reactions only.
    for (id, rxn) in &joint.reactions {
        if rxn.species == tag {
            continue;
        }
        let Some(&x_col) = p.biomass_cols.get(&rxn.species) else {
            return invalid(format!("reaction {id} belongs to species {}, which is not a member", rxn.species));
        };
        let col = p.flux_cols[id];
        if rxn.upper_bound.is_finite() {
            p.push_row(format!("{id}|ub"), vec![(col, 1.0), (x_col, -rxn.upper_bound)], f64::NEG_INFINITY, 0.0);
        }
        if rxn.lower_bound.is_finite() {
            p.push_row(format!("{id}|lb"), vec![(col, 1.0), (x_col, -rxn.lower_bound)], 0.0, f64::INFINITY);
        }
    }

    // Normalisation: fixed when there is no cap, capped when the caller
    // means to maximise the total instead.
    push_normalisation(&mut p, total, biomass_cap);

    // Any extra linear constraints the caller asked for.
    for constraint in &joint.constraints {
        let mut entries = Vec::new();
        let label = constraint.name.clone().unwrap_or_else(|| "undefined".into());
        for term in &constraint.terms {
            let Some(&col) = p.flux_cols.get(&term.reaction) else {
                return invalid(format!("constraint {label}: no reaction {}", term.reaction));
            };
            if !term.uptake_only {
                entries.push((col, term.coef));
                continue;
            }
            // Only the uptake side of this flux counts. Split it: u >= 0 and
            // u >= -V, then use u. Nothing pushes u above max(0, -V), because u
            // appears only here and only with a positive coefficient in a <=
            // constraint, so the bound is tight at the optimum and this is
            // exact rather than a relaxation.
            let uptake = p.push_col(format!("uptake|{}", term.reaction), 0.0, f64::INFINITY);
            p.push_row(format!("uptake|{}|split", term.reaction), vec![(col, 1.0), (uptake, 1.0)], 0.0, f64::INFINITY);
            entries.push((uptake, term.coef));
        }
        for term in &constraint.biomass {
            let Some(&col) = p.biomass_cols.get(&term.species) else {
                return invalid(format!("constraint {label}: no member {}", term.species));
            };
            entries.push((col, term.coef));
        }
        if !entries.is_empty() {
            let name = constraint.name.clone().unwrap_or_else(|| format!("constraint{}", p.rows.len()));
            p.push_row(name, entries, constraint.lo.unwrap_or(f64::NEG_INFINITY), constraint.hi.unwrap_or(f64::INFINITY));
        }
    }

    // Growth coupling, rewritten per bisection step.
    for member in &spec.members {
        let Some(&col) = p.flux_cols.get(&member.biomass_reaction) else {
            return invalid(format!("member {}: no biomass reaction {}", member.id, member.biomass_reaction));
        };
        let x = p.biomass_cols[&member.id];
        let row = p.push_row(format!("growth|{}", member.id), vec![(col, 1.0), (x, 0.0)], 0.0, 0.0);
        p.growth_rows.push(row);
    }
    Ok(p)
}

/// An objective over columns: `{ entries: [[col, coef]], maximize }`.
#[derive(Debug, Clone)]
struct LinearObjective {
    entries: Vec<(usize, f64)>,
    maximize: bool,
}

fn biomass_objective(p: &Problem) -> LinearObjective {
    LinearObjective { entries: p.biomass_cols.values().map(|&c| (c, 1.0)).collect(), maximize: true }
}

/// Solve the LP for one fixed growth rate (`solveAt`).
///
/// With no objective this is a feasibility question (can the community carry
/// the required biomass at this growth rate?), which is what the bisection
/// asks. With one it becomes the FVA step: the same feasible set, minimised or
/// maximised over a linear target.
fn solve_at(
    p: &mut Problem,
    mu: f64,
    total_biomass: f64,
    objective: Option<&LinearObjective>,
    backend: &dyn LpBackend,
    cancel: Option<&Arc<AtomicBool>>,
) -> AtResult {
    // Only the growth-coupling coefficient changes with mu.
    for &r in &p.growth_rows {
        p.rows[r].entries[1].1 = -mu;
    }
    let lp = to_lp(p, objective);
    let res = backend.solve(
        &lp,
        &SolveOptions {
            time_limit: p.time_limit_seconds,
            presolve: p.presolve,
            interrupt: cancel.cloned(),
            ..Default::default()
        },
    );

    if res.status != LpStatus::Optimal {
        return AtResult {
            feasible: false,
            total: f64::NAN,
            status: res.status,
            message: res.message,
            objective: None,
            abundances: IndexMap::new(),
            fluxes: IndexMap::new(),
            lp_evaluations: 1,
            total_flux: None,
        };
    }
    let x = res.x.unwrap_or_default();
    let mut total = 0.0;
    for &col in p.biomass_cols.values() {
        total += x[col];
    }
    let feasible = total >= total_biomass - 1e-7;
    let abundances: IndexMap<String, f64> = p.biomass_cols.iter().map(|(id, &col)| (id.clone(), x[col])).collect();

    if let Some(scale) = p.growth_row_scale.filter(|s| *s != 0.0) {
        let inaccurate = p.member_ids.iter().enumerate().any(|(index, id)| {
            let x_k = abundances[id];
            let biomass_column = p.rows[p.growth_rows[index]].entries[0].0;
            x_k > 1e-9 && (x[biomass_column] / x_k - mu).abs() > 1e-7
        });
        if inaccurate {
            if p.presolve {
                p.presolve = false;
                let mut refined = solve_at(p, mu, total_biomass, objective, backend, cancel);
                refined.lp_evaluations += 1;
                return refined;
            }
            if scale < 1e7 {
                p.growth_row_scale = Some(scale * 100.0);
                let mut refined = solve_at(p, mu, total_biomass, objective, backend, cancel);
                refined.lp_evaluations += 1;
                return refined;
            }
            return AtResult {
                feasible: false,
                total: f64::NAN,
                status: LpStatus::IterationLimit,
                message: "The solver did not resolve the specific growth of tiny populations accurately enough.".into(),
                objective: None,
                abundances: IndexMap::new(),
                fluxes: IndexMap::new(),
                lp_evaluations: 1,
                total_flux: None,
            };
        }
    }
    // Keys are `member\0reaction` when the members came in as separate models,
    // and a bare reaction id for a joint model whose reactions are unique.
    let fluxes: IndexMap<String, f64> = p
        .flux_cols
        .iter()
        .map(|(k, &col)| {
            let name = match k.split_once('\u{0}') {
                Some((member, rid)) => format!("{member}|{rid}"),
                None => k.clone(),
            };
            (name, x[col])
        })
        .collect();
    AtResult {
        feasible,
        total,
        status: res.status,
        message: String::new(),
        objective: Some(res.objective),
        abundances,
        fluxes,
        lp_evaluations: 1,
        total_flux: None,
    }
}

/// The LP of `p` as it stands.
///
/// Each row is equilibrated: divided by its largest coefficient. The bound
/// coupling contributes `V - 1000*X <= 0` while a biomass reaction contributes
/// stoichiometry around 1e-5, so the untouched matrix spans eight orders of
/// magnitude, and GLPK answered GLP_UNDEF instead of solving it. Scaling a
/// row changes nothing about its solution set, exactly rather than
/// approximately: every bound here is either zero or scaled with the row. It
/// does scale that row's dual by the same factor, which is why nothing in this
/// module reads duals.
fn to_lp(p: &Problem, objective: Option<&LinearObjective>) -> LpProblem {
    let m = p.rows.len();
    let n = p.cols.len();
    let mut a = SparseColumns::new(m, n);
    let mut row_lower = vec![0.0; m];
    let mut row_upper = vec![0.0; m];
    for (i, row) in p.rows.iter().enumerate() {
        let largest = row.entries.iter().fold(0.0f64, |l, (_, c)| js::max(l, c.abs()));
        let growth = if row.name.starts_with("growth|") { p.growth_row_scale.unwrap_or(1.0) } else { 1.0 };
        let scale = (if largest > 0.0 { 1.0 / largest } else { 1.0 }) * growth;
        for &(col, coef) in &row.entries {
            a.add(i, col, coef * scale);
        }
        row_lower[i] = if row.lo.is_finite() { row.lo * scale } else { row.lo };
        row_upper[i] = if row.hi.is_finite() { row.hi * scale } else { row.hi };
    }
    let mut cost = vec![0.0; n];
    // Without an objective the total is already fixed by the normalisation
    // row, so there is nothing left to optimise: any feasible point is an
    // answer and the question is only whether one exists.
    if let Some(o) = objective {
        for &(col, coef) in &o.entries {
            cost[col] += coef;
        }
    }
    LpProblem {
        a,
        row_lower,
        row_upper,
        cost,
        col_lower: p.cols.iter().map(|c| c.lb).collect(),
        col_upper: p.cols.iter().map(|c| c.ub).collect(),
        maximize: objective.is_none_or(|o| o.maximize),
        integer: Vec::new(),
    }
}

fn scaled(values: &IndexMap<String, f64>, factor: f64) -> IndexMap<String, f64> {
    if factor == 1.0 {
        return values.clone();
    }
    values.iter().map(|(k, v)| (k.clone(), v * factor)).collect()
}

fn fail(message: String, status: SteadyComStatus, iterations: u32, zero_feasible: Option<bool>) -> SteadyComResult {
    SteadyComResult {
        status,
        growth_rate: 0.0,
        range_lower: None,
        range_upper: None,
        upper_bound: None,
        resolution: None,
        zero_feasible,
        abundances: IndexMap::new(),
        fluxes: IndexMap::new(),
        iterations,
        message,
    }
}

// The searches

/// The largest growth rate the community supports, by bisection on mu.
pub fn steady_com(spec: &SteadyComSpec, mut options: SteadyComOptions<'_>) -> Result<SteadyComResult, SteadyComError> {
    validate_spec(spec)?;
    positive_number(options.tolerance, "Growth-rate tolerance")?;
    positive_number(options.upper_guess, "Initial upper growth-rate guess")?;
    positive_number(options.time_limit_seconds, "LP time limit")?;
    if options.max_iterations < 1 {
        return invalid("The maximum LP count must be a positive integer.");
    }
    let backend: &dyn LpBackend = options.backend.unwrap_or(&HighsBackend);
    let cancel = options.cancel.clone();
    let total_biomass = spec.total_biomass;

    // With a cap, each step maximises the total biomass and the bisection
    // compares what it reached against what is required. Without one, each
    // step asks directly whether the required total is feasible. Same question,
    // and the same answer. The difference is cost. On a nine-species
    // genome-scale community, proving one LP infeasible took fifteen to
    // twenty-six seconds against two and a half for a feasible one (GLPK), and
    // half of a bisection's steps are infeasible by construction. Capping turns
    // every step into the cheap case.
    let cap = options.biomass_cap.or(spec.biomass_cap);
    validate_cap(cap, total_biomass)?;
    let mut problem = build_any(spec, cap)?;
    problem.time_limit_seconds = Some(options.time_limit_seconds);
    let objective = cap.map(|_| biomass_objective(&problem));

    // The capped formulation is the cheap one, but it is an optimisation, and
    // within a whisker of the maximum growth rate the problem is degenerate
    // enough that GLPK's simplex could cycle and never return (every rate down
    // to mu* + 2^-18 solved in milliseconds; mu* + 2^-19 never came back).
    // The bisection therefore stops refining at CAPPED_FLOOR rather than
    // probing arbitrarily close to the edge. The growth rate is then known to
    // 1e-5, which is still finer than anything these papers report, and a
    // caller that wants the last decimals can leave the cap off and pay for the
    // exact formulation instead.
    let effective_tolerance = if cap.is_none() { options.tolerance } else { options.tolerance.max(CAPPED_FLOOR) };
    let mut fallback: Option<Problem> = None;
    let time_limit = options.time_limit_seconds;
    let fixed_problem = |fallback: &mut Option<Problem>| -> Result<(), SteadyComError> {
        if fallback.is_none() {
            let mut f = build_any(spec, None)?;
            f.time_limit_seconds = Some(time_limit);
            f.growth_row_scale = Some(1000.0);
            *fallback = Some(f);
        }
        Ok(())
    };

    // This search covers a range starting at zero only after checking that
    // zero is feasible. A positive biomass lower bound can exclude zero even
    // when positive growth is possible. That is a failed search premise, not a
    // verdict that the community cannot grow at any rate.
    let mut lo = 0.0;
    let mut hi = options.upper_guess;
    let mut iterations: u32 = 0;
    let mut failure: Option<String> = None;
    let mut failure_status = SteadyComStatus::SolverError;
    let max_iterations = options.max_iterations;

    // `run`: one counted LP, with progress.
    let mut run = |inner: &mut Problem,
                   mu: f64,
                   objective: Option<&LinearObjective>,
                   lo: f64,
                   hi: f64,
                   iterations: &mut u32,
                   failure: &mut Option<String>,
                   failure_status: &mut SteadyComStatus|
     -> Result<Option<AtResult>, SteadyComError> {
        if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(SteadyComError::Cancelled);
        }
        if *iterations >= max_iterations {
            *failure_status = SteadyComStatus::IterationLimit;
            *failure = Some("The LP count limit was reached before the growth-rate range was verified.".into());
            return Ok(None);
        }
        let result = solve_at(inner, mu, total_biomass, objective, backend, cancel.as_ref());
        if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(SteadyComError::Cancelled);
        }
        *iterations += result.lp_evaluations;
        if let Some(cb) = options.on_progress.as_mut() {
            cb(&SteadyComProgress { iterations: *iterations, lo, hi, mu });
        }
        Ok(Some(result))
    };

    // A step only narrows the bracket when the solver actually answered. What
    // counts as an answer differs between the two formulations, and conflating
    // them is its own bug. Fixed: the verdict IS feasibility, so a step
    // reporting infeasible has answered and anything else has not. Capped: the
    // LP is always feasible (every member can simply be absent), and the
    // verdict is the objective value, so an optimal solve has answered even
    // when the total it reached falls short of the one required.
    macro_rules! step {
        ($mu:expr) => {{
            let mu: f64 = $mu;
            let capped = cap.is_some();
            let mut out: Option<AtResult> = None;
            if let Some(res) = run(&mut problem, mu, objective.as_ref(), lo, hi, &mut iterations, &mut failure, &mut failure_status)? {
                let ok = if capped { res.status == LpStatus::Optimal } else { res.feasible || res.status == LpStatus::Infeasible };
                if ok {
                    out = Some(res);
                } else {
                    let mut last = Some(res);
                    if capped {
                        // The capped solve did not come back with an answer. Ask
                        // the same question the other way before giving up.
                        fixed_problem(&mut fallback)?;
                        last = run(fallback.as_mut().unwrap(), mu, None, lo, hi, &mut iterations, &mut failure, &mut failure_status)?;
                        if let Some(r) = &last {
                            if r.feasible || r.status == LpStatus::Infeasible {
                                out = last.clone();
                            }
                        }
                    }
                    if out.is_none() {
                        if let Some(r) = last {
                            failure = Some(format!(
                                "the LP at mu = {} did not solve ({}: {}). The result would be meaningless, so no growth rate is reported.",
                                js::number_to_string(mu),
                                r.status,
                                r.message
                            ));
                        }
                    }
                }
            }
            out
        }};
    }

    let failed = |failure: &Option<String>, status: SteadyComStatus, iterations: u32, zero: Option<bool>| {
        fail(failure.clone().unwrap_or_default(), status, iterations, zero)
    };

    let Some(feasible_at_zero) = step!(0.0) else {
        return Ok(failed(&failure, failure_status, iterations, None));
    };
    if !feasible_at_zero.feasible {
        return Ok(fail(
            "The range search requires a feasible solution at µ = 0, but zero growth cannot sustain the required biomass. \
             Check the medium and positive lower bounds; a fixed solve at a positive growth rate may still be feasible."
                .into(),
            SteadyComStatus::ZeroInfeasible,
            iterations,
            Some(false),
        ));
    }

    let mut best = feasible_at_zero;
    let mut bracketed = false;
    for _ in 0..20 {
        let Some(res) = step!(hi) else {
            return Ok(failed(&failure, failure_status, iterations, Some(true)));
        };
        if !res.feasible {
            bracketed = true;
            break;
        }
        lo = hi;
        best = res;
        hi *= 2.0;
        if !hi.is_finite() {
            break;
        }
    }
    if !bracketed {
        return Ok(fail(
            "No finite upper growth-rate bound was established. The maximum may be unbounded, or the initial guess needs \
             adjustment; no optimum is reported."
                .into(),
            SteadyComStatus::IterationLimit,
            iterations,
            Some(true),
        ));
    }

    while hi - lo > effective_tolerance && iterations < max_iterations {
        let mid = (lo + hi) / 2.0;
        if !(mid > lo && mid < hi) {
            break;
        }
        let Some(res) = step!(mid) else {
            return Ok(failed(&failure, failure_status, iterations, Some(true)));
        };
        if res.feasible {
            lo = mid;
            best = res;
        } else {
            hi = mid;
        }
    }
    if hi - lo > effective_tolerance {
        return Ok(fail(
            "The search stopped before its requested growth-rate resolution was reached; no verified maximum is reported.".into(),
            SteadyComStatus::IterationLimit,
            iterations,
            Some(true),
        ));
    }

    // Certify the usable endpoint with fixed total biomass and tighter growth
    // equalities. Tiny populations can otherwise hide a biomass error inside
    // the solver's absolute row tolerance, which becomes large after dividing
    // by X.
    for attempt in 0..3 {
        fixed_problem(&mut fallback)?;
        let Some(verified) = run(fallback.as_mut().unwrap(), lo, None, lo, hi, &mut iterations, &mut failure, &mut failure_status)? else {
            return Ok(failed(&failure, failure_status, iterations, Some(true)));
        };
        if verified.status == LpStatus::Optimal && verified.feasible {
            best = verified;
            break;
        }
        if verified.status != LpStatus::Infeasible {
            return Ok(fail(
                format!("The fixed-biomass endpoint at µ = {} was not verified ({}).", js::number_to_string(lo), verified.status),
                SteadyComStatus::SolverError,
                iterations,
                Some(true),
            ));
        }
        if attempt == 2 || lo == 0.0 {
            return Ok(fail("No numerically verified endpoint was found.".into(), SteadyComStatus::SolverError, iterations, Some(true)));
        }
        lo = (lo - effective_tolerance).max(0.0);
    }

    // The capped formulation maximises the total, so at a growth rate just
    // below the maximum it comes back with more biomass than was asked for,
    // and abundances that sum to that, not to one. Scaling the whole solution
    // down is exact rather than cosmetic: every constraint here is homogeneous
    // in (V, X) except the community exchange bounds, and those are intervals
    // containing zero, so shrinking a feasible solution leaves it feasible. The
    // paper's abundances sum to the total biomass by equation (15), and so do
    // these.
    let achieved = if best.total.is_nan() { total_biomass } else { best.total };
    let scale = if achieved > 0.0 { total_biomass / achieved } else { 1.0 };
    Ok(SteadyComResult {
        status: SteadyComStatus::Optimal,
        growth_rate: lo,
        range_lower: Some(0.0),
        range_upper: Some(lo),
        upper_bound: Some(hi),
        resolution: Some(effective_tolerance.max(hi - lo)),
        zero_feasible: Some(true),
        abundances: scaled(&best.abundances, scale),
        fluxes: scaled(&best.fluxes, scale),
        iterations,
        message: String::new(),
    })
}

/// The inner LP on its own: the largest total biomass the medium supports if
/// every member grows at exactly `mu`. SteadyCom's answer is the largest mu at
/// which this still reaches the required total, so this is the function being
/// bisected, and the thing to look at when an answer is surprising.
pub fn max_total_biomass_at(spec: &SteadyComSpec, mu: f64, options: &AtOptions<'_>) -> Result<AtResult, SteadyComError> {
    validate_spec(spec)?;
    validate_mu(mu)?;
    let cap = options.biomass_cap.or(spec.biomass_cap);
    validate_cap(cap, spec.total_biomass)?;
    let time_limit = options.time_limit_seconds.unwrap_or(600.0);
    positive_number(time_limit, "LP time limit")?;
    let mut problem = build_any(spec, cap)?;
    problem.time_limit_seconds = Some(time_limit);
    if cap.is_none() {
        problem.growth_row_scale = Some(1000.0);
    }
    let objective = cap.map(|_| biomass_objective(&problem));
    let backend: &dyn LpBackend = options.backend.unwrap_or(&HighsBackend);
    let res = solve_at(&mut problem, mu, spec.total_biomass, objective.as_ref(), backend, options.cancel.as_ref());
    check_cancel(options.cancel.as_ref())?;
    Ok(res)
}

/// The LP [`max_total_biomass_at`] solves at `mu`, for diagnostics.
#[doc(hidden)]
pub fn debug_lp_at(spec: &SteadyComSpec, mu: f64, cap: Option<f64>, growth_row_scale: Option<f64>) -> Result<LpProblem, SteadyComError> {
    let mut problem = build_any(spec, cap)?;
    problem.growth_row_scale = growth_row_scale;
    for &r in &problem.growth_rows {
        problem.rows[r].entries[1].1 = -mu;
    }
    let objective = cap.map(|_| biomass_objective(&problem));
    Ok(to_lp(&problem, objective.as_ref()))
}

fn check_cancel(cancel: Option<&Arc<AtomicBool>>) -> Result<(), SteadyComError> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) { Err(SteadyComError::Cancelled) } else { Ok(()) }
}

/// The most economical flux distribution among those that hold the community
/// at a fixed growth rate: parsimonious FBA, asked of SteadyCom.
///
/// At a fixed mu the feasible set is a polytope, and a feasibility solve
/// returns whichever vertex the simplex reaches first. On the nine-species gut
/// community that vertex carried futile cycles at the flux bounds (a member at
/// 0.15 % of the biomass running a reaction at 1000 mmol/gDW/h) and loops that
/// pass a metabolite back and forth between two members, which no per-species
/// loopless constraint can see. Minimising the summed magnitude of every member
/// flux picks the cheapest of the solutions instead, the way pFBA does for one
/// organism.
///
/// The sum runs over member reactions only, in their absolute (X-scaled)
/// units, so a gram of community is charged the same for flux in any member.
/// Community uptake and export are not charged: they follow from the member
/// exchanges by mass balance. The composition is left free under the
/// normalisation, so the answer is the cheapest community at this growth rate.
pub fn parsimonious_at(spec: &SteadyComSpec, mu: f64, options: &AtOptions<'_>) -> Result<AtResult, SteadyComError> {
    validate_spec(spec)?;
    validate_mu(mu)?;
    let time_limit = options.time_limit_seconds.unwrap_or(600.0);
    positive_number(time_limit, "LP time limit")?;
    let mut problem = build_any(spec, None)?;
    problem.time_limit_seconds = Some(time_limit);
    problem.growth_row_scale = Some(1000.0);
    let entries = absolute_flux_terms(&mut problem, spec);
    let backend: &dyn LpBackend = options.backend.unwrap_or(&HighsBackend);
    let objective = LinearObjective { entries, maximize: false };
    let mut result = solve_at(&mut problem, mu, spec.total_biomass, Some(&objective), backend, options.cancel.as_ref());
    check_cancel(options.cancel.as_ref())?;
    result.total_flux = Some(if result.status == LpStatus::Optimal { result.objective.unwrap_or(f64::NAN) } else { f64::NAN });
    Ok(result)
}

/// Objective terms whose sum is the total magnitude of the member fluxes.
///
/// A reaction whose lower bound is at least zero can only carry V >= LB*X >= 0,
/// so |V| is V itself, and one whose upper bound is at most zero has |V| = -V. Only
/// a reversible reaction needs an auxiliary t >= |V|, written as t - V >= 0 and
/// t + V >= 0, and minimising t makes it equal |V| at the optimum.
fn absolute_flux_terms(p: &mut Problem, spec: &SteadyComSpec) -> Vec<(usize, f64)> {
    let mut reactions: Vec<(usize, f64, f64)> = Vec::new();
    match &spec.joint {
        Some(joint) => {
            for (id, rxn) in &joint.reactions {
                if rxn.species != joint.community_tag {
                    reactions.push((p.flux_cols[id], rxn.lower_bound, rxn.upper_bound));
                }
            }
        }
        None => {
            for member in &spec.members {
                if let Some(model) = &member.model {
                    for (rid, rxn) in &model.reactions {
                        reactions.push((p.flux_cols[&key(&member.id, rid)], rxn.lower_bound, rxn.upper_bound));
                    }
                }
            }
        }
    }
    let mut entries = Vec::new();
    for (col, lower, upper) in reactions {
        if lower >= 0.0 {
            entries.push((col, 1.0));
            continue;
        }
        if upper <= 0.0 {
            entries.push((col, -1.0));
            continue;
        }
        let name = p.cols[col].name.clone();
        let t = p.push_col(format!("abs|{name}"), 0.0, f64::INFINITY);
        p.push_row(format!("abs|{name}|+"), vec![(t, 1.0), (col, -1.0)], 0.0, f64::INFINITY);
        p.push_row(format!("abs|{name}|-"), vec![(t, 1.0), (col, 1.0)], 0.0, f64::INFINITY);
        entries.push((t, 1.0));
    }
    entries
}

// FVA

/// A SteadyComFVA target: a linear combination of fluxes and biomasses.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SteadyComTarget {
    pub name: String,
    /// Reaction id (or `member\0reaction`) → coefficient.
    pub reactions: IndexMap<String, f64>,
    /// Member id → coefficient on X^k.
    pub biomass: IndexMap<String, f64>,
    /// For separate members: the member whose reactions `reactions` names.
    pub species: Option<String>,
}

/// One target's range.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetRange {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub min_status: LpStatus,
    pub max_status: LpStatus,
    pub min_relaxed: bool,
    pub max_relaxed: bool,
    pub min_fluxes: IndexMap<String, f64>,
    pub max_fluxes: IndexMap<String, f64>,
}

/// The result of [`steady_com_fva`].
#[derive(Debug, Clone, PartialEq)]
pub struct SteadyComFvaResult {
    pub status: SteadyComStatus,
    pub growth_rate: f64,
    pub abundances: IndexMap<String, f64>,
    pub ranges: Vec<TargetRange>,
    pub relaxations: u32,
    pub unsolved: Vec<String>,
    pub iterations: u32,
    pub message: String,
}

/// Flux variability at community steady state: the range each target can take
/// while the community still grows at its maximum rate.
///
/// This is what SteadyComFVA does. The growth rate is found first, then
/// fixed, and each target minimised and maximised with the total biomass still
/// fixed at one: the paper's optGRpercent = 100 and optBMpercent = 100.
/// `growth_fraction` holds the community at that fraction of the maximum.
pub fn steady_com_fva(
    spec: &SteadyComSpec,
    targets: &[SteadyComTarget],
    growth_fraction: f64,
    options: SteadyComOptions<'_>,
) -> Result<SteadyComFvaResult, SteadyComError> {
    let tolerance = options.tolerance;
    let backend: &dyn LpBackend = options.backend.unwrap_or(&HighsBackend);
    let cancel = options.cancel.clone();
    let base = steady_com(spec, options)?;
    if base.status != SteadyComStatus::Optimal {
        return Ok(SteadyComFvaResult {
            status: base.status,
            growth_rate: 0.0,
            abundances: IndexMap::new(),
            ranges: Vec::new(),
            relaxations: 0,
            unsolved: Vec::new(),
            iterations: base.iterations,
            message: base.message,
        });
    }
    let total_biomass = spec.total_biomass;
    let mu = base.growth_rate * growth_fraction;
    // The ranges are taken with the total biomass fixed, the paper's
    // optBMpercent = 100, whatever cap the growth-rate search used.
    let mut problem = build_any(spec, None)?;

    // Held at exactly the maximum growth rate, the feasible set is a sliver:
    // the bisection found this mu as the largest it could verify, and one
    // tolerance further would have been infeasible. Some of these solves land
    // on the wrong side of that edge and come back non-optimal. Backing mu off
    // by the bisection tolerance (the precision to which the growth rate is
    // known in the first place) moves the slice off the edge without moving it
    // anywhere the search had not already verified. Each range records whether
    // it needed that, so a relaxed answer never passes for an unrelaxed one.
    let relaxed = (mu - tolerance).max(0.0);
    let mut ranges = Vec::new();
    let mut relaxations = 0;
    for target in targets {
        let entries = target_entries(&problem, target)?;
        let mut solve = |maximize: bool, problem: &mut Problem| -> Result<(AtResult, bool), SteadyComError> {
            let objective = LinearObjective { entries: entries.clone(), maximize };
            let at = solve_at(problem, mu, total_biomass, Some(&objective), backend, cancel.as_ref());
            check_cancel(cancel.as_ref())?;
            if at.status == LpStatus::Optimal {
                return Ok((at, false));
            }
            let retry = solve_at(problem, relaxed, total_biomass, Some(&objective), backend, cancel.as_ref());
            check_cancel(cancel.as_ref())?;
            if retry.status == LpStatus::Optimal {
                relaxations += 1;
            }
            Ok((retry, true))
        };
        let (min, min_relaxed) = solve(false, &mut problem)?;
        let (max, max_relaxed) = solve(true, &mut problem)?;
        ranges.push(TargetRange {
            name: target.name.clone(),
            min: if min.status == LpStatus::Optimal { min.objective.unwrap_or(f64::NAN) } else { f64::NAN },
            max: if max.status == LpStatus::Optimal { max.objective.unwrap_or(f64::NAN) } else { f64::NAN },
            min_status: min.status,
            max_status: max.status,
            min_relaxed,
            max_relaxed,
            min_fluxes: min.fluxes,
            max_fluxes: max.fluxes,
        });
    }
    let unsolved: Vec<String> = ranges.iter().filter(|r| r.min.is_nan() || r.max.is_nan()).map(|r| r.name.clone()).collect();
    let message = if unsolved.is_empty() {
        String::new()
    } else {
        format!("{} of {} targets did not solve even relaxed: {}", unsolved.len(), targets.len(), unsolved.join(", "))
    };
    Ok(SteadyComFvaResult {
        status: SteadyComStatus::Optimal,
        growth_rate: base.growth_rate,
        abundances: base.abundances,
        ranges,
        relaxations,
        unsolved,
        iterations: base.iterations,
        message,
    })
}

fn target_entries(p: &Problem, target: &SteadyComTarget) -> Result<Vec<(usize, f64)>, SteadyComError> {
    let mut entries = Vec::new();
    for (rid, coef) in &target.reactions {
        let col = p.flux_cols.get(rid).or_else(|| {
            let species = target.species.as_deref().unwrap_or("undefined");
            p.flux_cols.get(&key(species, rid))
        });
        let Some(&col) = col else {
            return invalid(format!("target {}: no reaction {rid}", target.name));
        };
        entries.push((col, *coef));
    }
    for (id, coef) in &target.biomass {
        let Some(&col) = p.biomass_cols.get(id) else {
            return invalid(format!("target {}: no member {id}", target.name));
        };
        entries.push((col, *coef));
    }
    Ok(entries)
}
