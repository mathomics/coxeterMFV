//! Port of `src/communities/zuniga.js`.
//!
//! Exact source-matrix FBA for the published ΔpheA rescue experiment
//! (Zuñiga et al. 2020, Nat Commun 11:3803). Models are downloaded privately,
//! never redistributed with Coxeter.
//!
//! The authors' MAT files are fetched from the pinned commit the manifest
//! names and checked against its SHA-256 before anything reads them. The JS
//! keeps them in the browser's private IndexedDB. Here a [`SourceCache`] does
//! (a [`DirectoryCache`] in the user's data folder, for the app). Fetching goes
//! through a [`Fetcher`], so tests and offline callers give their own. The
//! HTTPS one ([`HttpsFetcher`]) is behind the `download` feature.
//!
//! Scenario communities are not built by the community merger: the authors'
//! own joint matrix is solved as it is, and [`ZunigaCommunity`] only reads
//! ownership and cross-feeding off its identifiers (`Ec@…`, `Se@…`, `M@…`).

use std::path::PathBuf;
use std::sync::Arc;

use coxeter_core::community::{CrossFeeding, CrossFeedingLink, FluxShare, MetaboliteExchange, MetaboliteOwner, ReactionOwner, ReactionRole};
use coxeter_core::js;
use coxeter_core::model::strip_compartment;
use coxeter_core::Model;
use coxeter_io::mat5::{MatValue, MatVariables, read_mat5};
use coxeter_io::reproducibility::sha256_hex;
use coxeter_solver::fba::{FluxProblem, FluxResult};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};

use crate::community_view::{CommunityView, TransferRef};
use crate::error::{Error, ErrorKind, Result};
use crate::jsutil::{is_js_ws, js_trim};

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Zuniga, message)
}

pub const ZUNIGA_ID: &str = "zuniga-2020-phea-rescue";
/// The deleted gene: pheA.
pub const ZUNIGA_GENE: &str = "b2599";
const BIOMASS_EC: &str = "Ec@BIOMASS_Ec_iJO1366_core_53p95M";
const BIOMASS_SE: &str = "Se@BOF";

/// One of the three published conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZunigaScenario {
    pub id: &'static str,
    pub name: &'static str,
    /// pheA (b2599) deleted.
    pub mutant: bool,
    /// The joint E. coli + S. elongatus matrix rather than E. coli alone.
    pub coculture: bool,
}

impl ZunigaScenario {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "mutant": self.mutant, "coculture": self.coculture})
    }

    fn find(id: &str) -> Option<&'static ZunigaScenario> {
        ZUNIGA_SCENARIOS.iter().find(|s| s.id == id)
    }
}

pub const ZUNIGA_SCENARIOS: [ZunigaScenario; 3] = [
    ZunigaScenario { id: "wt-monoculture", name: "WT monoculture", mutant: false, coculture: false },
    ZunigaScenario { id: "mutant-monoculture", name: "Mutant monoculture", mutant: true, coculture: false },
    ZunigaScenario { id: "mutant-coculture", name: "Mutant coculture", mutant: true, coculture: true },
];

// Gene rules

/// `String(rule).match(/\(|\)|[^\s()]+/g)`
fn gene_rule_tokens(rule: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in rule.char_indices() {
        if c == '(' || c == ')' || is_js_ws(c) {
            if let Some(s) = start.take() {
                tokens.push(&rule[s..i]);
            }
            if c == '(' || c == ')' {
                tokens.push(&rule[i..i + 1]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        tokens.push(&rule[s..]);
    }
    tokens
}

struct RuleParser<'a, 'd> {
    tokens: Vec<&'a str>,
    index: usize,
    deleted: &'d dyn Fn(&str) -> bool,
}

impl<'a> RuleParser<'a, '_> {
    fn next(&mut self) -> Option<&'a str> {
        let t = self.tokens.get(self.index).copied();
        self.index += 1;
        t
    }

    fn atom(&mut self) -> Result<bool> {
        let Some(token) = self.next() else { return Err(fail("Incomplete gene rule.")) };
        if token == "(" {
            let value = self.or()?;
            if self.next() != Some(")") {
                return Err(fail("Unbalanced gene rule."));
            }
            return Ok(value);
        }
        if token == ")" || token.eq_ignore_ascii_case("and") || token.eq_ignore_ascii_case("or") {
            return Err(fail("Invalid gene rule."));
        }
        Ok(!(self.deleted)(token))
    }

    fn peek_is(&self, word: &str) -> bool {
        self.tokens.get(self.index).is_some_and(|t| t.eq_ignore_ascii_case(word))
    }

    fn and(&mut self) -> Result<bool> {
        let mut value = self.atom()?;
        while self.peek_is("and") {
            self.index += 1;
            let next = self.atom()?;
            value = value && next;
        }
        Ok(value)
    }

    fn or(&mut self) -> Result<bool> {
        let mut value = self.and()?;
        while self.peek_is("or") {
            self.index += 1;
            let next = self.and()?;
            value = value || next;
        }
        Ok(value)
    }
}

/// Whether a reaction keeps an enzyme once the `deleted` genes are gone, so
/// alternate isozymes stay functional. The rule is parsed, never evaluated as
/// code. An empty rule is always `true`, and a malformed rule is an error.
pub fn evaluate_zuniga_gene_rule(rule: &str, deleted: &[&str]) -> Result<bool> {
    evaluate_rule_with(rule, &|gene| deleted.contains(&gene))
}

fn evaluate_rule_with(rule: &str, deleted: &dyn Fn(&str) -> bool) -> Result<bool> {
    if js_trim(rule).is_empty() {
        return Ok(true);
    }
    let mut parser = RuleParser { tokens: gene_rule_tokens(rule), index: 0, deleted };
    let result = parser.or()?;
    if parser.index != parser.tokens.len() {
        return Err(fail("Unexpected token in gene rule."));
    }
    Ok(result)
}

// Reading the MAT values

/// One element of `array(value)`: what `Array.from` yields for a cell, a
/// character matrix, a structure array or a numeric array, or the value
/// itself.
#[derive(Debug, Clone, Copy)]
enum Item<'a> {
    Value(&'a MatValue),
    Text(&'a str),
    Number(f64),
    Struct,
}

/// `array(value)`
fn array(value: Option<&MatValue>) -> Vec<Item<'_>> {
    match value {
        None => Vec::new(),
        Some(MatValue::Cell(items)) => items.iter().map(Item::Value).collect(),
        Some(MatValue::TextRows(rows)) => rows.iter().map(|r| Item::Text(r)).collect(),
        Some(MatValue::StructArray(items)) => items.iter().map(|_| Item::Struct).collect(),
        Some(MatValue::Numeric { data, .. }) => data.values.iter().map(|v| Item::Number(*v)).collect(),
        Some(MatValue::Text(s)) => vec![Item::Text(s)],
        Some(other) => vec![Item::Value(other)],
    }
}

/// `text(value)`: a string, or the texts of its elements joined with `; `.
///
/// The JS recurses without end on a number or a structure (it never meets one
/// in the published files). Here a number prints as JS prints it and a
/// structure as `[object Object]`.
fn text_of(item: Item<'_>) -> String {
    match item {
        Item::Text(s) => s.to_string(),
        Item::Number(n) => js::number_to_string(n),
        Item::Struct => "[object Object]".into(),
        Item::Value(MatValue::Text(s)) => s.clone(),
        Item::Value(MatValue::Struct(_)) | Item::Value(MatValue::Sparse(_)) => "[object Object]".into(),
        Item::Value(v) => text(Some(v)),
    }
}

fn text(value: Option<&MatValue>) -> String {
    match value {
        Some(MatValue::Text(s)) => s.clone(),
        other => array(other).into_iter().map(text_of).collect::<Vec<_>>().join("; "),
    }
}

/// `text(array(value)[i] ?? fallback)`
fn text_at(items: &[Item<'_>], i: usize, fallback: &str) -> String {
    items.get(i).map_or_else(|| fallback.to_string(), |item| text_of(*item))
}

fn number_at(items: &[Item<'_>], i: usize) -> Option<f64> {
    match items.get(i) {
        Some(Item::Number(n)) => Some(*n),
        _ => None,
    }
}

fn field<'a>(raw: &'a MatValue, name: &str) -> Option<&'a MatValue> {
    raw.field(name)
}

/// Where a source metabolite lives and whose it is.
struct MetaboliteInfo {
    species_id: Option<&'static str>,
    original_id: String,
    compartment: String,
    base: String,
}

/// `/_([a-z][a-z0-9]?)$/.exec(local)?.[1]`
fn compartment_suffix(local: &str) -> Option<&str> {
    let b = local.as_bytes();
    let n = b.len();
    if n >= 3 && b[n - 3] == b'_' && b[n - 2].is_ascii_lowercase() && (b[n - 1].is_ascii_lowercase() || b[n - 1].is_ascii_digit()) {
        return Some(&local[n - 2..]);
    }
    if n >= 2 && b[n - 2] == b'_' && b[n - 1].is_ascii_lowercase() {
        return Some(&local[n - 1..]);
    }
    None
}

fn original_metabolite_info(mid: &str) -> MetaboliteInfo {
    if let Some(stem) = mid.strip_suffix("[smp]") {
        return MetaboliteInfo {
            species_id: None,
            original_id: mid.to_string(),
            compartment: "s".into(),
            base: stem.strip_suffix('_').unwrap_or(stem).to_string(),
        };
    }
    let suffix = member_suffix(mid);
    let local = if suffix.is_some() { &mid[..mid.len() - 2] } else { mid };
    let compartment = compartment_suffix(local).unwrap_or("c").to_string();
    MetaboliteInfo { species_id: suffix, original_id: local.to_string(), base: strip_compartment(local, &compartment), compartment }
}

/// `/_(A|D)$/`: `_A` is S. elongatus, `_D` E. coli in the joint matrix.
fn member_suffix(id: &str) -> Option<&'static str> {
    if id.ends_with("_A") {
        Some("Se")
    } else if id.ends_with("_D") {
        Some("Ec")
    } else {
        None
    }
}

/// The rule a reaction's original grRules gives, or, when that text is not a
/// valid rule, the equivalent indexed expression of `rules`.
fn source_gene_rule(raw: &MatValue, index: usize, genes: &[String]) -> Result<(String, Map<String, Value>)> {
    let original = text_at(&array(field(raw, "grRules")), index, "");
    if evaluate_rule_with(&original, &|_| false).is_ok() {
        return Ok((original, Map::new()));
    }
    let encoded = text_at(&array(field(raw, "rules")), index, "");
    if encoded.is_empty() {
        return Err(fail(format!("No valid original GPR for reaction {index}.")));
    }
    let mut translated = String::new();
    let mut rest = encoded.as_str();
    // `.replace(/x\((\d+)\)/g, …)`
    while let Some(at) = rest.find("x(") {
        let digits: &str = {
            let after = &rest[at + 2..];
            let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
            &after[..end]
        };
        let closes = rest[at + 2 + digits.len()..].starts_with(')');
        if digits.is_empty() || !closes {
            translated.push_str(&rest[..at + 2]);
            rest = &rest[at + 2..];
            continue;
        }
        let n = js::string_to_number(digits);
        let gene = (n >= 1.0).then(|| genes.get(n as usize - 1)).flatten().filter(|g| !g.is_empty());
        let Some(gene) = gene else { return Err(fail("Original GPR references an unknown gene.")) };
        translated.push_str(&rest[..at]);
        translated.push_str(gene);
        rest = &rest[at + 2 + digits.len() + 1..];
    }
    translated.push_str(rest);
    let translated = translated.replace('&', " and ").replace('|', " or ");
    evaluate_rule_with(&translated, &|_| false)?;
    let mut notes = Map::new();
    notes.insert("original_gr_rule".into(), json!(original));
    notes.insert("original_rules".into(), json!(encoded));
    notes.insert("gpr_source".into(), json!("rules: original indexed Boolean expression"));
    Ok((translated, notes))
}

/// Convert every coefficient, bound and objective, in original MATLAB order:
/// one model structure of the authors' MAT files (`SingleM{k}`,
/// `modelCo2{k}`) into a COBRA JSON document.
pub fn convert_zuniga_mat_model(raw: &MatValue, id: &str) -> Result<Value> {
    let rxns: Vec<String> = array(field(raw, "rxns")).into_iter().map(text_of).collect();
    let mets: Vec<String> = array(field(raw, "mets")).into_iter().map(text_of).collect();
    let genes: Vec<String> = array(field(raw, "genes")).into_iter().map(text_of).collect();
    let s = match field(raw, "S") {
        Some(MatValue::Sparse(s)) if s.shape.first() == Some(&mets.len()) && s.shape.get(1) == Some(&rxns.len()) => s,
        _ => return Err(fail("The original sparse matrix does not match its identifiers.")),
    };
    let (lb, ub, objective, rhs) =
        (array(field(raw, "lb")), array(field(raw, "ub")), array(field(raw, "c")), array(field(raw, "b")));
    if [&lb, &ub, &objective].iter().any(|values| values.len() != rxns.len()) {
        return Err(fail("Incomplete source reaction vectors."));
    }
    if rhs.len() != mets.len() || rhs.iter().any(|v| !matches!(v, Item::Number(x) if *x == 0.0)) {
        return Err(fail("This case requires the original zero equality RHS."));
    }
    if let Some(csense) = field(raw, "csense").filter(|c| !matches!(c, MatValue::Text(t) if t.is_empty())) {
        let sense: String = text(Some(csense)).chars().filter(|c| !is_js_ws(*c)).collect();
        if sense.encode_utf16().any(|u| u != u16::from(b'E')) {
            return Err(fail("Unsupported source constraint sense."));
        }
    }
    let mut compartments = Map::new();
    let met_names = array(field(raw, "metNames"));
    let formulas = array(field(raw, "metFormulas"));
    let charges = array(field(raw, "metCharges").or_else(|| field(raw, "metCharge")));
    let metabolites: Vec<Value> = mets
        .iter()
        .enumerate()
        .map(|(i, mid)| {
            let info = original_metabolite_info(mid);
            compartments.insert(info.compartment.clone(), json!(info.compartment));
            json!({
                "id": mid,
                "name": text_at(&met_names, i, mid),
                "compartment": info.compartment,
                "base": info.base,
                "formula": text_at(&formulas, i, ""),
                "charge": number_at(&charges, i).filter(|c| c.is_finite()),
            })
        })
        .collect();
    let names = array(field(raw, "rxnNames"));
    let subsystems = array(field(raw, "subSystems"));
    let (jc, ir, values) = (&s.jc.values, &s.ir.values, &s.values.values);
    let mut reactions = Vec::with_capacity(rxns.len());
    for (j, rid) in rxns.iter().enumerate() {
        let mut stoich = Map::new();
        let (start, end) = (jc.get(j).copied().unwrap_or(0.0) as usize, jc.get(j + 1).copied().unwrap_or(0.0) as usize);
        for k in start..end {
            let value = values.get(k).copied().unwrap_or(f64::NAN);
            let met = ir.get(k).and_then(|r| mets.get(*r as usize));
            let Some(met) = met.filter(|m| !m.is_empty() && value.is_finite()) else {
                return Err(fail("Invalid source stoichiometry."));
            };
            if value != 0.0 {
                stoich.insert(met.clone(), json!(value));
            }
        }
        js::js_object_key_order(&mut stoich);
        let (l, u, c) = (number_at(&lb, j), number_at(&ub, j), number_at(&objective, j));
        let (Some(l), Some(u), Some(c)) = (l, u, c) else { return Err(fail("Invalid original reaction bounds or objective.")) };
        if !(l.is_finite() && u.is_finite() && c.is_finite()) || l > u {
            return Err(fail("Invalid original reaction bounds or objective."));
        }
        let (rule, gpr_notes) = source_gene_rule(raw, j, &genes)?;
        let mut notes = Map::new();
        notes.insert("source_reaction_id".into(), json!(rid));
        notes.extend(gpr_notes);
        reactions.push(json!({
            "id": rid,
            "name": text_at(&names, j, rid),
            "metabolites": stoich,
            "lower_bound": l,
            "upper_bound": u,
            "objective_coefficient": c,
            "subsystem": text_at(&subsystems, j, ""),
            "gene_reaction_rule": rule,
            "notes": notes,
        }));
    }
    let description = field(raw, "description");
    Ok(json!({
        "id": id,
        "name": if description.is_some() { text(description) } else { id.to_string() },
        "metabolites": metabolites,
        "reactions": reactions,
        "genes": genes.iter().map(|g| json!({"id": g, "name": g})).collect::<Vec<_>>(),
        "compartments": compartments,
        "notes": {
            "source_description": text(description),
            "source_reaction_count": rxns.len(),
            "source_metabolite_count": mets.len(),
            "source_matrix_nonzeros": values.len(),
            "namespace_conversion": "Identifiers only; no abundance scaling, added reactions, medium changes or objective rewiring.",
        },
    }))
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `{...(x.notes ?? {}), key: value}`
fn with_note(notes: Option<&Value>, key: &str, value: &str) -> Value {
    let mut out = match notes {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    out.insert(key.into(), json!(value));
    Value::Object(out)
}

/// Lossless identifier bijection. The scientific LP coefficients are unchanged.
///
/// `species_id` names the one member of a monoculture model (`Ec`). `None`
/// reads membership off the `_A`/`_D` suffixes of the joint matrix. Shared
/// `[smp]` metabolites become `M@<base>`, and the joint matrix's exchanges of
/// them `EX_<base>_s`.
pub fn canonical_zuniga_model(source: &Value, species_id: Option<&str>) -> Result<Value> {
    let mut doc = source.clone();
    let mut met_ids: IndexMap<String, String> = IndexMap::new();
    let mut used_metabolites: IndexSet<String> = IndexSet::new();
    if let Some(mets) = doc.get_mut("metabolites").and_then(Value::as_array_mut) {
        for met in mets.iter_mut() {
            let id = str_field(met, "id").to_string();
            let info = original_metabolite_info(&id);
            let species = species_id.or(info.species_id);
            let target = if id.ends_with("[smp]") {
                format!("M@{}", info.base)
            } else if let Some(sp) = species {
                format!("{sp}@{}", info.original_id)
            } else {
                id.clone()
            };
            if !used_metabolites.insert(target.clone()) {
                return Err(fail("Metabolite namespace collision."));
            }
            met_ids.insert(id.clone(), target.clone());
            let notes = with_note(met.get("notes"), "source_metabolite_id", &id);
            if let Some(m) = met.as_object_mut() {
                m.insert("notes".into(), notes);
                m.insert("id".into(), json!(target));
                m.insert("base".into(), json!(info.base));
                m.insert("compartment".into(), json!(info.compartment));
            }
        }
    }
    let mut used: IndexSet<String> = IndexSet::new();
    if let Some(reactions) = doc.get_mut("reactions").and_then(Value::as_array_mut) {
        for reaction in reactions.iter_mut() {
            let original = str_field(reaction, "id").to_string();
            let suffix = member_suffix(&original);
            let species = species_id.or(suffix);
            let local = if suffix.is_some() && species_id.is_none() { &original[..original.len() - 2] } else { original.as_str() };
            let entries: Vec<(String, Value)> = match reaction.get("metabolites") {
                Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                _ => Vec::new(),
            };
            let only_shared = entries.len() == 1 && entries[0].0.ends_with("[smp]");
            let new_id = if let Some(sp) = species {
                format!("{sp}@{local}")
            } else if only_shared && original.starts_with("EX_") {
                format!("EX_{}_s", original_metabolite_info(&entries[0].0).base)
            } else {
                original.clone()
            };
            if !used.insert(new_id.clone()) {
                return Err(fail("Reaction namespace collision."));
            }
            let mut metabolites: Map<String, Value> = entries
                .into_iter()
                .map(|(mid, value)| (met_ids.get(&mid).cloned().unwrap_or_else(|| "undefined".into()), value))
                .collect();
            js::js_object_key_order(&mut metabolites);
            let notes = with_note(reaction.get("notes"), "source_reaction_id", &original);
            if let Some(r) = reaction.as_object_mut() {
                r.insert("id".into(), json!(new_id));
                r.insert("metabolites".into(), Value::Object(metabolites));
                r.insert("notes".into(), notes);
            }
        }
    }
    let source_id = source.get("id").map_or_else(|| "undefined".to_string(), js::to_string);
    if let Some(d) = doc.as_object_mut() {
        d.insert("id".into(), json!(format!("{source_id}-canonical")));
    }
    Ok(doc)
}

// The study

/// The two source documents a scenario is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct ZunigaSourceModels {
    /// `M-models.mat:SingleM{4}`, E. coli iJO1366, converted.
    pub mono: Value,
    /// `CM-modelsKO.mat:modelCo2{2}`, the joint matrix, converted.
    pub joint: Value,
}

/// The member models, drawn as cells.
#[derive(Debug, Clone)]
pub struct ZunigaModels {
    pub ec: Arc<Model>,
    pub se: Arc<Model>,
}

impl ZunigaModels {
    /// The model of member `Ec` or `Se`.
    pub fn get(&self, id: &str) -> Option<&Arc<Model>> {
        match id {
            "Ec" => Some(&self.ec),
            "Se" => Some(&self.se),
            _ => None,
        }
    }
}

/// The loaded study (`loadZunigaStudy`'s result).
#[derive(Debug, Clone)]
pub struct ZunigaStudy {
    pub manifest: Value,
    /// File name → SHA-256 of the bytes read.
    pub source_hashes: IndexMap<String, String>,
    pub source_models: ZunigaSourceModels,
    pub models: ZunigaModels,
}

/// The answer to a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// `response.ok`
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Something that downloads a URL (`fetch`). An error is a failure to get any
/// answer at all, while an HTTP error status is a response.
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<FetchResponse>;
}

impl<F: Fn(&str) -> Result<FetchResponse>> Fetcher for F {
    fn fetch(&self, url: &str) -> Result<FetchResponse> {
        self(url)
    }
}

/// Where verified source files are kept between runs (the JS uses a private
/// IndexedDB store). Failures to store are ignored, as in the JS.
pub trait SourceCache {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn put(&self, key: &str, bytes: &[u8]);
}

/// A cache in a folder: one file per key, named by the key's SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryCache {
    pub dir: PathBuf,
}

impl DirectoryCache {
    /// A cache in `dir`. The folder is created on the first store.
    pub fn new(dir: impl Into<PathBuf>) -> DirectoryCache {
        DirectoryCache { dir: dir.into() }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.bin", sha256_hex(key.as_bytes())))
    }
}

impl SourceCache for DirectoryCache {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(key)).ok()
    }

    fn put(&self, key: &str, bytes: &[u8]) {
        let _ = std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(self.path(key), bytes));
    }
}

/// HTTPS downloads through ureq.
#[cfg(feature = "download")]
#[derive(Debug, Clone)]
pub struct HttpsFetcher {
    agent: ureq::Agent,
}

#[cfg(feature = "download")]
impl Default for HttpsFetcher {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build();
        HttpsFetcher { agent: ureq::Agent::new_with_config(config) }
    }
}

#[cfg(feature = "download")]
impl Fetcher for HttpsFetcher {
    fn fetch(&self, url: &str) -> Result<FetchResponse> {
        let failed = |e: ureq::Error| Error::new(ErrorKind::Download, format!("{url}: {e}"));
        let mut response = self.agent.get(url).call().map_err(failed)?;
        let status = response.status().as_u16();
        // The MAT reader refuses more than 128 MB, so read no more than that.
        let body = response.body_mut().with_config().limit(128 * 1024 * 1024 + 1).read_to_vec().map_err(failed)?;
        Ok(FetchResponse { status, body })
    }
}

/// Options of [`load_zuniga_study`].
pub struct ZunigaLoadOptions<'a> {
    pub fetcher: &'a dyn Fetcher,
    /// Where verified downloads are kept. `None` downloads every time.
    pub cache: Option<&'a dyn SourceCache>,
    /// Told what is happening (`Downloading the original M-models.mat…`).
    pub on_progress: Option<&'a mut dyn FnMut(&str)>,
}

/// Download (or take from the cache) every source file the manifest pins,
/// verify each against its SHA-256 before reading it, and convert the three
/// models the study uses.
pub fn load_zuniga_study(manifest: &Value, options: ZunigaLoadOptions<'_>) -> Result<ZunigaStudy> {
    let ZunigaLoadOptions { fetcher, cache, mut on_progress } = options;
    let mut progress = |message: String| {
        if let Some(f) = on_progress.as_mut() {
            f(&message);
        }
    };
    let mut sources: IndexMap<String, MatVariables> = IndexMap::new();
    let mut source_hashes: IndexMap<String, String> = IndexMap::new();
    for source in manifest.get("sources").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        let file = source.get("file").map_or_else(|| "undefined".into(), js::to_string);
        let url = source.get("url").map_or_else(|| "undefined".into(), js::to_string);
        let pinned = source.get("sha256").map_or_else(|| "undefined".into(), js::to_string);
        progress(format!("Downloading the original {file}…"));
        let key = format!("{url}#{pinned}");
        let bytes = match cache.and_then(|c| c.get(&key)) {
            Some(bytes) => bytes,
            None => {
                let response = fetcher.fetch(&url)?;
                if !response.ok() {
                    return Err(Error::new(ErrorKind::Download, format!("{file}: HTTP {}", response.status)));
                }
                response.body
            }
        };
        let hash = sha256_hex(&bytes);
        if hash != pinned {
            return Err(Error::new(ErrorKind::Download, format!("Source integrity check failed for {file}.")));
        }
        if let Some(c) = cache {
            c.put(&key, &bytes);
        }
        source_hashes.insert(file.clone(), hash);
        progress(format!("Reading the full original {file}…"));
        sources.insert(file, read_mat5(&bytes).map_err(|e| fail(e.message))?);
    }
    zuniga_study_from_sources(manifest, &sources, source_hashes)
}

/// The study from MAT files already read and verified (file name →
/// variables): the conversion half of [`load_zuniga_study`].
pub fn zuniga_study_from_sources(
    manifest: &Value,
    sources: &IndexMap<String, MatVariables>,
    source_hashes: IndexMap<String, String>,
) -> Result<ZunigaStudy> {
    let cell = |file: &str, var: &str, i: usize| match sources.get(file).and_then(|v| v.get(var)) {
        Some(MatValue::Cell(items)) => items.get(i).filter(|v| !matches!(v, MatValue::Text(t) if t.is_empty())),
        _ => None,
    };
    let (Some(single4), Some(single2), Some(joint2)) =
        (cell("M-models.mat", "SingleM", 3), cell("M-models.mat", "SingleM", 1), cell("CM-modelsKO.mat", "modelCo2", 1))
    else {
        return Err(fail("The original model cell arrays are incomplete."));
    };
    let mut mono = convert_zuniga_mat_model(single4, "zuniga-iJO1366")?;
    let mut se = convert_zuniga_mat_model(single2, "zuniga-iJB785-cscB")?;
    let co = convert_zuniga_mat_model(joint2, "zuniga-original-knockout-analysis-community")?;
    for (doc, rxns, mets) in [(&mono, 2583, 1805), (&se, 899, 793), (&co, 3162, 2547)] {
        let count = |key: &str| doc.get(key).and_then(Value::as_array).map_or(0, Vec::len);
        if count("reactions") != rxns || count("metabolites") != mets {
            return Err(fail("The pinned source model dimensions do not match."));
        }
    }
    mono["cell"] = json!({"name": "E. coli K-12", "organism": "Escherichia coli K-12 MG1655", "color": "#b45309"});
    se["cell"] = json!({"name": "S. elongatus cscB+", "organism": "Synechococcus elongatus PCC7942 cscB+", "color": "#047857"});
    let models = ZunigaModels { ec: Arc::new(Model::from_value(&mono)?), se: Arc::new(Model::from_value(&se)?) };
    Ok(ZunigaStudy {
        manifest: manifest.clone(),
        source_hashes,
        source_models: ZunigaSourceModels { mono, joint: co },
        models,
    })
}

// The community

/// A member of the scenario community.
#[derive(Debug, Clone)]
pub struct ZunigaMember {
    pub id: String,
    pub model: Arc<Model>,
    pub abundance: f64,
    /// S. elongatus only in coculture.
    pub enabled: bool,
    pub color: String,
}

/// A reaction joining a cell to the shared `M@` pool.
#[derive(Debug, Clone, PartialEq)]
pub struct ZunigaTransfer {
    pub species_id: String,
    pub reaction_id: String,
    /// The member's own metabolite in it, if it names one.
    pub local_id: Option<String>,
    /// The shared metabolite's coefficient.
    pub coefficient: f64,
}

/// The community a scenario shows (`exactCommunity`): the authors' matrix as
/// it is, with ownership read off the identifiers. No abundance scaling.
#[derive(Debug, Clone)]
pub struct ZunigaCommunity {
    pub name: String,
    pub objective_type: String,
    pub species: Vec<ZunigaMember>,
    pub merged: Arc<Model>,
    pub reaction_owner: IndexMap<String, ReactionOwner>,
    pub metabolite_owner: IndexMap<String, MetaboliteOwner>,
    pub transfers_by_base: IndexMap<String, Vec<ZunigaTransfer>>,
    /// Member → its biomass reaction (S. elongatus only in coculture).
    pub biomass_ids: IndexMap<String, String>,
    /// 1 for an enabled member, 0 otherwise.
    pub abundances: IndexMap<String, f64>,
    pub species_compartments: IndexMap<String, Vec<String>>,
}

/// `^(Ec|Se)@(.+)$`
fn split_member(id: &str) -> Option<(&str, &str)> {
    let (sp, rest) = id.split_once('@')?;
    let line_end = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    ((sp == "Ec" || sp == "Se") && !rest.is_empty() && !rest.contains(line_end)).then_some((sp, rest))
}

impl ZunigaCommunity {
    fn exact(model: Arc<Model>, study: &ZunigaStudy, scenario: &ZunigaScenario) -> ZunigaCommunity {
        let members: Vec<ZunigaMember> = ["Ec", "Se"]
            .iter()
            .map(|id| ZunigaMember {
                id: id.to_string(),
                model: study.models.get(id).expect("Ec and Se").clone(),
                abundance: 1.0,
                enabled: *id == "Ec" || scenario.coculture,
                color: if *id == "Ec" { "#b45309" } else { "#047857" }.into(),
            })
            .collect();
        let mut metabolite_owner = IndexMap::new();
        for met in model.metabolites.values() {
            let m = split_member(&met.id);
            metabolite_owner.insert(
                met.id.clone(),
                MetaboliteOwner {
                    species_id: m.map(|(sp, _)| sp.to_string()),
                    original_id: m.map_or_else(|| met.base.clone(), |(_, rest)| rest.to_string()),
                    compartment: met.compartment.clone(),
                },
            );
        }
        let mut reaction_owner = IndexMap::new();
        let mut transfers_by_base: IndexMap<String, Vec<ZunigaTransfer>> = IndexMap::new();
        for rxn in model.reactions.values() {
            let m = split_member(&rxn.id);
            let shared: Vec<(&String, f64)> = rxn.metabolites.iter().filter(|(mid, _)| mid.starts_with("M@")).map(|(k, v)| (k, *v)).collect();
            let role = if m.is_some() && !shared.is_empty() { ReactionRole::Transfer } else { ReactionRole::from(rxn.kind) };
            reaction_owner.insert(
                rxn.id.clone(),
                ReactionOwner {
                    species_id: m.map(|(sp, _)| sp.to_string()),
                    original_id: m.map_or_else(|| rxn.id.clone(), |(_, rest)| rest.to_string()),
                    role,
                },
            );
            if let Some((sp, _)) = m {
                for (mid, coefficient) in shared {
                    let local_id = rxn.metabolites.keys().find(|id| id.starts_with(&format!("{sp}@"))).cloned();
                    transfers_by_base.entry(mid[2..].to_string()).or_default().push(ZunigaTransfer {
                        species_id: sp.to_string(),
                        reaction_id: rxn.id.clone(),
                        local_id,
                        coefficient,
                    });
                }
            }
        }
        let mut biomass_ids = IndexMap::new();
        biomass_ids.insert("Ec".to_string(), BIOMASS_EC.to_string());
        if scenario.coculture {
            biomass_ids.insert("Se".to_string(), BIOMASS_SE.to_string());
        }
        ZunigaCommunity {
            name: "Zuñiga et al. (2020): ΔpheA rescue".into(),
            objective_type: "source-biomass-objective".into(),
            abundances: members.iter().map(|m| (m.id.clone(), if m.enabled { 1.0 } else { 0.0 })).collect(),
            species_compartments: members.iter().map(|m| (m.id.clone(), m.model.compartment_ids())).collect(),
            species: members,
            merged: model,
            reaction_owner,
            metabolite_owner,
            transfers_by_base,
            biomass_ids,
        }
    }

    /// The enabled members.
    pub fn active_species(&self) -> impl Iterator<Item = &ZunigaMember> {
        self.species.iter().filter(|m| m.enabled)
    }

    /// The abundances as they are: nothing is normalised in this case.
    pub fn normalised_abundances(&self) -> IndexMap<String, f64> {
        self.abundances.clone()
    }

    /// Each member's biomass flux.
    pub fn growth_rates(&self, fluxes: &IndexMap<String, f64>) -> IndexMap<String, f64> {
        self.biomass_ids.iter().map(|(id, rxn)| (id.clone(), fluxes.get(rxn).copied().unwrap_or(0.0))).collect()
    }

    /// Net transfer between cells for every shared metabolite: each member's
    /// transfers are summed first, so simultaneous uptake and release through
    /// two transporters is not reported as feeding.
    pub fn cross_feeding(&self, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding {
        let mut links = Vec::new();
        let mut per_metabolite = Vec::new();
        for (base, transfers) in &self.transfers_by_base {
            let mut producers = Vec::new();
            let mut consumers = Vec::new();
            let mut by_species: IndexMap<&str, f64> = IndexMap::new();
            for t in transfers {
                let net = fluxes.get(&t.reaction_id).copied().unwrap_or(0.0) * t.coefficient;
                *by_species.entry(t.species_id.as_str()).or_insert(0.0) += net;
            }
            for (species_id, net) in by_species {
                if net > threshold {
                    producers.push(FluxShare { species_id: Some(species_id.to_string()), flux: net });
                }
                if net < -threshold {
                    consumers.push(FluxShare { species_id: Some(species_id.to_string()), flux: -net });
                }
            }
            let net = match self.merged.reactions.get(&format!("EX_{base}_s")) {
                Some(boundary) => {
                    fluxes.get(&boundary.id).copied().unwrap_or(0.0) * boundary.metabolites.get(&format!("M@{base}")).copied().unwrap_or(0.0)
                }
                None => 0.0,
            };
            if net > threshold {
                producers.push(FluxShare { species_id: None, flux: net });
            }
            if net < -threshold {
                consumers.push(FluxShare { species_id: None, flux: -net });
            }
            let total_out = producers.iter().fold(0.0, |s, e| s + e.flux);
            let total_in = consumers.iter().fold(0.0, |s, e| s + e.flux);
            let total = js::min(total_out, total_in);
            let truthy = |x: f64| x != 0.0 && !x.is_nan();
            if truthy(total_out) || truthy(total_in) {
                let shown = if truthy(total) {
                    total
                } else if truthy(total_out) {
                    total_out
                } else {
                    total_in
                };
                per_metabolite.push(MetaboliteExchange {
                    base: base.clone(),
                    producers: producers.clone(),
                    consumers: consumers.clone(),
                    total: shown,
                });
            }
            if total_out > 0.0 && total_in > 0.0 {
                for producer in &producers {
                    for consumer in &consumers {
                        if producer.species_id == consumer.species_id {
                            continue;
                        }
                        let flux = total * producer.flux / total_out * consumer.flux / total_in;
                        if flux > threshold {
                            links.push(CrossFeedingLink {
                                base: base.clone(),
                                from: producer.species_id.clone(),
                                to: consumer.species_id.clone(),
                                flux,
                            });
                        }
                    }
                }
            }
        }
        links.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));
        CrossFeeding { links, per_metabolite }
    }

    /// `community.toJSON()`.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "objective": self.objective_type,
            "species": self.species.iter().map(|m| json!({"id": m.id, "enabled": m.enabled, "model": m.model.id})).collect::<Vec<_>>(),
        })
    }
}

impl CommunityView for ZunigaCommunity {
    fn merged_reaction_ids(&self) -> Vec<&str> {
        self.merged.reactions.keys().map(String::as_str).collect()
    }

    fn reaction_owner(&self, reaction_id: &str) -> Option<&ReactionOwner> {
        self.reaction_owner.get(reaction_id)
    }

    fn transfers(&self, base: &str) -> Vec<TransferRef<'_>> {
        self.transfers_by_base
            .get(base)
            .map(|list| list.iter().map(|t| TransferRef { species_id: &t.species_id, reaction_id: &t.reaction_id }).collect())
            .unwrap_or_default()
    }

    fn cross_feeding(&self, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding {
        ZunigaCommunity::cross_feeding(self, fluxes, threshold)
    }
}

// Scenarios

/// A scenario ready to solve.
#[derive(Debug, Clone)]
pub struct ZunigaPrepared {
    pub model: Arc<Model>,
    pub community: ZunigaCommunity,
    pub problem: FluxProblem,
    pub scenario: ZunigaScenario,
    /// Reactions the deletion closed (`Ec@PPNDH` in a mutant).
    pub knockouts: Vec<String>,
    pub source_hashes: IndexMap<String, String>,
    /// `CM-modelsKO.mat:modelCo2{2}` or `M-models.mat:SingleM{4}`.
    pub source: &'static str,
}

/// Build one of the three scenarios from the study: the monoculture or joint
/// matrix, its identifiers made canonical, and, for a mutant, the reactions
/// the b2599 deletion stops, which must be exactly `Ec@PPNDH`.
pub fn create_zuniga_scenario(study: &ZunigaStudy, scenario_id: &str) -> Result<ZunigaPrepared> {
    let Some(scenario) = ZunigaScenario::find(scenario_id) else {
        return Err(fail(format!("Unknown Zuñiga scenario: {scenario_id}")));
    };
    let raw = if scenario.coculture { &study.source_models.joint } else { &study.source_models.mono };
    let mut doc = canonical_zuniga_model(raw, if scenario.coculture { None } else { Some("Ec") })?;
    let mut knockouts = Vec::new();
    if scenario.mutant {
        for rxn in doc.get_mut("reactions").and_then(Value::as_array_mut).into_iter().flatten() {
            let rule = match rxn.get("gene_reaction_rule") {
                None | Some(Value::Null) => String::new(),
                Some(v) => js::to_string(v),
            };
            if !evaluate_zuniga_gene_rule(&rule, &[ZUNIGA_GENE])? {
                rxn["lower_bound"] = json!(0);
                rxn["upper_bound"] = json!(0);
                knockouts.push(str_field(rxn, "id").to_string());
            }
        }
        if knockouts.len() != 1 || knockouts[0] != "Ec@PPNDH" {
            return Err(fail("The original pheA deletion does not match the documented PPNDH knockout."));
        }
    }
    restore_zuniga_scenario(study, scenario_id, Model::from_value(&doc)?, Some(knockouts))
}

/// Replay a personal export against its full canonical LP, without a merger.
/// `knockouts` defaults to the documented one for a mutant scenario.
pub fn restore_zuniga_scenario(
    study: &ZunigaStudy,
    scenario_id: &str,
    active_model: Model,
    knockouts: Option<Vec<String>>,
) -> Result<ZunigaPrepared> {
    let Some(scenario) = ZunigaScenario::find(scenario_id) else {
        return Err(fail("Unknown case scenario."));
    };
    let model = Arc::new(active_model);
    let community = ZunigaCommunity::exact(model.clone(), study, scenario);
    Ok(ZunigaPrepared {
        problem: FluxProblem::new(&model),
        model,
        community,
        scenario: *scenario,
        knockouts: knockouts.unwrap_or_else(|| if scenario.mutant { vec!["Ec@PPNDH".into()] } else { Vec::new() }),
        source_hashes: study.source_hashes.clone(),
        source: if scenario.coculture { "CM-modelsKO.mat:modelCo2{2}" } else { "M-models.mat:SingleM{4}" },
    })
}

/// What a solved scenario records about itself (`result.zuniga`).
#[derive(Debug, Clone, PartialEq)]
pub struct ZunigaRun {
    pub id: &'static str,
    pub scenario_id: &'static str,
    /// `b2599` for a mutant.
    pub gene: Option<&'static str>,
    pub knockouts: Vec<String>,
    pub source: &'static str,
    pub source_hashes: IndexMap<String, String>,
    pub method: &'static str,
    /// The fluxes under the authors' own reaction ids.
    pub original_fluxes: IndexMap<String, f64>,
}

impl ZunigaRun {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "scenarioId": self.scenario_id, "gene": self.gene, "knockouts": self.knockouts,
            "source": self.source, "sourceHashes": self.source_hashes, "method": self.method,
            "originalFluxes": Value::Object(self.original_fluxes.iter().map(|(k, v)| (k.clone(), crate::jsutil::json_number(*v))).collect()),
        })
    }
}

/// An FBA result with the record of the scenario it solves.
#[derive(Debug, Clone)]
pub struct ZunigaSolution {
    pub result: FluxResult,
    pub zuniga: ZunigaRun,
}

/// Stationary FBA with the source objective.
pub fn solve_zuniga_scenario(prepared: &ZunigaPrepared) -> ZunigaSolution {
    let result = prepared.problem.optimize();
    let mut original_fluxes = IndexMap::new();
    for (id, value) in &result.fluxes {
        let original = prepared
            .model
            .reactions
            .get(id)
            .and_then(|r| r.notes.get("source_reaction_id"))
            .filter(|v| !v.is_null())
            .map_or_else(|| id.clone(), js::to_string);
        original_fluxes.insert(original, *value);
    }
    js::js_key_order(&mut original_fluxes);
    let zuniga = ZunigaRun {
        id: ZUNIGA_ID,
        scenario_id: prepared.scenario.id,
        gene: prepared.scenario.mutant.then_some(ZUNIGA_GENE),
        knockouts: prepared.knockouts.clone(),
        source: prepared.source,
        source_hashes: prepared.source_hashes.clone(),
        method: "Stationary FBA with the source objective; no equal-growth or abundance scaling",
        original_fluxes,
    };
    ZunigaSolution { result, zuniga }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_on_parentheses_and_javascript_whitespace() {
        assert_eq!(gene_rule_tokens("(a and b)or\u{FEFF}c"), ["(", "a", "and", "b", ")", "or", "c"]);
        assert_eq!(gene_rule_tokens("  "), Vec::<&str>::new());
    }

    #[test]
    fn compartment_suffix_is_the_regex_one() {
        assert_eq!(compartment_suffix("glc__D_e"), Some("e"));
        assert_eq!(compartment_suffix("x_c1"), Some("c1"));
        assert_eq!(compartment_suffix("x_1"), None);
        assert_eq!(compartment_suffix("x"), None);
        assert_eq!(compartment_suffix("_ab"), Some("ab"));
    }
}
