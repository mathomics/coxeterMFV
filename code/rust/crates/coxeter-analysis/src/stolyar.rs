//! Port of `src/communities/stolyar.js`.
//!
//! Stolyar et al. 2007: the joint model of a two-species mutualism.
//!
//! The supplement is four plain-text files in CellNetAnalyzer's project format,
//! not SBML and not COBRA JSON, so this reads that format directly rather than
//! retyping the network out of the paper:
//!
//! ```text
//! reactions.txt               id, equation, measured rate, bounds, objective,
//!                             map position and layer
//! metabolites.txt             id, name, and a flag marking the external ones
//! macromolecules.txt          the seven D. vulgaris macromolecules and the
//!                             fraction of dry weight each one contributes
//! macromolecule_synthesis.txt what it costs to make one unit of each
//! ```
//!
//! Two organisms share one stoichiometric matrix. Which reaction belongs to
//! which is in the file: the layer column, 1 for D. vulgaris and 3 for
//! M. maripaludis, with layer 2 holding the methanogen's macromolecule
//! synthesis. The methanogen's metabolites also carry a `ZM_` prefix, and the
//! two agree everywhere. The import checks that rather than trusting it.
//!
//! Four metabolites appear in both organisms' reactions, and they are not
//! declared as shared anywhere: they are found by looking at which side of the
//! model touches them. That is the mutualism, and it is derived, not asserted.
//!
//! Two conventions of this format matter for reading it as a COBRA model:
//!
//! - The equation's `=` separates substrates from products, and an empty side
//!   is allowed: `1 H2(exch) =` is a drain. `mue`, the D. vulgaris growth
//!   reaction, has no equation at all: its stoichiometry is the macromolecule
//!   fractions, which is what the other two files are for.
//! - The measured-rate column is a constraint, not a note. `#` means
//!   unmeasured, and a number fixes that flux. Thirteen reactions are fixed at
//!   zero, `D86` fixes the lactate supply at 10, and `H2gen` fixes the
//!   hydrogen leaving the pair at zero, which is the syntrophy, stated as a
//!   constraint rather than described.
//!
//! In macromolecule_synthesis.txt a positive coefficient is consumed and a
//! negative one produced, the opposite of the usual reading. The methanogen's
//! own synthesis reactions are written out in reactions.txt (MC_RNA and friends)
//! and say the same thing the other way round, which is how that sign
//! convention is confirmed here instead of guessed.

use std::sync::LazyLock;

use coxeter_core::js;
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::jsutil::{WS_CHARS, js_trim, json_number, json_quote, json_quote_prefix, number, numbers_split_ws, split_lines};

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Stolyar, message)
}

/// One organism of the joint model and the layers of the file that are its.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StolyarSpecies {
    pub id: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub layers: &'static [f64],
    pub prefix: Option<&'static str>,
    pub biomass_reaction: &'static str,
}

impl StolyarSpecies {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "short": self.short, "layers": self.layers,
            "prefix": self.prefix, "biomassReaction": self.biomass_reaction,
        })
    }
}

/// Which organism each layer in the file belongs to.
pub const SPECIES: [StolyarSpecies; 2] = [
    StolyarSpecies {
        id: "Dv",
        name: "Desulfovibrio vulgaris Hildenborough",
        short: "D. vulgaris",
        layers: &[1.0],
        prefix: None,
        biomass_reaction: "mue",
    },
    StolyarSpecies {
        id: "Mm",
        name: "Methanococcus maripaludis S2",
        short: "M. maripaludis",
        layers: &[2.0, 3.0],
        prefix: Some("ZM_"),
        biomass_reaction: "MC_BIO",
    },
];

/// What a correct import produces, so a silent change in the files shows up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StolyarExpected {
    pub species: usize,
    pub reactions: usize,
    pub metabolites: usize,
    pub macromolecules: usize,
    pub external_metabolites: usize,
    pub shared_metabolites: usize,
    pub measured_reactions: usize,
    pub objective_reactions: usize,
}

pub const EXPECTED: StolyarExpected = StolyarExpected {
    species: 2,
    reactions: 180,
    metabolites: 163,
    macromolecules: 7,
    external_metabolites: 8,
    shared_metabolites: 4,
    measured_reactions: 15,
    objective_reactions: 3,
};

/// The four files of the supplement, as text.
#[derive(Debug, Clone, Copy)]
pub struct StolyarFiles<'a> {
    pub reactions: &'a str,
    pub metabolites: &'a str,
    pub macromolecules: &'a str,
    pub synthesis: &'a str,
}

/// `String(text).split(/\r?\n/).filter((l) => l.trim())`
fn trimmed_lines(text: &str) -> impl Iterator<Item = &str> {
    split_lines(text).filter(|l| !js_trim(l).is_empty())
}

/// `line.split('\t').map((s) => s.trim())`
fn fields(line: &str) -> Vec<&str> {
    line.split('\t').map(js_trim).collect()
}

/// `${value}` for a number that may be missing.
fn number_text(value: Option<f64>) -> String {
    value.map_or_else(|| "undefined".to_string(), js::number_to_string)
}

/// One term of an equation side.
#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub coefficient: f64,
    pub metabolite: String,
}

/// JS `.` (anything but a line terminator) and `\s`.
static TERM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^([0-9]*\.?[0-9]+)[{WS_CHARS}]+([^\n\r\x{{2028}}\x{{2029}}]+)$")).expect("pattern"));
static SIGNED_TERM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^(-?[0-9]*\.?[0-9]+)[{WS_CHARS}]+([^\n\r\x{{2028}}\x{{2029}}]+)$")).expect("pattern"));
static PLUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"[{WS_CHARS}]+\+[{WS_CHARS}]+")).expect("pattern"));

/// Split one side of an equation into terms.
///
/// The separator is ` + ` with spaces: metabolite ids in this model contain a
/// bare `+` (`H+`, `ZM_H+`), so splitting on the character alone invents two
/// metabolites out of one.
pub fn parse_terms(side: &str) -> Vec<Term> {
    PLUS.split(side)
        .map(js_trim)
        .filter(|t| !t.is_empty())
        .map(|term| match TERM.captures(term) {
            Some(m) => Term { coefficient: number(&m[1]), metabolite: js_trim(&m[2]).to_string() },
            None => Term { coefficient: 1.0, metabolite: term.to_string() },
        })
        .collect()
}

/// `1 A + 2 B = 1 C` into a COBRA stoichiometry map.
pub fn parse_equation(equation: &str) -> IndexMap<String, f64> {
    let mut sides = equation.split('=');
    let left = sides.next().unwrap_or("");
    let right = sides.next().unwrap_or("");
    let mut stoichiometry: IndexMap<String, f64> = IndexMap::new();
    for Term { coefficient, metabolite } in parse_terms(left) {
        *stoichiometry.entry(metabolite).or_insert(0.0) -= coefficient;
    }
    for Term { coefficient, metabolite } in parse_terms(right) {
        *stoichiometry.entry(metabolite).or_insert(0.0) += coefficient;
    }
    stoichiometry.retain(|_, c| *c != 0.0);
    js::js_key_order(&mut stoichiometry);
    stoichiometry
}

/// Where the file draws a reaction (`{x, y}`; a missing number is absent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StolyarPosition {
    pub x: Option<f64>,
    pub y: Option<f64>,
}

/// One reaction of the file (or one derived by the import).
#[derive(Debug, Clone, PartialEq)]
pub struct StolyarReaction {
    pub id: String,
    pub equation: String,
    pub metabolites: IndexMap<String, f64>,
    /// The measured rate, a constraint, or `None` for `#`.
    pub measured: Option<f64>,
    pub lower_bound: f64,
    pub upper_bound: f64,
    /// CellNetAnalyzer's (minimised) objective coefficient, or `None` when the
    /// column has no third number.
    pub objective: Option<f64>,
    pub layer: Option<f64>,
    pub position: Option<StolyarPosition>,
    /// What the import derived it from, for the reactions it builds or fills.
    pub derived: Option<String>,
    /// After import: the organism that owns it, `None` for the pair's boundary.
    pub species: Option<String>,
}

impl StolyarReaction {
    /// The object `parseReactions` yields (without `species`, which the
    /// import adds).
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("equation".into(), json!(self.equation));
        out.insert("metabolites".into(), number_object(&self.metabolites));
        out.insert("measured".into(), self.measured.map_or(Value::Null, json_number));
        out.insert("lowerBound".into(), json_number(self.lower_bound));
        out.insert("upperBound".into(), json_number(self.upper_bound));
        if let Some(o) = self.objective {
            out.insert("objective".into(), json_number(o));
        }
        if let Some(l) = self.layer {
            out.insert("layer".into(), json_number(l));
        }
        out.insert(
            "position".into(),
            match self.position {
                None => Value::Null,
                Some(p) => {
                    let mut pos = Map::new();
                    if let Some(x) = p.x {
                        pos.insert("x".into(), json_number(x));
                    }
                    if let Some(y) = p.y {
                        pos.insert("y".into(), json_number(y));
                    }
                    Value::Object(pos)
                }
            },
        );
        if let Some(d) = &self.derived {
            out.insert("derived".into(), json!(d));
        }
        Value::Object(out)
    }
}

fn number_object(m: &IndexMap<String, f64>) -> Value {
    Value::Object(m.iter().map(|(k, v)| (k.clone(), json_number(*v))).collect())
}

/// One reaction per line. The `|` column is the anchor rather than a fixed field
/// count: `mue` carries an empty equation and shifts everything after it.
pub fn parse_reactions(text: &str) -> Result<IndexMap<String, StolyarReaction>> {
    let mut out = IndexMap::new();
    for line in trimmed_lines(text) {
        let f = fields(line);
        let bar = f.iter().position(|x| *x == "|").filter(|b| *b >= 1);
        let Some(bar) = bar else {
            return Err(fail(format!("reactions: no \"|\" column in {}", json_quote_prefix(line, 40))));
        };
        let id = f[0].to_string();
        let equation = js_trim(&f[1..bar].join(" ")).to_string();
        let measured = f.get(bar + 1).copied();
        let Some(bounds_text) = f.get(bar + 2) else {
            // The JS stops on a TypeError here.
            return Err(fail(format!("reactions: {id} has unreadable bounds undefined")));
        };
        let bounds = numbers_split_ws(bounds_text);
        let (lower_bound, upper_bound) = (bounds.first().copied(), bounds.get(1).copied());
        let objective = bounds.get(2).copied();
        let Some(place_text) = f.get(bar + 3) else {
            // The JS stops on a TypeError here.
            return Err(fail(format!("reactions: {id} has an unreadable map position undefined")));
        };
        let place = numbers_split_ws(place_text);
        let (Some(lower_bound), Some(upper_bound)) = (lower_bound.filter(|v| v.is_finite()), upper_bound.filter(|v| v.is_finite()))
        else {
            return Err(fail(format!("reactions: {id} has unreadable bounds {}", json_quote(bounds_text))));
        };
        let metabolites = parse_equation(&equation);
        out.insert(
            id.clone(),
            StolyarReaction {
                id,
                equation,
                metabolites,
                measured: match measured {
                    Some("#") => None,
                    Some(m) => Some(number(m)),
                    None => Some(f64::NAN),
                },
                lower_bound,
                upper_bound,
                objective,
                layer: place.get(2).copied(),
                position: Some(StolyarPosition { x: place.first().copied(), y: place.get(1).copied() }),
                derived: None,
                species: None,
            },
        );
    }
    Ok(out)
}

/// One metabolite of metabolites.txt (or a macromolecule the import adds).
#[derive(Debug, Clone, PartialEq)]
pub struct StolyarMetabolite {
    pub id: String,
    pub name: String,
    /// Unbalanced in CellNetAnalyzer: free to enter or leave.
    pub external: bool,
    /// Added by the import for a D. vulgaris macromolecule.
    pub macromolecule: bool,
}

impl StolyarMetabolite {
    pub fn to_json(&self) -> Value {
        let mut out = json!({"id": self.id, "name": self.name, "external": self.external});
        if self.macromolecule {
            out["macromolecule"] = json!(true);
        }
        out
    }
}

/// id, name, a tolerance this import does not use, and the external flag.
pub fn parse_metabolites(text: &str) -> IndexMap<String, StolyarMetabolite> {
    let mut out = IndexMap::new();
    for line in trimmed_lines(text) {
        let f = fields(line);
        let id = f[0].to_string();
        let name = f.get(1).filter(|n| !n.is_empty()).map_or_else(|| id.clone(), |n| n.to_string());
        let external = f.get(3) == Some(&"1");
        out.insert(id.clone(), StolyarMetabolite { id, name, external, macromolecule: false });
    }
    out
}

/// One D. vulgaris macromolecule and its share of the dry weight.
#[derive(Debug, Clone, PartialEq)]
pub struct StolyarMacromolecule {
    pub id: String,
    pub name: String,
    pub fraction: f64,
}

impl StolyarMacromolecule {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "fraction": json_number(self.fraction)})
    }
}

/// id, name, mass fraction, then map position.
pub fn parse_macromolecules(text: &str) -> IndexMap<String, StolyarMacromolecule> {
    let mut out = IndexMap::new();
    for line in trimmed_lines(text) {
        let f = fields(line);
        let id = f[0].to_string();
        let name = f.get(1).filter(|n| !n.is_empty()).map_or_else(|| id.clone(), |n| n.to_string());
        let fraction = f.get(2).map_or(f64::NAN, |x| number(x));
        out.insert(id.clone(), StolyarMacromolecule { id, name, fraction });
    }
    out
}

/// `RNA = 6.17 Asp + -3.47 fum + …`, where a positive coefficient is consumed
/// and a negative one produced.
pub fn parse_macromolecule_synthesis(text: &str) -> Result<IndexMap<String, IndexMap<String, f64>>> {
    let mut out = IndexMap::new();
    for line in trimmed_lines(text) {
        let mut parts = line.split('=');
        let name = parts.next().unwrap_or("");
        let body = parts.next().unwrap_or("");
        let id = js_trim(name);
        if id.is_empty() {
            continue;
        }
        let mut stoichiometry: IndexMap<String, f64> = IndexMap::new();
        for term in body.split('+') {
            let t = js_trim(term);
            if t.is_empty() {
                continue;
            }
            let Some(m) = SIGNED_TERM.captures(t) else {
                return Err(fail(format!("macromolecule synthesis: unreadable term {} in {id}", json_quote(t))));
            };
            // Consumed is positive here, so the sign flips to reach COBRA's reading.
            *stoichiometry.entry(js_trim(&m[2]).to_string()).or_insert(0.0) -= number(&m[1]);
        }
        js::js_key_order(&mut stoichiometry);
        out.insert(id.to_string(), stoichiometry);
    }
    Ok(out)
}

/// The joint model as the import leaves it.
#[derive(Debug, Clone, PartialEq)]
pub struct StolyarModel {
    pub metabolites: IndexMap<String, StolyarMetabolite>,
    pub reactions: IndexMap<String, StolyarReaction>,
    pub macromolecules: IndexMap<String, StolyarMacromolecule>,
    /// Metabolites both organisms touch, in the order they were met.
    pub shared: IndexSet<String>,
}

impl StolyarModel {
    /// `SPECIES` (the JS model carries it as `species`).
    pub fn species(&self) -> &'static [StolyarSpecies] {
        &SPECIES
    }

    /// The model with its Maps as objects and `shared` as an array.
    pub fn to_json(&self) -> Value {
        let reactions: Map<String, Value> = self
            .reactions
            .iter()
            .map(|(k, r)| {
                let mut v = r.to_json();
                v["species"] = json!(r.species);
                (k.clone(), v)
            })
            .collect();
        json!({
            "metabolites": Value::Object(self.metabolites.iter().map(|(k, m)| (k.clone(), m.to_json())).collect()),
            "reactions": Value::Object(reactions),
            "macromolecules": Value::Object(self.macromolecules.iter().map(|(k, m)| (k.clone(), m.to_json())).collect()),
            "shared": self.shared.iter().collect::<Vec<_>>(),
            "species": SPECIES.iter().map(StolyarSpecies::to_json).collect::<Vec<_>>(),
        })
    }
}

fn layer_owner(layer: Option<f64>) -> Option<&'static str> {
    let layer = layer?;
    SPECIES.iter().find(|sp| sp.layers.contains(&layer)).map(|sp| sp.id)
}

/// Read the four files into one model.
pub fn import_stolyar(files: &StolyarFiles<'_>) -> Result<StolyarModel> {
    let mut metabolites = parse_metabolites(files.metabolites);
    let mut reactions = parse_reactions(files.reactions)?;
    let macromolecules = parse_macromolecules(files.macromolecules);
    let synthesis = parse_macromolecule_synthesis(files.synthesis)?;

    // The D. vulgaris biomass, which the reaction file leaves empty
    // Each macromolecule becomes a metabolite, its synthesis a reaction, and the
    // growth reaction consumes them in the fractions macromolecules.txt gives.
    // The methanogen's equivalent (MC_BIO) is already written out in the reaction
    // file, and this mirrors it.
    let growth_id = SPECIES[0].biomass_reaction;
    match reactions.get(growth_id) {
        None => return Err(fail(format!("reactions: the growth reaction {growth_id} is missing"))),
        Some(growth) if !growth.metabolites.is_empty() => {
            return Err(fail(format!("reactions: {} was expected to carry no equation of its own", growth.id)));
        }
        Some(_) => {}
    }

    for (id, macro_) in &macromolecules {
        let Some(stoichiometry) = synthesis.get(id) else {
            return Err(fail(format!("macromolecules: {id} has no synthesis equation")));
        };
        metabolites.insert(
            id.clone(),
            StolyarMetabolite { id: id.clone(), name: macro_.name.clone(), external: false, macromolecule: true },
        );
        let mut synthesised = stoichiometry.clone();
        synthesised.insert(id.clone(), 1.0);
        js::js_key_order(&mut synthesised);
        reactions.insert(
            format!("{id}_syn"),
            StolyarReaction {
                id: format!("{id}_syn"),
                equation: String::new(),
                metabolites: synthesised,
                measured: None,
                lower_bound: 0.0,
                upper_bound: 100.0,
                objective: Some(0.0),
                layer: Some(SPECIES[0].layers[0]),
                position: None,
                derived: Some("macromolecule synthesis, from macromolecule_synthesis.txt".into()),
                species: None,
            },
        );
        let growth = reactions.get_mut(growth_id).expect("checked above");
        growth.metabolites.insert(id.clone(), -macro_.fraction);
        js::js_key_order(&mut growth.metabolites);
    }
    reactions.get_mut(growth_id).expect("checked above").derived = Some("biomass composition, from macromolecules.txt".into());

    for rxn in reactions.values() {
        for id in rxn.metabolites.keys() {
            if !metabolites.contains_key(id) {
                return Err(fail(format!("reactions: {} uses {id}, which metabolites.txt does not declare", rxn.id)));
            }
        }
    }

    // Who owns what
    let mut touched_by: IndexMap<String, IndexSet<&'static str>> = IndexMap::new();
    for rxn in reactions.values() {
        let Some(owner) = layer_owner(rxn.layer) else {
            return Err(fail(format!("reactions: {} sits on unknown layer {}", rxn.id, number_text(rxn.layer))));
        };
        for id in rxn.metabolites.keys() {
            touched_by.entry(id.clone()).or_default().insert(owner);
        }
    }

    // Shared: used by both organisms. Nothing in the files says which metabolites
    // these are, so they are read off the model.
    let shared: IndexSet<String> = touched_by.iter().filter(|(_, owners)| owners.len() > 1).map(|(id, _)| id.clone()).collect();

    for rxn in reactions.values_mut() {
        let ids: Vec<&String> = rxn.metabolites.keys().collect();
        // A reaction touching nothing but the shared pool belongs to neither
        // organism: it is the pair's boundary with the rest of the world.
        rxn.species = if !ids.is_empty() && ids.iter().all(|id| shared.contains(*id)) {
            None
        } else {
            layer_owner(rxn.layer).map(str::to_string)
        };

        // The prefix and the layer have to agree, or the layer is not what it looks
        // like and every split below would be wrong.
        let prefixed: Vec<&&String> = ids.iter().filter(|id| id.starts_with("ZM_")).collect();
        if rxn.species.as_deref() == Some("Mm") && !ids.is_empty() && prefixed.is_empty() {
            return Err(fail(format!("reactions: {} is on a methanogen layer but names no ZM_ metabolite", rxn.id)));
        }
        if rxn.species.as_deref() == Some("Dv") && !prefixed.is_empty() {
            return Err(fail(format!("reactions: {} is on the D. vulgaris layer but names {}", rxn.id, prefixed[0])));
        }
    }

    Ok(StolyarModel { metabolites, reactions, macromolecules, shared })
}

/// Metabolites the file marks external: unbalanced, free to enter or leave.
pub fn external_metabolites(model: &StolyarModel) -> Vec<String> {
    model.metabolites.values().filter(|m| m.external).map(|m| m.id.clone()).collect()
}

/// Reaction ids per organism, plus the pair's own boundary reactions.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionsBySpecies {
    pub by_species: IndexMap<String, Vec<String>>,
    pub community: Vec<String>,
}

/// Reaction ids per organism, plus the pair's own boundary reactions.
pub fn reactions_by_species(model: &StolyarModel) -> ReactionsBySpecies {
    let mut by_species: IndexMap<String, Vec<String>> = SPECIES.iter().map(|sp| (sp.id.to_string(), Vec::new())).collect();
    let mut community = Vec::new();
    for rxn in model.reactions.values() {
        match &rxn.species {
            None => community.push(rxn.id.clone()),
            Some(sp) => by_species.entry(sp.clone()).or_default().push(rxn.id.clone()),
        }
    }
    ReactionsBySpecies { by_species, community }
}

/// Who can release and who can take up one shared metabolite.
#[derive(Debug, Clone, PartialEq)]
pub struct ExchangeStructure {
    pub metabolite: String,
    pub name: String,
    pub producers: Vec<String>,
    pub consumers: Vec<String>,
}

impl ExchangeStructure {
    pub fn to_json(&self) -> Value {
        json!({"metabolite": self.metabolite, "name": self.name, "producers": self.producers, "consumers": self.consumers})
    }
}

fn sorted(set: IndexSet<String>) -> Vec<String> {
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort_by(|a, b| js::code_unit_compare(a, b));
    v
}

/// Which organism releases each shared metabolite and which takes it up, read
/// off the stoichiometry rather than written down.
///
/// This is the structural picture: who *can* exchange what. What actually moves
/// is a question for the fluxes, and the app computes that from a solution.
pub fn exchange_structure(model: &StolyarModel) -> Vec<ExchangeStructure> {
    let mut ids: Vec<&String> = model.shared.iter().collect();
    ids.sort_by(|a, b| js::code_unit_compare(a, b));
    ids.into_iter()
        .map(|id| {
            let mut producers = IndexSet::new();
            let mut consumers = IndexSet::new();
            for rxn in model.reactions.values() {
                let coefficient = rxn.metabolites.get(id).copied().unwrap_or(0.0);
                let Some(species) = rxn.species.as_ref().filter(|s| !s.is_empty()) else { continue };
                if coefficient == 0.0 || coefficient.is_nan() {
                    continue;
                }
                // A reversible reaction can do either, so it counts as both.
                if coefficient > 0.0 || rxn.lower_bound < 0.0 {
                    producers.insert(species.clone());
                }
                if coefficient < 0.0 || rxn.lower_bound < 0.0 {
                    consumers.insert(species.clone());
                }
            }
            ExchangeStructure {
                metabolite: id.clone(),
                name: model.metabolites.get(id).map_or_else(|| id.clone(), |m| m.name.clone()),
                producers: sorted(producers),
                consumers: sorted(consumers),
            }
        })
        .collect()
}

fn compartment_names() -> Value {
    json!({"c": "Cytosol", "e": "Extracellular"})
}

/// Shared and external metabolites sit outside the cell, everything else in it.
fn compartment_of(model: &StolyarModel, id: &str) -> &'static str {
    if model.shared.contains(id) || model.metabolites.get(id).is_some_and(|m| m.external) { "e" } else { "c" }
}

fn metabolite_name<'a>(model: &'a StolyarModel, id: &'a str) -> &'a str {
    model.metabolites.get(id).map_or(id, |m| m.name.as_str())
}

/// Options of [`to_joint_cobra_json`].
#[derive(Debug, Clone)]
pub struct JointOptions {
    pub id: String,
    pub name: String,
}

impl Default for JointOptions {
    fn default() -> Self {
        JointOptions { id: "stolyar2007".into(), name: "Stolyar 2007 joint model".into() }
    }
}

/// The whole two-species model as one COBRA document, the file's own shape.
///
/// External metabolites are unbalanced in CellNetAnalyzer, which a COBRA model
/// has no notion of, so each one gets an unconstrained exchange reaction. That
/// is the faithful translation of "free to enter or leave", and it is the only
/// reaction this adds.
pub fn to_joint_cobra_json(model: &StolyarModel, options: &JointOptions) -> Value {
    let mut reactions: Vec<Value> = model.reactions.values().map(to_cobra_reaction).collect();
    for met in external_metabolites(model) {
        reactions.push(json!({
            "id": format!("EX_{met}"),
            "name": format!("{} exchange", metabolite_name(model, &met)),
            "metabolites": {met.clone(): -1},
            "lower_bound": -1000,
            "upper_bound": 1000,
            "subsystem": "Exchange",
        }));
    }
    json!({
        "id": options.id,
        "name": options.name,
        "compartments": compartment_names(),
        "metabolites": model.metabolites.values().map(|met| json!({
            "id": met.id, "name": met.name, "compartment": compartment_of(model, &met.id), "formula": "", "charge": 0,
        })).collect::<Vec<_>>(),
        "reactions": reactions,
        "genes": [],
        "objective": number_object(&objective_of(model)),
    })
}

/// CellNetAnalyzer minimises its objective. This project maximises, so the
/// coefficients change sign. The file asks for growth of both organisms (the
/// methanogen at a tenth of the weight) against the cost of running ATP
/// maintenance, and that weighting is the paper's, not this project's.
pub fn objective_of(model: &StolyarModel) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    for rxn in model.reactions.values() {
        if let Some(o) = rxn.objective.filter(|o| *o != 0.0 && !o.is_nan()) {
            out.insert(rxn.id.clone(), -o);
        }
    }
    js::js_key_order(&mut out);
    out
}

fn to_cobra_reaction(rxn: &StolyarReaction) -> Value {
    // A measured rate is a constraint in this format, so it narrows the bounds
    // rather than sitting beside them as a comment.
    let lower = rxn.measured.unwrap_or(rxn.lower_bound);
    let upper = rxn.measured.unwrap_or(rxn.upper_bound);
    json!({
        "id": rxn.id,
        "name": rxn.id,
        "metabolites": number_object(&rxn.metabolites),
        "lower_bound": json_number(lower),
        "upper_bound": json_number(upper),
        "subsystem": rxn.derived.clone().unwrap_or_default(),
    })
}

/// One organism on its own, with the shared pool turned into exchanges.
///
/// The joint model keeps the shared metabolites inside the matrix, where they
/// are neither organism's. Split out, each organism keeps its own copy in its
/// extracellular compartment and gains one exchange reaction per shared
/// metabolite, which is what lets the community layer draw a single medium
/// between the cells and compute the cross-feeding from the fluxes.
///
/// `cell` replaces the default cell description (`{name, organism, shape, color}`).
pub fn to_species_cobra_json(model: &StolyarModel, species_id: &str, cell: Option<&Value>) -> Result<Value> {
    let Some(sp) = SPECIES.iter().find(|s| s.id == species_id) else {
        return Err(fail(format!("Unknown species {species_id}")));
    };
    let ReactionsBySpecies { by_species, .. } = reactions_by_species(model);
    let ids = by_species.get(species_id).map(Vec::as_slice).unwrap_or(&[]);

    let mut reactions: Vec<Value> = ids.iter().filter_map(|rid| model.reactions.get(rid)).map(to_cobra_reaction).collect();
    let mut used: IndexSet<String> = IndexSet::new();
    for rid in ids {
        if let Some(rxn) = model.reactions.get(rid) {
            used.extend(rxn.metabolites.keys().cloned());
        }
    }
    let used = sorted(used);

    // One exchange per shared or external metabolite this organism touches.
    for met in &used {
        if !model.shared.contains(met) && !model.metabolites.get(met).is_some_and(|m| m.external) {
            continue;
        }
        reactions.push(json!({
            "id": format!("EX_{met}_{species_id}"),
            "name": format!("{} exchange", metabolite_name(model, met)),
            "metabolites": {met.clone(): -1},
            "lower_bound": -1000,
            "upper_bound": 1000,
            "subsystem": "Exchange",
        }));
    }

    let mut objective = Map::new();
    if model.reactions.contains_key(sp.biomass_reaction) {
        objective.insert(sp.biomass_reaction.into(), json!(1));
    }

    Ok(json!({
        "id": species_id,
        "name": sp.name,
        "cell": cell.cloned().unwrap_or_else(|| json!({"name": sp.id, "organism": sp.name, "shape": "rounded", "color": "#334155"})),
        "compartments": compartment_names(),
        "metabolites": used.iter().map(|id| json!({
            "id": id, "name": metabolite_name(model, id), "compartment": compartment_of(model, id), "formula": "", "charge": 0,
        })).collect::<Vec<_>>(),
        "reactions": reactions,
        "genes": [],
        "objective": Value::Object(objective),
    }))
}

/// The counts [`validate_stolyar`] reports.
#[derive(Debug, Clone, PartialEq)]
pub struct StolyarCounts {
    pub reactions: usize,
    pub metabolites: usize,
    pub by_species: IndexMap<String, usize>,
    pub community: usize,
    pub shared: Vec<String>,
    /// `id=rate` for every measured reaction.
    pub measured: Vec<String>,
}

/// Counts and invariants, checked against [`EXPECTED`].
#[derive(Debug, Clone, PartialEq)]
pub struct StolyarValidation {
    pub ok: bool,
    pub problems: Vec<String>,
    pub counts: StolyarCounts,
}

impl StolyarValidation {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok,
            "problems": self.problems,
            "counts": {
                "reactions": self.counts.reactions,
                "metabolites": self.counts.metabolites,
                "bySpecies": self.counts.by_species,
                "community": self.counts.community,
                "shared": self.counts.shared,
                "measured": self.counts.measured,
            },
        })
    }
}

/// Counts and invariants, checked against [`EXPECTED`].
pub fn validate_stolyar(model: &StolyarModel) -> StolyarValidation {
    let ReactionsBySpecies { by_species, community } = reactions_by_species(model);
    let measured: Vec<&StolyarReaction> = model.reactions.values().filter(|r| r.measured.is_some()).collect();
    let objective = model.reactions.values().filter(|r| r.objective.is_some_and(|o| o != 0.0 && !o.is_nan())).count();

    let mut problems = Vec::new();
    let mut expect = |label: &str, got: usize, want: usize| {
        if got != want {
            problems.push(format!("{label}: {got}, expected {want}"));
        }
    };
    // The macromolecule synthesis reactions are derived here, so they are not
    // part of the file's own count.
    let macros = model.macromolecules.len();
    expect("reactions in the file", model.reactions.len().wrapping_sub(macros), EXPECTED.reactions);
    expect("metabolites in the file", model.metabolites.len().wrapping_sub(macros), EXPECTED.metabolites);
    expect("macromolecules", macros, EXPECTED.macromolecules);
    expect("external metabolites", external_metabolites(model).len(), EXPECTED.external_metabolites);
    expect("shared metabolites", model.shared.len(), EXPECTED.shared_metabolites);
    expect("measured reactions", measured.len(), EXPECTED.measured_reactions);
    expect("objective reactions", objective, EXPECTED.objective_reactions);
    expect("species", SPECIES.len(), EXPECTED.species);

    let fractions = model.macromolecules.values().fold(0.0, |a, m| a + m.fraction);
    if (fractions - 1.0).abs() > 1e-6 {
        problems.push(format!("macromolecule fractions sum to {}, expected 1", js::number_to_string(fractions)));
    }

    for sp in &SPECIES {
        if by_species.get(sp.id).is_none_or(Vec::is_empty) {
            problems.push(format!("{} has no reactions", sp.id));
        }
        if !model.reactions.contains_key(sp.biomass_reaction) {
            problems.push(format!("{} has no biomass reaction {}", sp.id, sp.biomass_reaction));
        }
    }

    let mut shared: Vec<String> = model.shared.iter().cloned().collect();
    shared.sort_by(|a, b| js::code_unit_compare(a, b));
    StolyarValidation {
        ok: problems.is_empty(),
        problems,
        counts: StolyarCounts {
            reactions: model.reactions.len(),
            metabolites: model.metabolites.len(),
            by_species: by_species.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
            community: community.len(),
            shared,
            measured: measured.iter().map(|r| format!("{}={}", r.id, number_text(r.measured))).collect(),
        },
    }
}
