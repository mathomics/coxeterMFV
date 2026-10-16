//! Port of `src/core/model-import.js`: import ordinary COBRA JSON or SBML
//! Level 3/FBC models supplied by the user.
//!
//! The browser-facing code hands this module `{name, text}` records.
//! Everything here is pure and independently testable: JSON parsing,
//! structural validation, Model construction, stable species/model
//! identifiers and medium inference.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

use coxeter_core::js;
use coxeter_core::model::Model;

use crate::error::{Error, ErrorKind, Result};
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

static MODEL_EXTENSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?i-u:json|xml|sbml)$").expect("pattern"));
static SBML_EXTENSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?i-u:xml|sbml)$").expect("pattern"));
static UNSAFE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_.-]+").expect("pattern"));

/// Read every file, give each a distinct species id and model key, and infer
/// the community medium from their exchange reactions.
pub fn prepare_imported_models(files: &[ImportFile]) -> Result<PreparedImport> {
    if files.is_empty() {
        return Err(invalid("Select at least one COBRA JSON or SBML model.".into()));
    }
    let mut taken_species = HashSet::new();
    let mut taken_keys = HashSet::new();
    let mut entries = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let filename = if file.name.is_empty() { format!("model-{}.json", index + 1) } else { file.name.clone() };
        let model = parse_model_file(&file.text, &filename)?;
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
            abundance: 1.0 / files.len() as f64,
            enabled: true,
            summary,
        });
    }
    let medium = infer_community_medium(&entries.iter().map(|e| &e.model).collect::<Vec<_>>());
    Ok(PreparedImport { entries, medium })
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
