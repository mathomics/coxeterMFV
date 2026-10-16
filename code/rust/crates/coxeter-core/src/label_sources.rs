//! Where a map label can come from: the model's id or name, or one of the
//! cross-references (BiGG, KEGG, MetaNetX…) a model carries in its fields,
//! annotations, notes or parallel per-metabolite vectors.

use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value};

/// Label alternatives of one entity: source key → distinct values, in order.
pub type LabelSources = IndexMap<String, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Metabolite,
    Reaction,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Metabolite => "metabolite",
            EntityKind::Reaction => "reaction",
        }
    }
}

const SOURCE_NAMES: [(&str, &str); 14] = [
    ("base", "Base label"),
    ("id", "Model ID"),
    ("name", "Model name"),
    ("bigg", "BiGG"),
    ("metanetx", "MetaNetX"),
    ("kegg", "KEGG"),
    ("metacyc", "MetaCyc"),
    ("modelseed", "ModelSEED"),
    ("chebi", "ChEBI"),
    ("hmdb", "HMDB"),
    ("pubchem", "PubChem"),
    ("rhea", "Rhea"),
    ("ec", "EC number"),
    ("inchikey", "InChIKey"),
];

/// The display name of a label source key (`kegg` → `KEGG`). An unknown key
/// is shown as it is.
pub fn label_source_name(source: &str) -> String {
    SOURCE_NAMES
        .iter()
        .find(|(k, _)| *k == source)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| source.to_string())
}

static NON_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]").unwrap());
static NAME_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(met|rxn)?names?$").unwrap());
static SEED_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(met|rxn)?seed(compound|reaction|id)?$").unwrap());
static EC_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(rxn)?ec(numbers?|code)?$").unwrap());
static ALIAS_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(met|rxn).*(names?|aliases|ids?)$").unwrap());

/// Which label source a model field feeds, if any.
fn source_key(field: &str, kind: EntityKind) -> Option<String> {
    let key = NON_ALNUM.replace_all(&field.to_lowercase(), "").into_owned();
    if key.contains("pathway") || key.contains("subsystem") {
        return None;
    }
    if NAME_KEY.is_match(&key) {
        return Some("name".into());
    }
    if kind == EntityKind::Metabolite && key.starts_with("rxn") {
        return None;
    }
    if kind == EntityKind::Reaction
        && key.starts_with("met")
        && !key.starts_with("metanetx")
        && !key.starts_with("metacyc")
    {
        return None;
    }
    for source in ["bigg", "metanetx", "kegg", "metacyc", "modelseed", "chebi", "hmdb", "pubchem", "rhea", "inchikey"] {
        if key.contains(source) {
            if key.ends_with("names") || key.ends_with("name") {
                return Some(field.to_string());
            }
            return Some(source.to_string());
        }
    }
    if SEED_KEY.is_match(&key) {
        return Some("modelseed".into());
    }
    if key.contains("biocyc") {
        return Some("metacyc".into());
    }
    if kind == EntityKind::Reaction && EC_KEY.is_match(&key) {
        return Some("ec".into());
    }
    if ALIAS_KEY.is_match(&key) {
        return Some(field.to_string());
    }
    None
}

/// Every distinct non-blank string in a value, arrays flattened to any depth.
fn text_values(value: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            Value::String(s) => {
                let text = s.trim();
                if !text.is_empty() && !out.iter().any(|x| x == text) {
                    out.push(text.to_string());
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out);
    out
}

fn add_source(sources: &mut LabelSources, key: Option<String>, value: &Value) {
    let values = text_values(value);
    let Some(key) = key else { return };
    if values.is_empty() {
        return;
    }
    let existing = sources.entry(key).or_default();
    for v in values {
        if !existing.contains(&v) {
            existing.push(v);
        }
    }
}

/// A parallel per-entity vector of a model (`metKEGGID`, `rxnECNumbers`…).
#[derive(Debug, Clone)]
pub struct LabelVector {
    pub source: String,
    pub values: Vec<Value>,
}

/// The model-level vectors (`met…`/`rxn…` arrays with one entry per entity).
pub fn model_label_vectors(raw: &Map<String, Value>, kind: EntityKind, count: usize) -> Vec<LabelVector> {
    let prefix = if kind == EntityKind::Metabolite { "met" } else { "rxn" };
    let mut vectors = Vec::new();
    for (field, values) in raw {
        let Value::Array(values) = values else { continue };
        if !field.to_lowercase().starts_with(prefix) || values.len() != count {
            continue;
        }
        if let Some(source) = source_key(field, kind) {
            vectors.push(LabelVector { source, values: values.clone() });
        }
    }
    vectors
}

/// The label sources of one metabolite or reaction of a raw model document.
pub fn entity_label_sources(raw: &Map<String, Value>, kind: EntityKind, vectors: &[LabelVector], index: usize) -> LabelSources {
    let mut sources = LabelSources::new();
    if let Some(Value::Object(given)) = raw.get("labelSources") {
        for (key, value) in given {
            add_source(&mut sources, Some(key.clone()), value);
        }
    }
    let blocks: [(Option<&Map<String, Value>>, bool); 4] = [
        (Some(raw), true),
        (raw.get("annotation").and_then(Value::as_object), false),
        (raw.get("annotations").and_then(Value::as_object), false),
        (raw.get("notes").and_then(Value::as_object), false),
    ];
    for (fields, is_raw) in blocks {
        let Some(fields) = fields else { continue };
        for (field, value) in fields {
            if is_raw && field == "name" {
                continue;
            }
            add_source(&mut sources, source_key(field, kind), value);
        }
    }
    for vector in vectors {
        if let Some(v) = vector.values.get(index) {
            add_source(&mut sources, Some(vector.source.clone()), v);
        }
    }
    sources
}

/// Label sources serialised back to JSON (`{source: [values]}`).
pub fn label_sources_to_value(sources: &LabelSources) -> Value {
    Value::Object(
        sources
            .iter()
            .map(|(k, v)| (k.clone(), Value::Array(v.iter().cloned().map(Value::String).collect())))
            .collect(),
    )
}

/// What a drawn object can be labelled with: source → one display string.
pub type ObjectLabels = IndexMap<String, String>;

/// The entity a label is drawn for, as far as labels are concerned.
#[derive(Debug, Clone, Copy, Default)]
pub struct LabelEntity<'a> {
    pub id: Option<&'a str>,
    pub name: Option<&'a str>,
    pub base: Option<&'a str>,
    pub label_sources: Option<&'a LabelSources>,
}

/// The drawn object (a node or a reaction on the map), as far as labels care.
#[derive(Debug, Clone, Copy, Default)]
pub struct LabelObject<'a> {
    pub id: Option<&'a str>,
    pub local_id: Option<&'a str>,
    pub cell_id: Option<&'a str>,
    pub name: Option<&'a str>,
    pub base: Option<&'a str>,
    pub label_sources: Option<&'a LabelSources>,
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|x| !x.is_empty())
}

/// Every label a drawn object can show, by source: `id` (without the cell
/// prefix), `name`, `base` for a metabolite, then each cross-reference
/// source with its distinct values joined by `; `.
pub fn object_label_sources(object: &LabelObject, entity: Option<&LabelEntity>, kind: EntityKind) -> ObjectLabels {
    let mut local_id = object
        .local_id
        .or(entity.and_then(|e| e.id))
        .or(object.id)
        .unwrap_or("")
        .to_string();
    if let Some(cell) = non_empty(object.cell_id) {
        let prefix = format!("{cell}@");
        if let Some(rest) = local_id.strip_prefix(&prefix) {
            local_id = rest.to_string();
        }
    }
    let entity_name = entity.and_then(|e| non_empty(e.name));
    let mut sources = ObjectLabels::new();
    sources.insert("id".into(), local_id.clone());
    sources.insert(
        "name".into(),
        entity_name.or(non_empty(object.name)).map(str::to_string).unwrap_or_else(|| local_id.clone()),
    );
    if kind == EntityKind::Metabolite {
        let base = non_empty(object.base)
            .or(entity.and_then(|e| non_empty(e.base)))
            .unwrap_or(&local_id)
            .to_string();
        let name = entity_name.or(non_empty(object.name)).unwrap_or("");
        sources.insert("base".into(), readable_base(&base, name));
    }
    let alternatives = entity.and_then(|e| e.label_sources).or(object.label_sources);
    if let Some(alts) = alternatives {
        for (source, values) in alts {
            let mut distinct: Vec<&str> = Vec::new();
            for v in values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
                if !distinct.contains(&v) {
                    distinct.push(v);
                }
            }
            if !distinct.is_empty() {
                sources.insert(source.clone(), distinct.join("; "));
            }
        }
    }
    sources
}

static BASE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?)(\[[^\]]*\])?$").unwrap());
static MODELSEED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^cpd[0-9]{5}$").unwrap());
static MODELSEED_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^cpd[0-9]{5}").unwrap());
static SBML_ESCAPED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^M_(.+)_([a-z][a-z0-9]?)$").unwrap());

/// A metabolite's base label, made readable where the model's id is not: a
/// ModelSEED accession (`cpd00023[c_Er]`) gives way to the model name
/// (`L-Glutamate`); an SBML-escaped id (`M_ival_c[c_Ef]`) loses the prefix
/// and the compartment letter the bracket repeats (`ival[c_Ef]`).
pub fn readable_base(base: &str, name: &str) -> String {
    let caps = BASE_TAG.captures(base).expect("pattern matches every string");
    let core = caps.get(1).map_or("", |m| m.as_str());
    let tag = caps.get(2).map_or("", |m| m.as_str());
    if MODELSEED.is_match(core) && !name.is_empty() && !MODELSEED_PREFIX.is_match(name) {
        return name.to_string();
    }
    if let Some(esc) = SBML_ESCAPED.captures(core) {
        let letter = &esc[2];
        if tag.is_empty() || tag.starts_with(&format!("[{letter}")) {
            return format!("{}{}", &esc[1], tag);
        }
    }
    base.to_string()
}

/// The label of a drawn object for one source, or the base/id fallback.
pub fn object_label(object: &LabelObject, entity: Option<&LabelEntity>, kind: EntityKind, source: &str) -> String {
    let sources = object_label_sources(object, entity, kind);
    let fallback_key = if kind == EntityKind::Metabolite { "base" } else { "id" };
    match sources.get(source) {
        Some(v) if !v.is_empty() => v.clone(),
        _ => sources.get(fallback_key).cloned().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn readable_bases() {
        assert_eq!(readable_base("cpd00023[c_Er]", "L-Glutamate"), "L-Glutamate");
        assert_eq!(readable_base("M_ival_c[c_Ef]", ""), "ival[c_Ef]");
        assert_eq!(readable_base("glc__D", "D-Glucose"), "glc__D");
    }

    #[test]
    fn sources_from_fields_annotations_and_vectors() {
        let raw = json!({"id": "glc__D_c", "name": "Glucose", "annotation": {"kegg.compound": ["C00031"], "bigg.metabolite": "glc__D"}, "metNames": "x"});
        let raw = raw.as_object().unwrap();
        let s = entity_label_sources(raw, EntityKind::Metabolite, &[], 0);
        assert_eq!(s["kegg"], ["C00031"]);
        assert_eq!(s["bigg"], ["glc__D"]);
        assert_eq!(s["name"], ["x"]);
    }
}
