//! Port of `src/core/model-import.js`: import the models a user supplies:
//! COBRA JSON, SBML Level 3/FBC, a COBRA Toolbox MATLAB file (.mat, v5/v7)
//! or a COBRA Toolbox Excel workbook (.xlsx).
//!
//! The interface hands this module `{name, text}` records, or `{name, bytes}`
//! for the binary formats. Everything here is pure and independently
//! testable: parsing, structural validation, Model construction, stable
//! species/model identifiers and medium inference.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

use coxeter_core::js;
use coxeter_core::model::{Model, infer_compartment};

use crate::error::{Error, ErrorKind, Result};
use crate::gene_rules::{evaluate_gene_rule, gene_rule_genes};
use crate::mat5::{MatValue, MatVariables, read_mat5};
use crate::xlsx::{Workbook, read_workbook};
use crate::jscompat::{self, B, DOT, W, js_order, js_trim, object_entries};
use crate::provenance::extract_identifiers_org;
use crate::sbml::{UnmangleOptions, unmangle_id};

fn unmangle(id: &str, prefix: &str) -> String {
    unmangle_id(id, prefix, UnmangleOptions::default())
}

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidModel, message)
}

/// Read a COBRA JSON model. Errors name the file.
pub fn parse_model_json(text: &str, filename: &str) -> Result<Model> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|e| Error::new(ErrorKind::InvalidJson, format!("{filename}: invalid JSON ({e})")))?;
    validate_raw_model(&raw, filename)?;
    let model = Model::from_value(&raw).map_err(|e| invalid(format!("{filename}: {e}")))?;
    if model.reactions.is_empty() {
        return Err(invalid(format!("{filename}: the model contains no reactions.")));
    }
    if model.metabolites.is_empty() {
        return Err(invalid(format!("{filename}: the model contains no metabolites.")));
    }
    Ok(model)
}

/// Read an SBML Level 3 + FBC model. Errors name the file.
pub fn parse_model_sbml(text: &str, filename: &str) -> Result<Model> {
    let raw = sbml_to_cobra(text, filename)?;
    validate_raw_model(&raw, filename)?;
    Model::from_value(&raw).map_err(|e| invalid(format!("{filename}: {e}")))
}

/// Read a model as SBML when its name or content says so, else as JSON.
pub fn parse_model_file(text: &str, filename: &str) -> Result<Model> {
    let trimmed = js_trim(text);
    let lower = filename.to_lowercase();
    if lower.ends_with(".xml") || lower.ends_with(".sbml") || trimmed.starts_with('<') {
        return parse_model_sbml(text, filename);
    }
    parse_model_json(text, filename)
}

/// A file the user picked: its name and its text.
#[derive(Debug, Clone)]
pub struct ImportFile {
    pub name: String,
    pub text: String,
}

/// What the import dialog shows for one model.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSummary {
    pub reactions: usize,
    pub metabolites: usize,
    pub compartments: usize,
    pub biomass: Option<String>,
}

/// One uploaded model, ready to become a species.
#[derive(Debug, Clone)]
pub struct ImportedEntry {
    pub filename: String,
    pub model: Model,
    /// `upload:<stem>`, unique among the uploads.
    pub model_key: String,
    /// The model id made safe, unique among the uploads.
    pub species_id: String,
    pub abundance: f64,
    pub enabled: bool,
    pub summary: ImportSummary,
}

/// The models of one upload and the medium their exchanges imply.
#[derive(Debug, Clone)]
pub struct PreparedImport {
    pub entries: Vec<ImportedEntry>,
    pub medium: IndexMap<String, f64>,
}

static MODEL_EXTENSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?i-u:json|xml|sbml|mat|xlsx|xlsm)$").expect("pattern"));
static SBML_EXTENSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?i-u:xml|sbml)$").expect("pattern"));
static UNSAFE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_.-]+").expect("pattern"));

fn no_files() -> Error {
    invalid("Select at least one model: COBRA JSON, SBML, MATLAB (.mat) or Excel (.xlsx).".into())
}

/// Read every file, give each a distinct species id and model key, and infer
/// the community medium from their exchange reactions.
pub fn prepare_imported_models(files: &[ImportFile]) -> Result<PreparedImport> {
    if files.is_empty() {
        return Err(no_files());
    }
    let parsed = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let filename = if file.name.is_empty() { format!("model-{}.json", index + 1) } else { file.name.clone() };
            let model = parse_model_file(&file.text, &filename)?;
            Ok((filename, model))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(prepare_parsed(parsed))
}

/// The entries of models already read, `(file name, model)`, in order.
fn prepare_parsed(parsed: Vec<(String, Model)>) -> PreparedImport {
    let mut taken_species = HashSet::new();
    let mut taken_keys = HashSet::new();
    let count = parsed.len();
    let mut entries = Vec::with_capacity(count);
    for (index, (filename, model)) in parsed.into_iter().enumerate() {
        let stem = MODEL_EXTENSION.replace(&filename, "").into_owned();
        let fallback = format!("sp{}", index + 1);
        let base_species = safe_id(first_non_empty(&[&model.id, &stem, &fallback]));
        let species_id = unique_id(&base_species, &mut taken_species);
        let base_key = format!("upload:{}", safe_id(first_non_empty(&[&stem, &model.id, &species_id])));
        let model_key = unique_id(&base_key, &mut taken_keys);
        let summary = ImportSummary {
            reactions: model.reactions.len(),
            metabolites: model.metabolites.len(),
            compartments: model.compartment_ids().len(),
            biomass: model.biomass_reaction_id(),
        };
        entries.push(ImportedEntry {
            filename,
            model,
            model_key,
            species_id,
            abundance: 1.0 / count as f64,
            enabled: true,
            summary,
        });
    }
    let medium = infer_community_medium(&entries.iter().map(|e| &e.model).collect::<Vec<_>>());
    PreparedImport { entries, medium }
}

fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    values.iter().copied().find(|v| !v.is_empty()).unwrap_or("")
}

/// Base metabolite → the largest uptake any exchange of any model allows.
pub fn infer_community_medium(models: &[&Model]) -> IndexMap<String, f64> {
    let mut medium: IndexMap<String, f64> = IndexMap::new();
    for model in models {
        for reaction in model.exchange_reactions() {
            // `!(lowerBound < 0)`: only exchanges that allow uptake.
            if reaction.lower_bound >= 0.0 || reaction.lower_bound.is_nan() {
                continue;
            }
            if reaction.metabolites.len() != 1 {
                continue;
            }
            let Some(met) = reaction.metabolites.keys().next().and_then(|id| model.metabolites.get(id)) else {
                continue;
            };
            let previous = medium.get(&met.base).copied().unwrap_or(0.0);
            medium.insert(met.base.clone(), js::max(previous, reaction.lower_bound.abs()));
        }
    }
    js::js_key_order(&mut medium);
    medium
}

fn validate_raw_model(raw: &Value, filename: &str) -> Result<()> {
    let Some(obj) = raw.as_object() else {
        return Err(invalid(format!("{filename}: the root of the file must be a JSON object.")));
    };
    // Both files are "the JSON" the app offers to download, so they get swapped.
    // Saying "missing the metabolites array" about a saved map helps nobody.
    if obj.get("format").and_then(Value::as_str) == Some("EscherMCMCMap") {
        return Err(invalid(format!("{filename}: this is a saved map, not a model. Load it from File ▸ Load map.")));
    }
    let Some(metabolites) = obj.get("metabolites").and_then(Value::as_array) else {
        return Err(invalid(format!("{filename}: missing the \"metabolites\" array.")));
    };
    let Some(reactions) = obj.get("reactions").and_then(Value::as_array) else {
        return Err(invalid(format!("{filename}: missing the \"reactions\" array.")));
    };

    assert_unique_ids(metabolites, "metabolite", filename)?;
    assert_unique_ids(reactions, "reaction", filename)?;

    for reaction in reactions {
        let id = reaction.get("id").map(js::to_string).unwrap_or_default();
        let mets = reaction.get("metabolites");
        let Some(mets @ Value::Object(_)) = mets else {
            return Err(invalid(format!("{filename}: reaction {id} has no valid \"metabolites\" object.")));
        };
        for (mid, coefficient) in object_entries(mets) {
            if !js::to_number(&coefficient).is_finite() {
                return Err(invalid(format!("{filename}: non-numeric coefficient for {mid} in {id}.")));
            }
        }
    }
    Ok(())
}

fn assert_unique_ids(items: &[Value], kind: &str, filename: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            return Err(invalid(format!("{filename}: a {kind} has no valid id.")));
        };
        if !seen.insert(id) {
            return Err(invalid(format!("{filename}: duplicate {kind} id: {id}.")));
        }
    }
    Ok(())
}

fn safe_id(value: &str) -> String {
    let trimmed = js_trim(value);
    let without_extension = MODEL_EXTENSION.replace(trimmed, "");
    let replaced = UNSAFE_ID.replace_all(&without_extension, "_");
    let id = replaced.trim_matches('_');
    if id.is_empty() { "model".into() } else { id.to_string() }
}

fn unique_id(base: &str, taken: &mut HashSet<String>) -> String {
    let mut id = base.to_string();
    let mut i = 2;
    while taken.contains(&id) {
        id = format!("{base}_{i}");
        i += 1;
    }
    taken.insert(id.clone());
    id
}

// SBML Level 3/FBC

static LOOKS_SBML: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<(?:{W}+:)?(?i-u:sbml){B}")).expect("pattern"));
static LOOKS_MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<(?:{W}+:)?(?i-u:model){B}")).expect("pattern"));
static FORMULA_IN_NOTES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"FORMULA:{}*([^{}<]+)", *jscompat::S, jscompat::WS_CHARS)).expect("pattern"));
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("pattern"));
static WS_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("{}+", *jscompat::S)).expect("pattern"));
static GENE_ASSOCIATION_TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"<(/)?(?:{W}+:)?((?i-u:and|or|geneProductRef)){B}([^>]*)/?{}*>", *jscompat::S)).expect("pattern")
});
static OUTER_PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^\(({DOT}*)\)$")).expect("pattern"));

/// Convert the subset of SBML Level 3 + FBC used by constraint-based metabolic
/// models into Coxeter's COBRA-compatible document. Namespace prefixes are
/// intentionally accepted rather than hard-coded (`fbc:`, `fbc2:`, etc.).
pub fn sbml_to_cobra(text: &str, filename: &str) -> Result<Value> {
    let xml = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    if !LOOKS_SBML.is_match(xml) || !LOOKS_MODEL.is_match(xml) {
        return Err(Error::new(ErrorKind::InvalidSbml, format!("{filename}: does not look like a valid SBML document.")));
    }

    let model_attrs = attrs(first_open_tag(xml, "model"));
    let mut parameters: HashMap<String, f64> = HashMap::new();
    for tag in open_tags(xml, "parameter") {
        let a = attrs(tag);
        if let (Some(id), Some(value)) = (a.get("id").filter(|s| !s.is_empty()), a.get("value")) {
            if finite_number(Some(value)) {
                parameters.insert(id.clone(), js::string_to_number(value));
            }
        }
    }

    let mut compartments = Map::new();
    for tag in open_tags(xml, "compartment") {
        let a = attrs(tag);
        if let Some(id) = a.get("id").filter(|s| !s.is_empty()) {
            let name = a.get("name").filter(|s| !s.is_empty()).unwrap_or(id);
            compartments.insert(unmangle(id, "C_"), json!(name));
        }
    }

    let mut metabolites = Vec::new();
    // Genome-scale SBML files such as Yeast9 contain thousands of species.
    // Rescanning the entire document once per species to recover notes and
    // annotations would make large imports O(n²), so the paired element
    // bodies are indexed once, and the open-tag pass keeps self-closing species
    // supported.
    let species_blocks = index_blocks_by_attr(xml, "species", "id");
    for tag in open_tags(xml, "species") {
        let a = attrs(tag);
        let Some(id) = a.get("id").filter(|s| !s.is_empty()) else { continue };
        // The block lookup needs the id as written. Everything stored uses the
        // id the papers use. SBML forbids most of BiGG's punctuation, so an
        // exporter writes `M_12ppd__45__R_e` and this is where that is undone.
        // Without it a perfectly good upload arrives with every identifier renamed.
        let block = species_blocks.get(id.as_str()).copied().unwrap_or("");
        let notes = extract_notes(block);
        // A formula fbc cannot hold (fractional counts, an underscore) travels
        // in the notes under COBRA's own key, which is also where Level 2 keeps them.
        let formula = match attr_any(&a, "chemicalFormula").filter(|s| !s.is_empty()) {
            Some(f) => f.to_string(),
            None => notes
                .get("text")
                .and_then(Value::as_str)
                .and_then(|t| FORMULA_IN_NOTES.captures(t))
                .map(|c| c[1].to_string())
                .unwrap_or_default(),
        };
        let annotation = extract_identifiers_org(block)?;
        metabolites.push(json!({
            "id": unmangle(id, "M_"),
            "name": a.get("name").filter(|s| !s.is_empty()).unwrap_or(id),
            "compartment": unmangle(a.get("compartment").filter(|s| !s.is_empty()).map_or("c", String::as_str), "C_"),
            "formula": formula,
            "charge": number_or(attr_any(&a, "charge"), 0.0),
            "notes": notes,
            "annotation": annotation,
        }));
    }

    let mut genes = Vec::new();
    let gene_blocks = index_blocks_by_attr(xml, "geneProduct", "id");
    for tag in open_tags(xml, "geneProduct") {
        let a = attrs(tag);
        let Some(id) = attr_any(&a, "id").filter(|s| !s.is_empty()) else { continue };
        let gene_block = gene_blocks.get(id).copied().unwrap_or("");
        let label = attr_any(&a, "label").filter(|s| !s.is_empty()).map_or_else(|| unmangle(id, "G_"), str::to_string);
        let name = attr_any(&a, "name").filter(|s| !s.is_empty()).map_or_else(|| label.clone(), str::to_string);
        genes.push(json!({
            "id": label,
            "name": name,
            "annotation": extract_identifiers_org(gene_block)?,
            "notes": extract_notes(gene_block),
        }));
    }

    let objective = parse_fbc_objective(xml);
    let mut reactions = Vec::new();
    for (open, body) in element_blocks(xml, "reaction") {
        let a = attrs(open);
        let Some(raw_id) = a.get("id").filter(|s| !s.is_empty()) else { continue };
        let mut stoich = Map::new();
        add_species_refs(&mut stoich, inner_of(body, "listOfReactants"), -1.0);
        add_species_refs(&mut stoich, inner_of(body, "listOfProducts"), 1.0);

        let reversible = a.get("reversible").map_or("true", String::as_str).to_lowercase() != "false";
        let lower = resolve_bound(attr_any(&a, "lowerFluxBound"), &parameters, if reversible { -1000.0 } else { 0.0 });
        let upper = resolve_bound(attr_any(&a, "upperFluxBound"), &parameters, 1000.0);
        let gpr_block = inner_of(body, "geneProductAssociation");
        let gpr = if gpr_block.is_empty() { String::new() } else { parse_gene_association(gpr_block) };
        let id = unmangle(raw_id, "R_");
        let coefficient = objective.get(&id).map_or(0.0, js::to_number);
        let notes = extract_notes(body);
        let annotation = extract_identifiers_org(body)?;

        reactions.push(json!({
            "id": id,
            "name": a.get("name").filter(|s| !s.is_empty()).unwrap_or(raw_id),
            "metabolites": js_order(stoich),
            "lower_bound": lower,
            "upper_bound": upper,
            "objective_coefficient": coefficient,
            "gene_reaction_rule": gpr,
            "subsystem": "",
            "notes": notes,
            "annotation": annotation,
        }));
    }

    if metabolites.is_empty() {
        return Err(Error::new(ErrorKind::InvalidSbml, format!("{filename}: SBML contains no species/metabolites.")));
    }
    if reactions.is_empty() {
        return Err(Error::new(ErrorKind::InvalidSbml, format!("{filename}: SBML contains no reactions.")));
    }

    let id = match model_attrs.get("id").filter(|s| !s.is_empty()) {
        Some(id) => unmangle(id, "M_"),
        None => SBML_EXTENSION.replace(filename, "").into_owned(),
    };
    let name = model_attrs
        .get("name")
        .filter(|s| !s.is_empty())
        .or_else(|| model_attrs.get("id").filter(|s| !s.is_empty()))
        .cloned()
        .unwrap_or_else(|| filename.to_string());
    Ok(json!({
        "id": id,
        "name": name,
        "version": "SBML L3 FBC",
        "compartments": js_order(compartments),
        "metabolites": metabolites,
        "reactions": reactions,
        "genes": genes,
        "objective": objective,
    }))
}

fn parse_fbc_objective(xml: &str) -> Map<String, Value> {
    let mut out = Map::new();
    let objectives = inner_of(xml, "listOfObjectives");
    if objectives.is_empty() {
        return out;
    }
    let list_attrs = attrs(first_open_tag(xml, "listOfObjectives"));
    let active = attr_any(&list_attrs, "activeObjective").filter(|s| !s.is_empty());
    let mut selected: Option<&str> = None;
    for (open, body) in element_blocks(objectives, "objective") {
        let a = attrs(open);
        let id = attr_any(&a, "id");
        let is_active = active.is_some() && id == active;
        if selected.is_none() || is_active {
            selected = Some(body);
        }
        if is_active {
            break;
        }
    }
    let Some(selected) = selected else { return out };
    for tag in open_tags(selected, "fluxObjective") {
        let a = attrs(tag);
        let coef = number_or(attr_any(&a, "coefficient"), 1.0);
        if let Some(rid) = attr_any(&a, "reaction").filter(|s| !s.is_empty()) {
            out.insert(unmangle(rid, "R_"), json!(coef));
        }
    }
    js_order(out)
}

fn add_species_refs(target: &mut Map<String, Value>, xml: &str, sign: f64) {
    if xml.is_empty() {
        return;
    }
    for tag in open_tags(xml, "speciesReference") {
        let a = attrs(tag);
        let Some(species) = a.get("species").filter(|s| !s.is_empty()) else { continue };
        let coefficient = number_or(a.get("stoichiometry").map(String::as_str), 1.0) * sign;
        let id = unmangle(species, "M_");
        // `(target[id] || 0) + coefficient`
        let previous = target.get(&id).and_then(Value::as_f64).filter(|v| *v != 0.0 && !v.is_nan()).unwrap_or(0.0);
        target.insert(id, json!(previous + coefficient));
    }
}

fn parse_gene_association(xml: &str) -> String {
    struct Token {
        text: String,
    }
    let mut tokens: Vec<Token> = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();
    for m in GENE_ASSOCIATION_TAGS.captures_iter(xml) {
        let closing = m.get(1).is_some();
        let kind = m[2].to_lowercase();
        if kind == "geneproductref" && !closing {
            let a = attrs(m.get(0).expect("whole match").as_str());
            if let Some(gene) = attr_any(&a, "geneProduct").filter(|s| !s.is_empty()) {
                tokens.push(Token { text: unmangle(gene, "G_") });
            }
        } else if !closing {
            stack.push((kind, tokens.len()));
        } else {
            let Some((kind, start)) = stack.pop() else { continue };
            let children = tokens.split_off(start.min(tokens.len()));
            if !children.is_empty() {
                let text = if children.len() == 1 {
                    children.into_iter().next().expect("one child").text
                } else {
                    format!("({})", children.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(&format!(" {kind} ")))
                };
                tokens.push(Token { text });
            }
        }
    }
    match tokens.last() {
        Some(t) => OUTER_PARENS.replace(&t.text, "$1").into_owned(),
        None => String::new(),
    }
}

fn resolve_bound(reference: Option<&str>, parameters: &HashMap<String, f64>, fallback: f64) -> f64 {
    let Some(r) = reference.filter(|r| !r.is_empty()) else { return fallback };
    if let Some(v) = parameters.get(r) {
        return *v;
    }
    if finite_number(Some(r)) { js::string_to_number(r) } else { fallback }
}

/// `attrAny(object, localName)`: the attribute itself, or the first one whose
/// name ends in `:<localName>` (any namespace prefix).
fn attr_any<'a>(object: &'a IndexMap<String, String>, local_name: &str) -> Option<&'a str> {
    if let Some(v) = object.get(local_name) {
        return Some(v);
    }
    object.iter().find(|(k, _)| k.rsplit(':').next() == Some(local_name)).map(|(_, v)| v.as_str())
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '-')
}

/// `attrs(tag)`: every `name="value"` (or `'value'`) pair, entity-decoded, as
/// the JS pattern `/([:\w.-]+)\s*=\s*(["'])(.*?)\2/g` finds them.
fn attrs(tag: &str) -> IndexMap<String, String> {
    let chars: Vec<(usize, char)> = tag.char_indices().collect();
    let n = chars.len();
    let byte = |i: usize| if i < n { chars[i].0 } else { tag.len() };
    // The value of a match whose name ends before `k`, if there is one.
    let value_after = |mut k: usize| -> Option<(usize, usize)> {
        while k < n && jscompat::is_js_ws(chars[k].1) {
            k += 1;
        }
        if k >= n || chars[k].1 != '=' {
            return None;
        }
        k += 1;
        while k < n && jscompat::is_js_ws(chars[k].1) {
            k += 1;
        }
        let quote = chars.get(k)?.1;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let start = k + 1;
        for (e, (_, c)) in chars.iter().enumerate().skip(start) {
            if *c == quote {
                return Some((start, e));
            }
            if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                return None;
            }
        }
        None
    };
    let mut out = IndexMap::new();
    let mut i = 0;
    while i < n {
        if !is_name_char(chars[i].1) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while end < n && is_name_char(chars[end].1) {
            end += 1;
        }
        // A start inside the same run of name characters fails the same way, so
        // a failed run is skipped whole.
        match value_after(end) {
            Some((vs, ve)) => {
                out.insert(tag[byte(start)..byte(end)].to_string(), decode_xml(&tag[byte(vs)..byte(ve)]));
                i = ve + 1;
            }
            None => i = end,
        }
    }
    js::js_key_order(&mut out);
    out
}

fn tag_pattern(local_name: &str) -> String {
    format!(r"<(?:{W}+:)?(?i-u:{local_name}){B}[^>]*>")
}

/// `openTags(xml, localName)`: every opening (or self-closing) tag of that
/// local name, any prefix, any case.
fn open_tags<'a>(xml: &'a str, local_name: &str) -> Vec<&'a str> {
    jscompat::regex(&tag_pattern(local_name)).find_iter(xml).map(|m| m.as_str()).collect()
}

fn first_open_tag<'a>(xml: &'a str, local_name: &str) -> &'a str {
    jscompat::regex(&tag_pattern(local_name)).find(xml).map_or("", |m| m.as_str())
}

fn block_pattern(local_name: &str) -> String {
    format!(r"(<(?:{W}+:)?(?i-u:{local_name}){B}[^>]*>)((?s:.)*?)</(?:{W}+:)?(?i-u:{local_name}){}*>", *jscompat::S)
}

/// `elementBlocks(xml, localName)`: `(open tag, body)` of every element with a
/// closing tag. A self-closing one runs to the next closing tag, as in the JS.
fn element_blocks<'a>(xml: &'a str, local_name: &str) -> Vec<(&'a str, &'a str)> {
    jscompat::regex(&block_pattern(local_name))
        .captures_iter(xml)
        .map(|c| (c.get(1).expect("group").as_str(), c.get(2).expect("group").as_str()))
        .collect()
}

fn inner_of<'a>(xml: &'a str, local_name: &str) -> &'a str {
    jscompat::regex(&block_pattern(local_name))
        .captures(xml)
        .map_or("", |c| c.get(2).expect("group").as_str())
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// `value !== '' && value != null && Number.isFinite(Number(value))`.
fn finite_number(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty() && js::string_to_number(v).is_finite())
}

fn number_or(value: Option<&str>, fallback: f64) -> f64 {
    match value {
        Some(v) if finite_number(Some(v)) => js::string_to_number(v),
        _ => fallback,
    }
}

fn index_blocks_by_attr<'a>(xml: &'a str, local_name: &str, attr: &str) -> HashMap<String, &'a str> {
    let mut out = HashMap::new();
    for (open, body) in element_blocks(xml, local_name) {
        let a = attrs(open);
        if let Some(value) = attr_any(&a, attr).filter(|s| !s.is_empty()) {
            out.insert(value.to_string(), body);
        }
    }
    out
}

fn extract_notes(xml: &str) -> Value {
    let body = inner_of(xml, "notes");
    if body.is_empty() {
        return json!({});
    }
    let spaced = TAG.replace_all(body, " ");
    let collapsed = WS_RUN.replace_all(&spaced, " ");
    let text = decode_xml(js_trim(&collapsed));
    if text.is_empty() { json!({}) } else { json!({ "text": text }) }
}

// COBRA Toolbox MAT and Excel

/// A file the user picked, as bytes: what MATLAB and Excel files need.
#[derive(Debug, Clone)]
pub struct UploadFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// `modelFileKind(filename, bytes)`: what an uploaded file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFileKind {
    Mat,
    Xlsx,
    /// SBML or COBRA JSON.
    Text,
}

/// `modelFileKind(filename, bytes)`: by the extension, else by the first
/// bytes. The old binary Excel format is an error.
pub fn model_file_kind(filename: &str, bytes: &[u8]) -> Result<ModelFileKind> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".mat") {
        return Ok(ModelFileKind::Mat);
    }
    if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") {
        return Ok(ModelFileKind::Xlsx);
    }
    if lower.ends_with(".xls") {
        return Err(invalid(format!("{filename}: Excel 97-2003 workbooks (.xls) cannot be read; save it as .xlsx.")));
    }
    if bytes.starts_with(b"MATLAB") {
        return Ok(ModelFileKind::Mat);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return Ok(ModelFileKind::Xlsx);
    }
    Ok(ModelFileKind::Text)
}

/// `isBinaryModelFile(filename)`: whether a picked file has to be read as
/// bytes rather than text.
pub fn is_binary_model_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    [".mat", ".xlsx", ".xlsm", ".xls"].iter().any(|ext| lower.ends_with(ext))
}

/// `parseModelUpload(file, index)`: one uploaded file in whichever format it
/// is (see [`model_file_kind`]).
pub fn parse_model_upload(file: &UploadFile, index: usize) -> Result<Model> {
    let filename = if file.name.is_empty() { format!("model-{}.json", index + 1) } else { file.name.clone() };
    match model_file_kind(&filename, &file.bytes)? {
        ModelFileKind::Mat => parse_model_mat(&file.bytes, &filename),
        ModelFileKind::Xlsx => parse_model_xlsx(&file.bytes, &filename),
        ModelFileKind::Text => parse_model_file(&String::from_utf8_lossy(&file.bytes), &filename),
    }
}

/// `prepareModelUploads(files)`: [`prepare_imported_models`] for uploads in
/// any format, each file parsed by [`parse_model_upload`] first.
pub fn prepare_model_uploads(files: &[UploadFile]) -> Result<PreparedImport> {
    if files.is_empty() {
        return Err(no_files());
    }
    let parsed = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let filename = if file.name.is_empty() { format!("model-{}.json", index + 1) } else { file.name.clone() };
            Ok((filename, parse_model_upload(file, index)?))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(prepare_parsed(parsed))
}

/// `parseModelMAT(bytes, filename)`: a COBRA Toolbox MATLAB file (MAT v5, or
/// v7 with compression): the first structure in it with rxns, mets and S. A
/// v7.3 file is HDF5, which this reader cannot open, and the error says how
/// to save it again.
pub fn parse_model_mat(bytes: &[u8], filename: &str) -> Result<Model> {
    if is_mat_hdf5(bytes) {
        return Err(invalid(format!(
            "{filename}: this is a MATLAB v7.3 (HDF5) file, which cannot be read here. In MATLAB, save the model again as v7 (save('model.mat', 'model', '-v7')) or export it with writeCbModel as SBML."
        )));
    }
    let variables = read_mat5(bytes).map_err(|e| invalid(format!("{filename}: {}", e.message)))?;
    let Some((name, value)) = find_cobra_struct(&variables) else {
        return Err(invalid(format!("{filename}: no COBRA model in the file (a structure with rxns, mets and S).")));
    };
    let raw = cobra_struct_to_document(&value, filename, &name)?;
    validate_raw_model(&raw, filename)?;
    build_model(&raw, filename)
}

/// `parseModelXLSX(bytes, filename)`: a COBRA Toolbox Excel workbook
/// (writeCbModel's "Reaction List" and "Metabolite List" sheets). Column
/// names are matched loosely (case, spaces and common synonyms), and a
/// metabolite only named in an equation is still created.
pub fn parse_model_xlsx(bytes: &[u8], filename: &str) -> Result<Model> {
    let workbook = read_workbook(bytes.to_vec()).map_err(|e| invalid(format!("{filename}: not a readable Excel workbook ({}).", e.message)))?;
    let raw = cobra_workbook_to_document(&workbook, filename)?;
    validate_raw_model(&raw, filename)?;
    build_model(&raw, filename)
}

fn build_model(raw: &Value, filename: &str) -> Result<Model> {
    let model = Model::from_value(raw).map_err(|e| invalid(format!("{filename}: {e}")))?;
    if model.reactions.is_empty() {
        return Err(invalid(format!("{filename}: the model contains no reactions.")));
    }
    if model.metabolites.is_empty() {
        return Err(invalid(format!("{filename}: the model contains no metabolites.")));
    }
    Ok(model)
}

/// A v7.3 MAT file: version 0x0200 in its header, or the HDF5 signature
/// after the 512-byte user block.
fn is_mat_hdf5(bytes: &[u8]) -> bool {
    if bytes.len() < 128 {
        return false;
    }
    let version = if bytes[126] == 0x49 {
        u16::from(bytes[124]) | (u16::from(bytes[125]) << 8)
    } else {
        (u16::from(bytes[124]) << 8) | u16::from(bytes[125])
    };
    let hdf5 = bytes.len() >= 516 && bytes[512] == 0x89 && &bytes[513..516] == b"HDF";
    bytes.starts_with(b"MATLAB 7.3") || version == 0x0200 || hdf5
}

/// The first variable that is a COBRA structure, or the first one inside a
/// cell or structure array (named `var{n}`).
fn find_cobra_struct(variables: &MatVariables) -> Option<(String, Cow<'_, MatValue>)> {
    let has_fields = |fields: &IndexMap<String, MatValue>| ["rxns", "mets", "S"].iter().all(|k| fields.contains_key(*k));
    for (name, value) in variables {
        match value {
            MatValue::Struct(fields) if has_fields(fields) => return Some((name.clone(), Cow::Borrowed(value))),
            MatValue::Cell(items) => {
                if let Some(index) = items.iter().position(|v| matches!(v, MatValue::Struct(f) if has_fields(f))) {
                    return Some((format!("{name}{{{}}}", index + 1), Cow::Borrowed(&items[index])));
                }
            }
            // A structure array holds its elements as field maps, not values.
            MatValue::StructArray(items) => {
                if let Some(index) = items.iter().position(has_fields) {
                    return Some((format!("{name}{{{}}}", index + 1), Cow::Owned(MatValue::Struct(items[index].clone()))));
                }
            }
            _ => {}
        }
    }
    None
}

/// `text(value)`: a string, or the text of each item joined with `; `.
fn mat_text(value: &MatValue) -> String {
    match value {
        MatValue::Text(s) => s.clone(),
        MatValue::TextRows(rows) => rows.iter().filter(|r| !r.is_empty()).cloned().collect::<Vec<_>>().join("; "),
        MatValue::Cell(items) => items.iter().map(mat_text).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("; "),
        MatValue::Numeric { data, .. } => data.values.iter().map(|v| js::number_to_string(*v)).collect::<Vec<_>>().join("; "),
        MatValue::Struct(_) | MatValue::StructArray(_) | MatValue::Sparse(_) => String::new(),
    }
}

/// `list(value).map(text)`.
fn mat_texts(value: Option<&MatValue>) -> Vec<String> {
    match value {
        None => Vec::new(),
        Some(MatValue::Cell(items)) => items.iter().map(mat_text).collect(),
        Some(MatValue::TextRows(rows)) => rows.clone(),
        Some(MatValue::Numeric { data, .. }) => data.values.iter().map(|v| js::number_to_string(*v)).collect(),
        Some(MatValue::StructArray(items)) => items.iter().map(|_| String::new()).collect(),
        Some(other) => vec![mat_text(other)],
    }
}

/// `list(value).map(Number)`.
fn mat_numbers(value: Option<&MatValue>) -> Vec<f64> {
    let one = |v: &MatValue| match v {
        MatValue::Numeric { data, .. } => match data.values.as_slice() {
            [] => 0.0,
            [x] => *x,
            _ => f64::NAN,
        },
        MatValue::Text(s) => js::string_to_number(s),
        _ => f64::NAN,
    };
    match value {
        None => Vec::new(),
        Some(MatValue::Numeric { data, .. }) => data.values.clone(),
        Some(MatValue::Cell(items)) => items.iter().map(one).collect(),
        Some(MatValue::TextRows(rows)) => rows.iter().map(|r| js::string_to_number(r)).collect(),
        Some(MatValue::StructArray(items)) => items.iter().map(|_| f64::NAN).collect(),
        Some(other) => vec![one(other)],
    }
}

/// How many items `list(value)` has.
fn mat_len(value: &MatValue) -> usize {
    match value {
        MatValue::Cell(items) => items.len(),
        MatValue::TextRows(rows) => rows.len(),
        MatValue::Numeric { data, .. } => data.values.len(),
        MatValue::StructArray(items) => items.len(),
        _ => 1,
    }
}

static BRACKET_COMPARTMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^({DOT}+)\[([^\]{}]+)\]$", jscompat::WS_CHARS)).expect("pattern"));

/// `cobraMetaboliteId(id)`: a COBRA Toolbox metabolite id as the other
/// importers write it: the trailing `[c]` becomes `_c`, which is how the
/// Toolbox's own SBML export spells it, so one model reads the same from
/// .mat, .xlsx and .xml. Returns the id and the bracketed compartment.
fn cobra_metabolite_id(id: &str) -> (String, Option<String>) {
    let trimmed = js_trim(id);
    match BRACKET_COMPARTMENT.captures(trimmed) {
        Some(c) => (format!("{}_{}", &c[1], &c[2]), Some(c[2].to_string())),
        None => (trimmed.to_string(), None),
    }
}

/// The converted id of every metabolite; one that would collide with
/// another keeps its own spelling.
fn metabolite_ids(originals: &[String]) -> Vec<(String, Option<String>)> {
    let listed: HashSet<&str> = originals.iter().map(String::as_str).collect();
    let mut taken: HashSet<String> = HashSet::new();
    originals
        .iter()
        .map(|original| {
            let (converted, compartment) = cobra_metabolite_id(original);
            let clash = taken.contains(&converted) || (converted != *original && listed.contains(converted.as_str()));
            let id = if clash { original.clone() } else { converted };
            taken.insert(id.clone());
            (id, compartment)
        })
        .collect()
}

static INDEXED_GENE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"x\(([0-9]+)\)").expect("pattern"));
static AMPERSANDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&+").expect("pattern"));
static BARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\|+").expect("pattern"));

/// `structGeneRule(grRule, encoded, genes)`: the rule of grRules, or of
/// `rules` (`x(3) & x(5)`) and `genes` when grRules is missing or unreadable.
fn struct_gene_rule(gr_rule: &str, encoded: &str, genes: &[String]) -> String {
    let readable = |rule: &str| evaluate_gene_rule(rule, &|_| false).is_ok();
    if !gr_rule.is_empty() && readable(gr_rule) {
        return gr_rule.to_string();
    }
    if !encoded.is_empty() {
        let named = INDEXED_GENE.replace_all(encoded, |c: &regex::Captures| {
            let n: usize = c[1].parse().unwrap_or(0);
            n.checked_sub(1).and_then(|i| genes.get(i)).cloned().unwrap_or_else(|| c[0].to_string())
        });
        let words = BARS.replace_all(&AMPERSANDS.replace_all(&named, " and "), " or ").into_owned();
        let translated = js_trim(&WS_RUN.replace_all(&words, " ")).to_string();
        if readable(&translated) {
            return translated;
        }
    }
    gr_rule.to_string()
}

/// S sparse or dense (column-major) as reaction j's `(row, value)` entries.
fn sparse_columns(s: Option<&MatValue>, m: usize, n: usize, filename: &str) -> Result<Vec<Vec<(usize, f64)>>> {
    match s {
        Some(MatValue::Sparse(sp)) => {
            if sp.shape.first() != Some(&m) || sp.shape.get(1) != Some(&n) {
                let shape = sp.shape.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("×");
                return Err(invalid(format!("{filename}: S is {shape}, but there are {m} metabolites and {n} reactions.")));
            }
            Ok((0..n)
                .map(|j| {
                    let (a, b) = (sp.jc.values[j] as usize, sp.jc.values[j + 1] as usize);
                    (a..b).map(|k| (sp.ir.values[k] as usize, sp.values.values[k])).collect()
                })
                .collect())
        }
        Some(MatValue::Numeric { data, .. }) if data.values.len() == m * n => Ok((0..n)
            .map(|j| (0..m).map(|i| (i, data.values[i + j * m])).filter(|(_, v)| *v != 0.0 && !v.is_nan()).collect())
            .collect()),
        _ => Err(invalid(format!("{filename}: S is not a {m}×{n} matrix."))),
    }
}

fn cobra_struct_to_document(raw: &MatValue, filename: &str, variable: &str) -> Result<Value> {
    let field = |name: &str| raw.field(name);
    let rxns = mat_texts(field("rxns"));
    let met_ids = mat_texts(field("mets"));
    let (m, n) = (met_ids.len(), rxns.len());
    let columns = sparse_columns(field("S"), m, n, filename)?;
    let lb = mat_numbers(field("lb"));
    let ub = mat_numbers(field("ub"));
    let c = mat_numbers(field("c"));
    let rev = mat_numbers(field("rev"));
    let genes = mat_texts(field("genes"));
    let gene_names = mat_texts(field("geneNames"));
    let gr_rules = mat_texts(field("grRules"));
    let encoded_rules = mat_texts(field("rules"));
    let names = mat_texts(field("rxnNames"));
    let subsystems = mat_texts(field("subSystems"));
    let met_names = mat_texts(field("metNames"));
    let formulas = mat_texts(field("metFormulas"));
    let charges = mat_numbers(field("metCharges").or_else(|| field("metCharge")));
    let comps = mat_texts(field("comps"));
    let comp_names = mat_texts(field("compNames"));
    let met_comps = mat_numbers(field("metComps"));
    let at = |list: &[String], i: usize| list.get(i).cloned().unwrap_or_default();

    let mut compartments = Map::new();
    for (i, id) in comps.iter().enumerate() {
        if !id.is_empty() {
            let name = at(&comp_names, i);
            compartments.insert(id.clone(), json!(if name.is_empty() { id.clone() } else { name }));
        }
    }
    let ids = metabolite_ids(&met_ids);
    let metabolites: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, (id, bracketed))| {
            let indexed = met_comps.get(i).filter(|v| v.fract() == 0.0 && **v >= 1.0).and_then(|v| comps.get(*v as usize - 1)).cloned();
            let compartment = bracketed.clone().or(indexed).unwrap_or_else(|| infer_compartment(id));
            compartments.entry(compartment.clone()).or_insert_with(|| json!(compartment));
            let name = at(&met_names, i);
            let charge = charges.get(i).copied().unwrap_or(f64::NAN);
            json!({
                "id": id,
                "name": if name.is_empty() { met_ids[i].clone() } else { name },
                "compartment": compartment,
                "formula": at(&formulas, i),
                "charge": if charge.is_finite() { charge } else { 0.0 },
            })
        })
        .collect();

    let mut reactions = Vec::with_capacity(n);
    for (j, id) in rxns.iter().enumerate() {
        let mut stoich = Map::new();
        for (row, value) in &columns[j] {
            if !value.is_finite() {
                return Err(invalid(format!("{filename}: reaction {id} has a non-numeric coefficient.")));
            }
            if *value != 0.0 {
                let mid = &ids[*row].0;
                let previous = stoich.get(mid).and_then(Value::as_f64).unwrap_or(0.0);
                stoich.insert(mid.clone(), json!(previous + value));
            }
        }
        let lower = lb.get(j).copied().unwrap_or(f64::NAN);
        let upper = ub.get(j).copied().unwrap_or(f64::NAN);
        let reversible = rev.get(j).is_some_and(|v| *v != 0.0 && !v.is_nan());
        let objective = c.get(j).copied().filter(|v| !v.is_nan()).unwrap_or(0.0);
        let name = at(&names, j);
        reactions.push(json!({
            "id": id,
            "name": if name.is_empty() { id.clone() } else { name },
            "metabolites": Value::Object(js_order(stoich)),
            "lower_bound": if lower.is_finite() { lower } else if reversible { -1000.0 } else { 0.0 },
            "upper_bound": if upper.is_finite() { upper } else { 1000.0 },
            "objective_coefficient": objective,
            "gene_reaction_rule": struct_gene_rule(&at(&gr_rules, j), &at(&encoded_rules, j), &genes),
            "subsystem": at(&subsystems, j),
        }));
    }

    let model_id = field("modelID").or_else(|| field("id")).map(mat_text).unwrap_or_default();
    let stem = MODEL_EXTENSION.replace(filename, "").into_owned();
    let model_name = field("modelName").or_else(|| field("name")).map(mat_text).unwrap_or_default();
    let description = field("description").map(mat_text).unwrap_or_default();
    let mut doc = Map::new();
    doc.insert("id".into(), json!(first_non_empty(&[model_id.as_str(), stem.as_str(), variable])));
    doc.insert("name".into(), json!(first_non_empty(&[model_name.as_str(), description.as_str(), variable])));
    doc.insert("version".into(), json!("COBRA Toolbox MAT"));
    doc.insert("compartments".into(), Value::Object(js_order(compartments)));
    doc.insert("metabolites".into(), Value::Array(metabolites));
    doc.insert("reactions".into(), Value::Array(reactions));
    doc.insert(
        "genes".into(),
        Value::Array(genes.iter().enumerate().map(|(i, g)| json!({"id": g, "name": first_non_empty(&[at(&gene_names, i).as_str(), g.as_str()])})).collect()),
    );
    // Further met*/rxn* text vectors (KEGG ids, EC numbers, ...) are label
    // sources the Model reads off the document, as it does for any COBRA file.
    const HANDLED: [&str; 8] = ["mets", "metNames", "metFormulas", "metCharges", "metCharge", "metComps", "rxns", "rxnNames"];
    if let MatValue::Struct(fields) = raw {
        for (name, value) in fields {
            let lower = name.to_ascii_lowercase();
            if HANDLED.contains(&name.as_str()) || !(lower.starts_with("met") || lower.starts_with("rxn")) {
                continue;
            }
            let count = if lower.starts_with("met") { m } else { n };
            let texty = match value {
                MatValue::Cell(items) => items.iter().all(|x| matches!(x, MatValue::Text(_) | MatValue::TextRows(_) | MatValue::Cell(_))),
                MatValue::Text(_) | MatValue::TextRows(_) => true,
                _ => false,
            };
            if texty && mat_len(value) == count {
                doc.insert(name.clone(), json!(mat_texts(Some(value))));
            }
        }
    }
    Ok(Value::Object(doc))
}

/// A sheet's column fields, in the order a header name is matched against them.
type Columns = &'static [(&'static str, &'static [&'static str])];

// Column names are compared lower-cased, with everything but letters and digits taken out.
const REACTION_COLUMNS: Columns = &[
    ("id", &["abbreviation", "id", "rxn", "rxns", "reactionid", "rxnid", "reactionabbreviation", "abbrev"]),
    ("name", &["description", "name", "rxnname", "rxnnames", "reactionname", "reactiondescription"]),
    ("formula", &["reaction", "formula", "equation", "reactionformula", "rxnformula", "reactionequation"]),
    ("gpr", &["gpr", "grrule", "grrules", "generule", "generules", "genereactionrule", "genereactionassociation"]),
    ("subsystem", &["subsystem", "subsystems", "pathway"]),
    ("reversible", &["reversible", "rev", "reversibility"]),
    ("lower", &["lowerbound", "lowerbounds", "lb", "lower", "min"]),
    ("upper", &["upperbound", "upperbounds", "ub", "upper", "max"]),
    ("objective", &["objective", "objectivecoefficient", "c", "obj"]),
    ("ec", &["ecnumber", "ecnumbers", "ec", "rxnecnumbers"]),
];
const METABOLITE_COLUMNS: Columns = &[
    ("id", &["abbreviation", "id", "met", "mets", "metaboliteid", "metid", "abbrev"]),
    ("name", &["description", "name", "metname", "metnames", "metabolitename", "fullname"]),
    ("charged", &["chargedformula", "formula", "metformula", "metformulas"]),
    ("neutral", &["neutralformula"]),
    ("charge", &["charge", "metcharge", "metcharges"]),
    ("compartment", &["compartment", "comp", "compartments", "metcompartment"]),
    ("kegg", &["keggid", "kegg", "metkeggid"]),
    ("pubchem", &["pubchemid", "pubchem"]),
    ("chebi", &["chebiid", "chebi"]),
    ("hmdb", &["hmdbid", "hmdb"]),
];

fn column_key(name: &str) -> String {
    name.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

type SheetRow = IndexMap<&'static str, String>;

/// `readSheet(workbook, sheet, columns)`: a sheet as field → value rows,
/// read under the first of its first ten rows that names an id column and
/// one more known column; `None` when none does.
fn read_sheet(workbook: &Workbook, sheet: &str, columns: Columns) -> Result<Option<Vec<SheetRow>>> {
    let mut rows = Vec::new();
    let mut header: Option<IndexMap<String, &'static str>> = None;
    let mut seen = 0;
    for row in workbook.rows(sheet)? {
        let row = row?;
        let Some(fields) = header.as_ref() else {
            seen += 1;
            if seen > 10 {
                break;
            }
            let mut fields: IndexMap<String, &'static str> = IndexMap::new();
            for (letter, value) in &row.cells {
                let key = column_key(value);
                let field = columns.iter().find(|(_, aliases)| aliases.contains(&key.as_str())).map(|(f, _)| *f);
                if let Some(field) = field.filter(|f| !fields.values().any(|v| v == f)) {
                    fields.insert(letter.clone(), field);
                }
            }
            if fields.values().any(|f| *f == "id") && fields.len() >= 2 {
                header = Some(fields);
            }
            continue;
        };
        let mut out = SheetRow::new();
        for (letter, field) in fields {
            if let Some(value) = row.cells.get(letter) {
                out.insert(*field, js_trim(value).to_string());
            }
        }
        if out.get("id").is_some_and(|id| !id.is_empty()) {
            rows.push(out);
        }
    }
    Ok(header.map(|_| rows))
}

/// `findSheet(workbook, names, columns, needs)`: the rows of the sheet
/// called one of `names`, or else of the first sheet that reads under
/// `columns` (with a `needs` value in some row, when given).
fn find_sheet(workbook: &Workbook, names: &[&str], columns: Columns, needs: Option<&str>) -> Result<Option<Vec<SheetRow>>> {
    let sheets = workbook.sheet_names();
    if let Some(named) = sheets.iter().find(|s| names.contains(&column_key(s).as_str())) {
        return read_sheet(workbook, named, columns);
    }
    for name in &sheets {
        if let Some(rows) = read_sheet(workbook, name, columns)? {
            if !rows.is_empty() && needs.is_none_or(|f| rows.iter().any(|r| r.get(f).is_some_and(|v| !v.is_empty()))) {
                return Ok(Some(rows));
            }
        }
    }
    Ok(None)
}

/// `parseCobraFormula(formula)`: a COBRA equation's stoichiometry (by the
/// metabolite ids as written) and whether its arrow is reversible.
#[derive(Debug, Clone, PartialEq)]
pub struct CobraFormula {
    pub stoichiometry: IndexMap<String, f64>,
    pub reversible: bool,
}

static FORMULA_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^\[([^\]]+)\]{s}*:{s}*", s = *jscompat::S)).expect("pattern"));
static ARROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"{s}*(<==>|<=>|<->|<--|<-|-->|->|=>){s}*", s = *jscompat::S)).expect("pattern"));
static PLUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"{s}+\+{s}+", s = *jscompat::S)).expect("pattern"));
static TERM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^(?:\(?{s}*([0-9]*\.?[0-9]+(?:[eE][-+]?[0-9]+)?){s}*\)?{s}+)?({ns}+)$", s = *jscompat::S, ns = *jscompat::NS))
        .expect("pattern")
});
static BRACKET_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]+\]$").expect("pattern"));

/// `parseCobraFormula(formula)`: `2 atp[c] + glc[c] -> ...`, `<=>` for
/// reversible, coefficients bare or in parentheses, a backward arrow turned
/// round, and the old `[c] : a + b -> c` form that puts every metabolite in
/// one compartment. The error says what could not be read.
pub fn parse_cobra_formula(formula: &str) -> std::result::Result<CobraFormula, String> {
    let mut rest = js_trim(formula);
    let prefix = FORMULA_PREFIX.captures(rest).map(|c| (c[0].len(), c[1].to_string()));
    if let Some((len, _)) = &prefix {
        rest = &rest[*len..];
    }
    let Some(arrow) = ARROW.captures(rest) else { return Err(format!("no reaction arrow in \"{formula}\"")) };
    let whole = arrow.get(0).expect("whole match");
    let symbol = arrow[1].to_string();
    let (mut left, mut right) = (&rest[..whole.start()], &rest[whole.end()..]);
    if symbol.starts_with('<') && !symbol.ends_with('>') {
        std::mem::swap(&mut left, &mut right);
    }
    let mut stoichiometry: IndexMap<String, f64> = IndexMap::new();
    for (part, sign) in [(left, -1.0), (right, 1.0)] {
        for term in PLUS.split(part).map(js_trim).filter(|t| !t.is_empty()) {
            let Some(m) = TERM.captures(term) else { return Err(format!("cannot read \"{term}\" in \"{formula}\"")) };
            let coefficient = m.get(1).map_or(1.0, |c| js::string_to_number(c.as_str()));
            let met = match &prefix {
                Some((_, comp)) if !BRACKET_SUFFIX.is_match(&m[2]) => format!("{}[{comp}]", &m[2]),
                _ => m[2].to_string(),
            };
            *stoichiometry.entry(met).or_insert(0.0) += sign * coefficient;
        }
    }
    Ok(CobraFormula { stoichiometry, reversible: ["<==>", "<=>", "<->"].contains(&symbol.as_str()) })
}

static REVERSIBLE_YES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i)(1|true|yes|y|reversible)$").expect("pattern"));
static SHORT_COMPARTMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_]{1,3}$").expect("pattern"));

fn cobra_workbook_to_document(workbook: &Workbook, filename: &str) -> Result<Value> {
    let xlsx_error = |e: Error| invalid(format!("{filename}: not a readable Excel workbook ({}).", e.message));
    let reaction_rows = find_sheet(workbook, &["reactionlist", "reactions", "reaction", "rxns"], REACTION_COLUMNS, Some("formula"))
        .map_err(xlsx_error)?
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            invalid(format!("{filename}: no reaction sheet with an Abbreviation and a Reaction column (the COBRA Toolbox \"Reaction List\")."))
        })?;
    let metabolite_rows =
        find_sheet(workbook, &["metabolitelist", "metabolites", "metabolite", "mets"], METABOLITE_COLUMNS, None).map_err(xlsx_error)?.unwrap_or_default();

    let mut compartments = Map::new();
    let mut metabolites: Vec<Value> = Vec::new();
    let mut known: HashSet<String> = HashSet::new();
    let vector_fields = [("metKEGGID", "kegg"), ("metPubChemID", "pubchem"), ("metChEBIID", "chebi"), ("metHMDBID", "hmdb")];
    let mut vectors: Vec<Vec<String>> = vec![Vec::new(); vector_fields.len()];
    let empty = SheetRow::new();
    let mut add_metabolite = |original: &str, row: &SheetRow| -> String {
        let (id, bracketed) = cobra_metabolite_id(original);
        if !known.insert(id.clone()) {
            return id;
        }
        let get = |k: &str| row.get(k).cloned().unwrap_or_default();
        // The Compartment column holds an id or a name; the id in brackets wins.
        let listed = get("compartment");
        let compartment = bracketed.unwrap_or_else(|| if SHORT_COMPARTMENT.is_match(&listed) { listed.clone() } else { infer_compartment(&id) });
        let current = compartments.get(&compartment).and_then(Value::as_str).map(str::to_string);
        if current.as_deref().is_none_or(|c| c == compartment) {
            let name = if !listed.is_empty() && listed != compartment { listed.clone() } else { compartment.clone() };
            compartments.insert(compartment.clone(), json!(name));
        }
        let charge = row.get("charge").map_or(f64::NAN, |c| js::string_to_number(c));
        let name = get("name");
        let (charged, neutral) = (get("charged"), get("neutral"));
        let formula = first_non_empty(&[charged.as_str(), neutral.as_str()]).to_string();
        metabolites.push(json!({
            "id": id,
            "name": if name.is_empty() { js_trim(original).to_string() } else { name },
            "compartment": compartment,
            "formula": formula,
            "charge": if charge.is_finite() { charge } else { 0.0 },
        }));
        for (slot, (_, key)) in vectors.iter_mut().zip(vector_fields) {
            slot.push(get(key));
        }
        id
    };
    for row in &metabolite_rows {
        add_metabolite(&row["id"], row);
    }

    let mut reactions = Vec::with_capacity(reaction_rows.len());
    let mut ec = Vec::with_capacity(reaction_rows.len());
    for row in &reaction_rows {
        let get = |k: &str| row.get(k).cloned();
        let id = &row["id"];
        let parsed = parse_cobra_formula(&get("formula").unwrap_or_default()).map_err(|e| invalid(format!("{filename}: reaction {id}: {e}.")))?;
        let mut stoich = Map::new();
        for (met, coefficient) in &parsed.stoichiometry {
            let mid = add_metabolite(met, &empty);
            let previous = stoich.get(&mid).and_then(Value::as_f64).unwrap_or(0.0);
            stoich.insert(mid, json!(previous + coefficient));
        }
        let reversible = get("reversible").map_or(parsed.reversible, |v| REVERSIBLE_YES.is_match(js_trim(&v)));
        let number = |k: &str| get(k).map(|v| js::string_to_number(&v)).filter(|v| v.is_finite());
        let name = get("name").unwrap_or_default();
        reactions.push(json!({
            "id": id,
            "name": if name.is_empty() { id.clone() } else { name },
            "metabolites": Value::Object(js_order(stoich)),
            "lower_bound": number("lower").unwrap_or(if reversible { -1000.0 } else { 0.0 }),
            "upper_bound": number("upper").unwrap_or(1000.0),
            "objective_coefficient": get("objective").map_or(0.0, |v| { let x = js::string_to_number(&v); if x.is_nan() { 0.0 } else { x } }),
            "gene_reaction_rule": get("gpr").unwrap_or_default(),
            "subsystem": get("subsystem").unwrap_or_default(),
        }));
        ec.push(get("ec").unwrap_or_default());
    }

    let mut genes: Vec<String> = Vec::new();
    for reaction in &reactions {
        for gene in gene_rule_genes(reaction["gene_reaction_rule"].as_str().unwrap_or("")) {
            if !genes.contains(&gene) {
                genes.push(gene);
            }
        }
    }
    let stem = MODEL_EXTENSION.replace(filename, "").into_owned();
    let stem = if stem.is_empty() { "model".to_string() } else { stem };
    let mut doc = Map::new();
    doc.insert("id".into(), json!(stem));
    doc.insert("name".into(), json!(stem));
    doc.insert("version".into(), json!("COBRA Toolbox XLSX"));
    doc.insert("compartments".into(), Value::Object(js_order(compartments)));
    doc.insert("metabolites".into(), Value::Array(metabolites));
    doc.insert("reactions".into(), Value::Array(reactions));
    doc.insert("genes".into(), Value::Array(genes.iter().map(|g| json!({"id": g, "name": g})).collect()));
    if ec.iter().any(|e| !e.is_empty()) {
        doc.insert("rxnECNumbers".into(), json!(ec));
    }
    for ((field, _), values) in vector_fields.iter().zip(vectors) {
        if values.iter().any(|v| !v.is_empty()) {
            doc.insert((*field).into(), json!(values));
        }
    }
    Ok(Value::Object(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_read_like_the_js_pattern() {
        let a = attrs(r#"<reaction id="R_x" name='a &amp; b' fbc:lowerFluxBound = "lb" metaid="m"/>"#);
        assert_eq!(a["id"], "R_x");
        assert_eq!(a["name"], "a & b");
        assert_eq!(a["fbc:lowerFluxBound"], "lb");
        assert_eq!(attr_any(&a, "lowerFluxBound"), Some("lb"));
        // A value may not cross a line break, so the pair is skipped.
        let b = attrs("<x a=\"1\n\" b=\"2\">");
        assert_eq!(b.get("a"), None);
        assert_eq!(b["b"], "2");
    }

    #[test]
    fn gene_associations_render_with_parentheses_per_group() {
        let xml = r#"<fbc:or><fbc:and><fbc:geneProductRef fbc:geneProduct="G_a"/><fbc:geneProductRef fbc:geneProduct="G_b"/></fbc:and><fbc:geneProductRef fbc:geneProduct="G_c"/></fbc:or>"#;
        assert_eq!(parse_gene_association(xml), "(a and b) or c");
    }

    #[test]
    fn safe_ids() {
        assert_eq!(safe_id(" my model.json "), "my_model");
        assert_eq!(safe_id("__"), "model");
    }
}
