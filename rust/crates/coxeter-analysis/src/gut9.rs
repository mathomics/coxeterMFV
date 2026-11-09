//! Port of `src/communities/gut9.js`.
//!
//! Published community: the nine-species human gut microbiota model (SteadyCom).
//!
//! > Chan SHJ, Simons MN, Maranas CD (2017) SteadyCom: Predicting microbial
//! > abundances while ensuring community stability.
//! > PLoS Comput Biol 13(5): e1005539.  doi:10.1371/journal.pcbi.1005539
//!
//! Built from the authors' own distribution of the model, which arrives as a
//! workbook rather than SBML:
//!
//! ```text
//! model9.xlsx              sheets: rxns, mets, stoich, comEx, comSp
//! model9_readme.txt        what each field of the COBRA structure holds
//! model9_rxns_params.txt   rev / lb / ub / c per reaction
//! model9_stoich.txt        the stoichiometric matrix as triples
//! simFigure4.m             the Figure 4 experiment
//! ```
//!
//! Unlike the four-auxotroph community, this model already carries its community
//! structure, so none of it is reconstructed here:
//!
//! ```text
//! a species metabolite is `x[c_Bt]`, a community metabolite is `x[u]`
//! EXsp   `EX_x[u]tr_Bt`   -1 x[e_Bt] +1 x[u]   species <-> community medium
//! EXcom  `UT_x(u)`        +1 x[u]              environment -> community medium
//!        `EX_x(u)`        -1 x[u]              community medium -> environment
//! ```
//!
//! So a positive EXsp flux is secretion into the shared medium and a negative one
//! is uptake from it, and the diet is the upper bound on the UT reactions. The
//! EXsp names are not derivable: Bt uses `EX_x[u]tr_Bt`, Fp uses `IEX_x[u]tr_Fp`
//! and Ec uses `EX_x(e)_Ec`, so every one is read from the comEx sheet.
//!
//! # The diet, and why it comes from the TXT
//!
//! In model9.xlsx all 558 UT reactions are bounded [0, 0]: the workbook export
//! carries no diet, and a community with every uptake closed cannot grow at all.
//! model9_rxns_params.txt, exported from the same model, carries the diet on
//! exactly 88 of those UT reactions and agrees with the workbook on all 10921
//! other reactions, objective coefficients included.
//!
//! The bounds therefore come from the TXT, and that is the one place where the
//! workbook is not the primary source. This is not a medium built here: it is the
//! authors' own export of their own bounds, and the alternative is a model that
//! is structurally complete and biologically dead. The import refuses to proceed
//! silently either way: see [`validate_bounds`], which reports every difference.
//!
//! The Figure 4 experiment (`gut9-figure4.js`) is ported in
//! `coxeter_solver::gut9_figure4`, and [`Gut9Community`] implements its
//! `Gut9Model` trait so `fiber_constraints` and `figure_targets` take it as
//! the JS functions take the imported model.

use std::collections::HashSet;

use coxeter_core::js;
use coxeter_io::xlsx::{Row, Workbook, read_workbook};
use coxeter_solver::gut9_figure4::Gut9Model;
use coxeter_solver::steadycom::{Constraint, JointCommunity, JointReaction, Member, SteadyComSpec};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::jsutil::{json_number, number, split_lines};

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Gut9, message)
}

fn io_fail(e: coxeter_io::Error) -> Error {
    fail(e.message)
}

/// The paper and the five files the work is allowed to draw on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gut9Provenance {
    pub paper: Gut9Paper,
    pub files: Gut9Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gut9Paper {
    pub title: &'static str,
    pub authors: &'static str,
    pub year: u32,
    pub journal: &'static str,
    pub doi: &'static str,
    pub framework: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gut9Files {
    pub model: &'static str,
    pub readme: &'static str,
    pub bounds: &'static str,
    pub stoichiometry: &'static str,
    pub experiment: &'static str,
}

impl Gut9Files {
    /// `Object.values(PROVENANCE.files)`.
    pub fn all(&self) -> [&'static str; 5] {
        [self.model, self.readme, self.bounds, self.stoichiometry, self.experiment]
    }
}

pub const PROVENANCE: Gut9Provenance = Gut9Provenance {
    paper: Gut9Paper {
        title: "SteadyCom: Predicting microbial abundances while ensuring community stability",
        authors: "Chan SHJ, Simons MN, Maranas CD",
        year: 2017,
        journal: "PLoS Computational Biology",
        doi: "10.1371/journal.pcbi.1005539",
        framework: "SteadyCom",
    },
    files: Gut9Files {
        model: "model9.xlsx",
        readme: "model9_readme.txt",
        bounds: "model9_rxns_params.txt",
        stoichiometry: "model9_stoich.txt",
        experiment: "simFigure4.m",
    },
};

/// Sizes the workbook itself declares, used as assertions rather than guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gut9Expected {
    pub species: usize,
    pub reactions: usize,
    pub metabolites: usize,
    pub stoichiometry_entries: usize,
    pub community_metabolites: usize,
    /// 558 uptake + 558 export.
    pub community_exchange_reactions: usize,
    /// The sum of comSp's #exchange column.
    pub species_exchange_reactions: usize,
    /// UT reactions the TXT opens.
    pub diet_reactions: usize,
}

pub const EXPECTED: Gut9Expected = Gut9Expected {
    species: 9,
    reactions: 11009,
    metabolites: 8160,
    stoichiometry_entries: 42420,
    community_metabolites: 558,
    community_exchange_reactions: 1116,
    species_exchange_reactions: 1355,
    diet_reactions: 88,
};

// Import

/// One member, from the comSp sheet. Cells the sheet leaves empty are `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Gut9Species {
    pub id: String,
    pub name: Option<String>,
    pub biomass_reaction: Option<String>,
    pub atp_maintenance: Option<String>,
    pub declared_metabolites: f64,
    pub declared_reactions: f64,
    pub declared_exchanges: f64,
}

/// One metabolite, from the mets sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Gut9Metabolite {
    pub id: String,
    pub name: String,
    pub formula: String,
    pub charge: f64,
    /// The member it belongs to, or `comm` for a community metabolite.
    pub species: Option<String>,
}

/// One reaction, from the rxns sheet, with its stoichiometry from stoich.
#[derive(Debug, Clone, PartialEq)]
pub struct Gut9Reaction {
    pub id: String,
    pub name: String,
    pub subsystem: String,
    pub metabolites: IndexMap<String, f64>,
    pub reversible: bool,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub objective: f64,
    /// The member it belongs to, or `comm` for a community exchange reaction.
    pub species: Option<String>,
}

/// A bound the TXT export changed.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundChange {
    pub id: String,
    pub species: Option<String>,
    pub from: [f64; 2],
    pub to: [f64; 2],
}

/// The joint community model as read from the workbook (`importGut9`'s result).
#[derive(Debug, Clone, PartialEq)]
pub struct Gut9Community {
    pub species: Vec<Gut9Species>,
    pub metabolites: IndexMap<String, Gut9Metabolite>,
    pub reactions: IndexMap<String, Gut9Reaction>,
    pub community_metabolites: Vec<String>,
    /// Community metabolite → its UT (environment → medium) reaction.
    pub community_uptake: IndexMap<String, String>,
    /// Community metabolite → its EX (medium → environment) reaction.
    pub community_export: IndexMap<String, String>,
    /// `"<metabolite>\0<species>"` → the species-to-medium exchange reaction.
    pub species_exchange: IndexMap<String, String>,
    /// The species order of comEx's EXsp columns.
    pub exsp_order: Vec<String>,
    pub stoichiometry_entries: usize,
    /// `model9.xlsx`, or `model9_rxns_params.txt` once [`apply_bounds`] ran.
    pub bounds_from: String,
    /// Set by [`apply_bounds`].
    pub bound_changes: Option<Vec<BoundChange>>,
    pub bounds_missing: Option<Vec<String>>,
}

fn opt_str(v: &Option<String>) -> Value {
    v.as_ref().map_or(Value::Null, |s| json!(s))
}

impl Gut9Community {
    /// The JS model with its Maps as objects (keys that are absent in the JS
    /// object, because a cell was empty, are left out).
    pub fn to_json(&self) -> Value {
        let species: Vec<Value> = self
            .species
            .iter()
            .map(|s| {
                let mut o = Map::new();
                o.insert("id".into(), json!(s.id));
                for (k, v) in [("name", &s.name), ("biomassReaction", &s.biomass_reaction), ("atpMaintenance", &s.atp_maintenance)] {
                    if v.is_some() {
                        o.insert(k.into(), opt_str(v));
                    }
                }
                o.insert("declaredMetabolites".into(), json_number(s.declared_metabolites));
                o.insert("declaredReactions".into(), json_number(s.declared_reactions));
                o.insert("declaredExchanges".into(), json_number(s.declared_exchanges));
                Value::Object(o)
            })
            .collect();
        let metabolites: Map<String, Value> = self
            .metabolites
            .iter()
            .map(|(k, m)| {
                let mut o = Map::new();
                o.insert("id".into(), json!(m.id));
                o.insert("name".into(), json!(m.name));
                o.insert("formula".into(), json!(m.formula));
                o.insert("charge".into(), json_number(m.charge));
                if m.species.is_some() {
                    o.insert("species".into(), opt_str(&m.species));
                }
                (k.clone(), Value::Object(o))
            })
            .collect();
        let reactions: Map<String, Value> = self.reactions.iter().map(|(k, r)| (k.clone(), reaction_json(r))).collect();
        let mut out = json!({
            "species": species,
            "metabolites": metabolites,
            "reactions": reactions,
            "communityMetabolites": self.community_metabolites,
            "communityUptake": self.community_uptake,
            "communityExport": self.community_export,
            "speciesExchange": self.species_exchange,
            "exspOrder": self.exsp_order,
            "stoichiometryEntries": self.stoichiometry_entries,
            "boundsFrom": self.bounds_from,
        });
        if let Some(changes) = &self.bound_changes {
            out["boundChanges"] = Value::Array(changes.iter().map(bound_change_json).collect());
        }
        if let Some(missing) = &self.bounds_missing {
            out["boundsMissing"] = json!(missing);
        }
        out
    }
}

fn reaction_json(r: &Gut9Reaction) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(r.id));
    o.insert("name".into(), json!(r.name));
    o.insert("subsystem".into(), json!(r.subsystem));
    o.insert("metabolites".into(), Value::Object(r.metabolites.iter().map(|(k, v)| (k.clone(), json_number(*v))).collect()));
    o.insert("reversible".into(), json!(r.reversible));
    o.insert("lowerBound".into(), json_number(r.lower_bound));
    o.insert("upperBound".into(), json_number(r.upper_bound));
    o.insert("objective".into(), json_number(r.objective));
    if r.species.is_some() {
        o.insert("species".into(), opt_str(&r.species));
    }
    Value::Object(o)
}

fn bound_change_json(c: &BoundChange) -> Value {
    json!({
        "id": c.id, "species": c.species,
        "from": [json_number(c.from[0]), json_number(c.from[1])],
        "to": [json_number(c.to[0]), json_number(c.to[1])],
    })
}

/// `Number(row.X ?? fallback)` for a cell that holds text.
fn cell_number(row: &Row, column: &str, fallback: f64) -> f64 {
    row.cells.get(column).map_or(fallback, |v| number(v))
}

fn cell(row: &Row, column: &str) -> Option<String> {
    row.cells.get(column).cloned()
}

fn sheet_rows(wb: &Workbook, name: &str) -> Result<Vec<Row>> {
    wb.rows(name).map_err(io_fail)?.map(|r| r.map_err(io_fail)).collect()
}

/// Options of [`import_gut9`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Gut9ImportOptions<'a> {
    /// Contents of model9_rxns_params.txt. When given, its lb/ub replace the
    /// workbook's, which is how the diet gets in. Without it the community is
    /// closed and cannot grow.
    pub bounds_text: Option<&'a str>,
}

/// Read the workbook (the bytes of model9.xlsx) into a joint community model.
pub fn import_gut9(xlsx: &[u8], options: &Gut9ImportOptions<'_>) -> Result<Gut9Community> {
    let wb = read_workbook(xlsx.to_vec()).map_err(io_fail)?;
    let not_header = |row: &Row| row.row != 1.0 && row.cells.get("A").is_some_and(|a| !a.is_empty());

    // Species
    let mut species = Vec::new();
    for row in sheet_rows(&wb, "comSp")?.iter().filter(|r| not_header(r)) {
        species.push(Gut9Species {
            id: row.cells["A"].clone(),
            name: cell(row, "B"),
            biomass_reaction: cell(row, "C"),
            atp_maintenance: cell(row, "D"),
            declared_metabolites: cell_number(row, "E", f64::NAN),
            declared_reactions: cell_number(row, "F", f64::NAN),
            declared_exchanges: cell_number(row, "G", f64::NAN),
        });
    }

    // Metabolites
    let mut metabolites = IndexMap::new();
    for row in sheet_rows(&wb, "mets")?.iter().filter(|r| not_header(r)) {
        let id = row.cells["A"].clone();
        metabolites.insert(
            id.clone(),
            Gut9Metabolite {
                name: cell(row, "B").unwrap_or_else(|| id.clone()),
                formula: cell(row, "C").unwrap_or_default(),
                charge: cell_number(row, "D", 0.0),
                // 'comm' for a community metabolite
                species: cell(row, "J"),
                id,
            },
        );
    }

    // Reactions
    // The rxns sheet has trailing rows with no identifier. A row without one is
    // not a reaction, and counting it would put the total at 11540.
    let mut reactions = IndexMap::new();
    for row in sheet_rows(&wb, "rxns")?.iter().filter(|r| not_header(r)) {
        let id = row.cells["A"].clone();
        reactions.insert(
            id.clone(),
            Gut9Reaction {
                name: cell(row, "B").unwrap_or_else(|| id.clone()),
                subsystem: cell(row, "C").unwrap_or_default(),
                metabolites: IndexMap::new(),
                reversible: cell_number(row, "J", 0.0) != 0.0,
                lower_bound: cell_number(row, "K", 0.0),
                upper_bound: cell_number(row, "L", 0.0),
                objective: cell_number(row, "M", 0.0),
                // 'comm' for a community exchange reaction
                species: cell(row, "N"),
                id,
            },
        );
    }

    // Stoichiometry
    let mut stoichiometry_entries = 0;
    for row in sheet_rows(&wb, "stoich")?.iter().filter(|r| not_header(r)) {
        let met = &row.cells["A"];
        let rxn_id = row.cells.get("B");
        let Some(rxn) = rxn_id.and_then(|id| reactions.get_mut(id)) else {
            return Err(fail(format!(
                "stoich names reaction {}, which the rxns sheet does not have",
                rxn_id.map_or("undefined", String::as_str)
            )));
        };
        if !metabolites.contains_key(met) {
            return Err(fail(format!("stoich names metabolite {met}, which the mets sheet does not have")));
        }
        *rxn.metabolites.entry(met.clone()).or_insert(0.0) += cell_number(row, "C", f64::NAN);
        stoichiometry_entries += 1;
    }
    for rxn in reactions.values_mut() {
        js::js_key_order(&mut rxn.metabolites);
    }

    // The community exchange space
    // Row 2 of comEx holds the species abbreviation for each EXsp column, in the
    // order the paper's EXsp(:,k) uses. It is read rather than assumed, because
    // simFigure4.m addresses those columns by index.
    let mut community_metabolites = Vec::new();
    let mut community_uptake = IndexMap::new();
    let mut community_export = IndexMap::new();
    let mut species_exchange = IndexMap::new();
    let mut exsp_columns: Option<Vec<(String, String)>> = None;

    for row in sheet_rows(&wb, "comEx")? {
        if row.row == 1.0 {
            continue;
        }
        if row.row == 2.0 {
            exsp_columns = Some(row.cells.iter().filter(|(_, v)| !v.is_empty()).map(|(c, s)| (c.clone(), s.clone())).collect());
            continue;
        }
        let Some(met) = row.cells.get("A").filter(|a| !a.is_empty()) else { continue };
        community_metabolites.push(met.clone());
        if let Some(ut) = row.cells.get("B").filter(|v| !v.is_empty()) {
            community_uptake.insert(met.clone(), ut.clone());
        }
        if let Some(ex) = row.cells.get("C").filter(|v| !v.is_empty()) {
            community_export.insert(met.clone(), ex.clone());
        }
        for (column, species_id) in exsp_columns.iter().flatten() {
            if let Some(rxn) = row.cells.get(column).filter(|v| !v.is_empty()) {
                species_exchange.insert(format!("{met}\u{0}{species_id}"), rxn.clone());
            }
        }
    }

    let mut model = Gut9Community {
        species,
        metabolites,
        reactions,
        community_metabolites,
        community_uptake,
        community_export,
        species_exchange,
        exsp_order: exsp_columns.iter().flatten().map(|(_, s)| s.clone()).collect(),
        stoichiometry_entries,
        bounds_from: "model9.xlsx".into(),
        bound_changes: None,
        bounds_missing: None,
    };

    if let Some(text) = options.bounds_text.filter(|t| !t.is_empty()) {
        apply_bounds(&mut model, text)?;
    }
    Ok(model)
}

// The diet

/// One line of model9_rxns_params.txt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gut9Bounds {
    pub reversible: bool,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub objective: f64,
}

impl Gut9Bounds {
    pub fn to_json(&self) -> Value {
        json!({
            "reversible": self.reversible, "lowerBound": json_number(self.lower_bound),
            "upperBound": json_number(self.upper_bound), "objective": json_number(self.objective),
        })
    }
}

/// Parse model9_rxns_params.txt into id → { rev, lb, ub, c }.
pub fn parse_bounds(text: &str) -> Result<IndexMap<String, Gut9Bounds>> {
    let mut lines = split_lines(text);
    let header = lines.next().unwrap_or("");
    if header.split('\t').next() != Some("rxns") {
        return Err(fail("model9_rxns_params.txt: unexpected header"));
    }
    let mut out = IndexMap::new();
    for line in lines {
        if crate::jsutil::js_trim(line).is_empty() {
            continue;
        }
        let mut f = line.split('\t');
        let id = f.next().unwrap_or("").to_string();
        let mut next = || f.next().map_or(f64::NAN, number);
        let (rev, lb, ub, c) = (next(), next(), next(), next());
        out.insert(id, Gut9Bounds { reversible: rev != 0.0, lower_bound: lb, upper_bound: ub, objective: c });
    }
    Ok(out)
}

/// Replace the workbook's bounds with the ones the TXT export carries, which is
/// where the authors' diet is. Every change is recorded, so nothing moves
/// silently: the caller gets the list and the tests assert it is the 88 diet
/// reactions and nothing else.
pub fn apply_bounds(model: &mut Gut9Community, bounds_text: &str) -> Result<Vec<BoundChange>> {
    let bounds = parse_bounds(bounds_text)?;
    let mut changed = Vec::new();
    let mut missing = Vec::new();

    for (id, rxn) in model.reactions.iter_mut() {
        let Some(b) = bounds.get(id) else {
            missing.push(id.clone());
            continue;
        };
        if b.lower_bound != rxn.lower_bound || b.upper_bound != rxn.upper_bound {
            changed.push(BoundChange {
                id: id.clone(),
                species: rxn.species.clone(),
                from: [rxn.lower_bound, rxn.upper_bound],
                to: [b.lower_bound, b.upper_bound],
            });
            rxn.lower_bound = b.lower_bound;
            rxn.upper_bound = b.upper_bound;
        }
    }

    model.bounds_from = "model9_rxns_params.txt".into();
    model.bound_changes = Some(changed.clone());
    model.bounds_missing = Some(missing);
    Ok(changed)
}

// Validation

/// A reaction whose bounds or objective differ between the model and the TXT.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundsDifference {
    pub id: String,
    /// `[lb, ub, c]` in the model.
    pub model: [f64; 3],
    /// `[lb, ub, c]` in the TXT.
    pub text: [f64; 3],
}

/// [`validate_bounds`]'s report.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundsValidation {
    pub identical: usize,
    pub differing: Vec<BoundsDifference>,
    pub only_in_model: Vec<String>,
    pub only_in_text: Vec<String>,
}

impl BoundsValidation {
    pub fn to_json(&self) -> Value {
        json!({
            "identical": self.identical,
            "differing": self.differing.iter().map(|d| json!({
                "id": d.id, "model": d.model.map(json_number), "text": d.text.map(json_number),
            })).collect::<Vec<_>>(),
            "onlyInModel": self.only_in_model,
            "onlyInText": self.only_in_text,
        })
    }
}

/// Check the imported bounds against model9_rxns_params.txt.
pub fn validate_bounds(model: &Gut9Community, bounds_text: &str) -> Result<BoundsValidation> {
    let bounds = parse_bounds(bounds_text)?;
    let mut identical = 0;
    let mut differing = Vec::new();
    let mut only_in_model = Vec::new();

    for (id, rxn) in &model.reactions {
        let Some(b) = bounds.get(id) else {
            only_in_model.push(id.clone());
            continue;
        };
        if b.lower_bound == rxn.lower_bound && b.upper_bound == rxn.upper_bound && b.objective == rxn.objective {
            identical += 1;
        } else {
            differing.push(BoundsDifference {
                id: id.clone(),
                model: [rxn.lower_bound, rxn.upper_bound, rxn.objective],
                text: [b.lower_bound, b.upper_bound, b.objective],
            });
        }
    }

    let only_in_text = bounds.keys().filter(|id| !model.reactions.contains_key(*id)).cloned().collect();
    Ok(BoundsValidation { identical, differing, only_in_model, only_in_text })
}

/// A coefficient that differs between the model and model9_stoich.txt.
#[derive(Debug, Clone, PartialEq)]
pub struct StoichiometryDifference {
    pub reaction: String,
    pub metabolite: String,
    pub model: f64,
    pub text: f64,
}

/// [`validate_stoichiometry`]'s report.
#[derive(Debug, Clone, PartialEq)]
pub struct StoichiometryValidation {
    pub entries: usize,
    pub identical: usize,
    pub differing: Vec<StoichiometryDifference>,
    /// `reaction:metabolite` entries the TXT has and the model does not.
    pub only_in_text: Vec<String>,
    pub only_in_model: Vec<String>,
}

impl StoichiometryValidation {
    pub fn to_json(&self) -> Value {
        json!({
            "entries": self.entries,
            "identical": self.identical,
            "differing": self.differing.iter().map(|d| json!({
                "reaction": d.reaction, "metabolite": d.metabolite, "model": json_number(d.model), "text": json_number(d.text),
            })).collect::<Vec<_>>(),
            "onlyInText": self.only_in_text,
            "onlyInModel": self.only_in_model,
        })
    }
}

/// Check the imported stoichiometric matrix against model9_stoich.txt, entry by
/// entry and in both directions, so neither a missing nor an extra coefficient
/// can pass.
pub fn validate_stoichiometry(model: &Gut9Community, stoich_text: &str) -> Result<StoichiometryValidation> {
    let mut lines = split_lines(stoich_text);
    if lines.next().unwrap_or("").split('\t').next() != Some("mets") {
        return Err(fail("model9_stoich.txt: unexpected header"));
    }

    let mut entries = 0;
    let mut identical = 0;
    let mut differing = Vec::new();
    let mut only_in_text = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for line in lines {
        if crate::jsutil::js_trim(line).is_empty() {
            continue;
        }
        let mut f = line.split('\t');
        let met = f.next().unwrap_or("");
        let rxn_id = f.next();
        let coef = f.next();
        entries += 1;
        let value = rxn_id.and_then(|r| model.reactions.get(r)).and_then(|r| r.metabolites.get(met)).copied();
        let rxn_text = rxn_id.unwrap_or("undefined");
        let Some(value) = value else {
            only_in_text.push(format!("{rxn_text}:{met}"));
            continue;
        };
        seen.insert((rxn_text.to_string(), met.to_string()));
        let text = coef.map_or(f64::NAN, number);
        // The TXT prints twelve decimals, so compare on value, not on spelling.
        if (value - text).abs() < 1e-9 {
            identical += 1;
        } else {
            differing.push(StoichiometryDifference { reaction: rxn_text.to_string(), metabolite: met.to_string(), model: value, text });
        }
    }

    let mut only_in_model = Vec::new();
    for (id, rxn) in &model.reactions {
        for met in rxn.metabolites.keys() {
            if !seen.contains(&(id.clone(), met.clone())) {
                only_in_model.push(format!("{id}:{met}"));
            }
        }
    }

    Ok(StoichiometryValidation { entries, identical, differing, only_in_text, only_in_model })
}

// Derived accessors

/// Reactions that belong to a species, as `species id -> reaction ids`.
pub fn reactions_by_species(model: &Gut9Community) -> Result<IndexMap<String, Vec<String>>> {
    let mut out: IndexMap<String, Vec<String>> = model.species.iter().map(|s| (s.id.clone(), Vec::new())).collect();
    for (id, rxn) in &model.reactions {
        if rxn.species.as_deref() == Some("comm") {
            continue;
        }
        let Some(list) = rxn.species.as_ref().and_then(|s| out.get_mut(s)) else {
            return Err(fail(format!(
                "reaction {id} belongs to unknown species {}",
                rxn.species.as_deref().unwrap_or("undefined")
            )));
        };
        list.push(id.clone());
    }
    Ok(out)
}

/// The 1116 community exchange reactions, which belong to no species.
pub fn community_reaction_ids(model: &Gut9Community) -> Vec<String> {
    model.reactions.values().filter(|r| r.species.as_deref() == Some("comm")).map(|r| r.id.clone()).collect()
}

/// What the diet actually supplies: community metabolite -> UT upper bound,
/// when positive.
pub fn diet(model: &Gut9Community) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    for (met, rxn_id) in &model.community_uptake {
        let ub = model.reactions.get(rxn_id).map_or(0.0, |r| r.upper_bound);
        if ub > 0.0 {
            out.insert(met.clone(), ub);
        }
    }
    out
}

/// Options of [`to_steady_com_spec`].
#[derive(Debug, Clone)]
pub struct Gut9SpecOptions {
    /// 1 gDW unless stated.
    pub total_biomass: f64,
    /// Extra linear constraints (the Figure 4 fiber limits).
    pub constraints: Vec<Constraint>,
}

impl Default for Gut9SpecOptions {
    fn default() -> Self {
        Gut9SpecOptions { total_biomass: 1.0, constraints: Vec::new() }
    }
}

/// Turn the imported model into the spec `steady_com` takes.
///
/// Nothing is reshaped: the reactions, metabolites and bounds go through as they
/// were read, and the species tag on each reaction is what tells the solver which
/// fluxes scale with abundance and which are absolute community rates.
pub fn to_steady_com_spec(model: &Gut9Community, options: &Gut9SpecOptions) -> Result<SteadyComSpec> {
    let mut members = Vec::new();
    for sp in &model.species {
        let Some(biomass) = sp.biomass_reaction.clone() else {
            return Err(fail(format!("{}: comSp names no biomass reaction", sp.id)));
        };
        members.push(Member {
            id: sp.id.clone(),
            name: sp.name.clone(),
            biomass_reaction: biomass,
            atp_maintenance: sp.atp_maintenance.clone(),
            model: None,
            exchange_for: IndexMap::new(),
        });
    }
    let reactions = model
        .reactions
        .iter()
        .map(|(id, r)| {
            (
                id.clone(),
                JointReaction {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    subsystem: r.subsystem.clone(),
                    metabolites: r.metabolites.clone(),
                    species: r.species.clone().unwrap_or_else(|| "undefined".into()),
                    lower_bound: r.lower_bound,
                    upper_bound: r.upper_bound,
                    objective: r.objective,
                },
            )
        })
        .collect();
    let metabolites = model
        .metabolites
        .iter()
        .map(|(id, m)| {
            let mut doc = Map::new();
            doc.insert("id".into(), json!(m.id));
            doc.insert("name".into(), json!(m.name));
            doc.insert("formula".into(), json!(m.formula));
            doc.insert("charge".into(), json_number(m.charge));
            if let Some(s) = &m.species {
                doc.insert("species".into(), json!(s));
            }
            (id.clone(), doc)
        })
        .collect();
    Ok(SteadyComSpec {
        members,
        total_biomass: options.total_biomass,
        biomass_cap: None,
        community_uptake: IndexMap::new(),
        community_exchange_bounds: IndexMap::new(),
        joint: Some(JointCommunity {
            reactions,
            metabolites,
            community_tag: "comm".into(),
            constraints: options.constraints.clone(),
            metadata: None,
        }),
    })
}

impl Gut9Model for Gut9Community {
    fn has_reaction(&self, id: &str) -> bool {
        self.reactions.contains_key(id)
    }

    fn species_exchange(&self) -> &IndexMap<String, String> {
        &self.species_exchange
    }

    fn species(&self) -> Vec<(String, String)> {
        self.species
            .iter()
            .map(|s| (s.id.clone(), s.biomass_reaction.clone().unwrap_or_else(|| "undefined".into())))
            .collect()
    }
}
