//! Port of `src/core/provenance.js`.
//!
//! Metabolic provenance helpers: normalize model annotations and turn
//! identifiers into safe links to authoritative biological databases.

use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

use coxeter_core::js;

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat::{self, DOT, decode_uri_component, encode_uri_component, js_order, object_entries};

/// Namespace → values, in the order they were met (a JS `Map`).
pub type Annotations = IndexMap<String, Vec<String>>;

/// One database the app links to.
struct Db {
    label: &'static str,
    url: fn(&str, &str) -> String,
}

fn enc(v: &str) -> String {
    encode_uri_component(v)
}

fn enc_doi(v: &str) -> String {
    v.split('/').map(enc).collect::<Vec<_>>().join("/")
}

/// `strip(v, p)`: drop a case-insensitive ASCII prefix.
fn strip<'a>(v: &'a str, p: &str) -> &'a str {
    if v.to_lowercase().starts_with(p) { jscompat::utf16_skip(v, p.len()) } else { v }
}

static PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][0-9A-Za-z_.-]*:").expect("pattern"));
static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i-u:META:)").expect("pattern"));

fn strip_prefix(v: &str) -> String {
    PREFIX.replace(v, "").into_owned()
}

fn normalize_chebi(v: &str) -> String {
    format!("CHEBI:{}", strip_prefix(v))
}

fn db(namespace: &str) -> Option<Db> {
    let d = |label, url| Some(Db { label, url });
    match namespace {
        "bigg" => d("BiGG", |id, t| {
            let kind = if t == "reaction" { "reactions" } else { "metabolites" };
            format!("https://bigg.bio/universal/{kind}/{}", enc(strip(id, "bigg:")))
        }),
        "bigg.reaction" => d("BiGG Reaction", |id, _| {
            format!("https://bigg.bio/universal/reactions/{}", enc(strip(id, "bigg.reaction:")))
        }),
        "bigg.metabolite" => d("BiGG Metabolite", |id, _| {
            format!("https://bigg.bio/universal/metabolites/{}", enc(strip(id, "bigg.metabolite:")))
        }),
        "kegg" => d("KEGG", |id, _| format!("https://www.kegg.jp/entry/{}", enc(&strip_prefix(id)))),
        "kegg.reaction" => d("KEGG Reaction", |id, _| format!("https://www.kegg.jp/entry/{}", enc(&strip_prefix(id)))),
        "kegg.compound" => d("KEGG Compound", |id, _| format!("https://www.kegg.jp/entry/{}", enc(&strip_prefix(id)))),
        "metanetx" | "metanetx.chemical" => {
            d("MetaNetX", |id, _| format!("https://www.metanetx.org/chem_info/{}", enc(&strip_prefix(id))))
        }
        "metanetx.reaction" => d("MetaNetX", |id, _| format!("https://www.metanetx.org/equa_info/{}", enc(&strip_prefix(id)))),
        "chebi" => d("ChEBI", |id, _| format!("https://www.ebi.ac.uk/chebi/searchId.do?chebiId={}", enc(&normalize_chebi(id)))),
        "pubchem" => d("PubChem", |id, _| format!("https://pubchem.ncbi.nlm.nih.gov/compound/{}", enc(&strip_prefix(id)))),
        "hmdb" => d("HMDB", |id, _| format!("https://www.hmdb.ca/metabolites/{}", enc(&strip_prefix(id)))),
        "rhea" => d("Rhea", |id, _| format!("https://www.rhea-db.org/rhea/{}", enc(&strip_prefix(id)))),
        "ec" | "ec-code" => d("EC", |id, _| format!("https://enzyme.expasy.org/EC/{}", enc(&strip_prefix(id)))),
        "uniprot" => d("UniProt", |id, _| format!("https://www.uniprot.org/uniprotkb/{}/entry", enc(&strip_prefix(id)))),
        "ncbigene" => d("NCBI Gene", |id, _| format!("https://www.ncbi.nlm.nih.gov/gene/{}", enc(&strip_prefix(id)))),
        "refseq" => d("NCBI RefSeq", |id, _| format!("https://www.ncbi.nlm.nih.gov/search/all/?term={}", enc(&strip_prefix(id)))),
        "pubmed" => d("PubMed", |id, _| format!("https://pubmed.ncbi.nlm.nih.gov/{}/", enc(&strip_prefix(id)))),
        "doi" => d("DOI", |id, _| format!("https://doi.org/{}", enc_doi(strip(id, "doi:")))),
        "biocyc" => d("BioCyc", |id, _| {
            format!("https://biocyc.org/gene?orgid=META&id={}", enc(&META.replace(&strip_prefix(id), "")))
        }),
        "reactome" => d("Reactome", |id, _| format!("https://reactome.org/content/detail/{}", enc(&strip_prefix(id)))),
        _ => None,
    }
}

static IDENTIFIERS_ORG_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://identifiers\.org/").expect("pattern"));
static KEGG_GENES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^kegg\.genes?$").expect("pattern"));
static EC_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ec\.number$").expect("pattern"));
static INCHIKEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^inchi{DOT}?key$")).expect("pattern"));

/// `normalizeNamespace`: `KEGG_Compound` → `kegg.compound`, `EC_number` → `ec`.
pub fn normalize_namespace(x: &str) -> String {
    let s = jscompat::js_trim(x).to_lowercase();
    let s = IDENTIFIERS_ORG_PREFIX.replace(&s, "").replace('_', ".");
    let s = KEGG_GENES.replace(&s, "kegg");
    let s = EC_NUMBER.replace(&s, "ec");
    INCHIKEY.replace(&s, "inchikey").into_owned()
}

static SPLITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("[;,]{}*", *jscompat::S)).expect("pattern"));

/// `splitValue`: arrays flattened, `a; b, c` split into its parts.
fn split_value(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Array(items) => items.iter().for_each(|x| split_value(x, out)),
        Value::String(s) if SPLITS.is_match(s) => {
            out.extend(SPLITS.split(s).filter(|p| !p.is_empty()).map(|p| Value::String(p.to_string())));
        }
        other => out.push(other.clone()),
    }
}

/// `String(v)` for a JSON value, as `normalizeAnnotations` stores it.
fn value_text(v: &Value) -> String {
    js::to_string(v)
}

/// Merge a model's annotation and notes into namespace → identifiers. Notes
/// only add namespaces the annotation does not already have.
pub fn normalize_annotations(annotation: &Value, notes: &Value) -> Annotations {
    let mut out = Annotations::new();
    let add = |out: &mut Annotations, key: &str, value: &Value| {
        if value.is_null() || value.as_str() == Some("") {
            return;
        }
        let k = normalize_namespace(key);
        let vals: Vec<Value> = match value {
            Value::Array(items) => items.clone(),
            other => vec![other.clone()],
        };
        let list = out.entry(k).or_default();
        let mut flat = Vec::new();
        vals.iter().for_each(|v| split_value(v, &mut flat));
        for v in flat {
            if v.as_str() == Some("") {
                continue;
            }
            let text = value_text(&v);
            if !list.contains(&text) {
                list.push(text);
            }
        }
    };
    let annotation = if js::truthy(annotation) { annotation } else { &Value::Null };
    for (k, v) in object_entries(annotation) {
        add(&mut out, &k, &v);
    }
    let notes = if js::truthy(notes) { notes } else { &Value::Null };
    for (k, v) in object_entries(notes) {
        if !out.contains_key(&normalize_namespace(&k)) {
            add(&mut out, &k, &v);
        }
    }
    out
}

/// One row of the inspector's provenance table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnnotationLink {
    pub namespace: String,
    pub label: String,
    pub value: String,
    pub url: Option<String>,
}

static CHEBI_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i-u:CHEBI:)").expect("pattern"));
static MNX_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i-u:MNX[MR])").expect("pattern"));
static RHEA_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i-u:RHEA:)").expect("pattern"));
static EC_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+\.").expect("pattern"));
static KEGG_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[CRG][0-9]{5}$").expect("pattern"));

fn infer_db(ns: &str, value: &str, kind: &str) -> Option<Db> {
    let n = normalize_namespace(ns);
    if let Some(d) = db(&n) {
        return Some(d);
    }
    if CHEBI_VALUE.is_match(value) {
        return db("chebi");
    }
    if MNX_VALUE.is_match(value) {
        return db("metanetx");
    }
    if RHEA_VALUE.is_match(value) {
        return db("rhea");
    }
    if EC_VALUE.is_match(value) {
        return db("ec");
    }
    if kind != "gene" && KEGG_VALUE.is_match(value) {
        return db("kegg");
    }
    None
}

/// Every identifier of an entity with its database and a link where one is
/// known. `kind` is `metabolite`, `reaction` or `gene`.
pub fn annotation_links(annotation: &Value, notes: &Value, kind: &str) -> Vec<AnnotationLink> {
    let mut rows = Vec::new();
    for (namespace, values) in normalize_annotations(annotation, notes) {
        for value in values {
            let found = db(&namespace).or_else(|| infer_db(&namespace, &value, kind));
            rows.push(AnnotationLink {
                label: found.as_ref().map_or_else(|| namespace.clone(), |d| d.label.to_string()),
                url: found.as_ref().map(|d| (d.url)(&value, kind)),
                namespace: namespace.clone(),
                value,
            });
        }
    }
    rows
}

static IDENTIFIERS_ORG: LazyLock<Regex> = LazyLock::new(|| {
    let not = format!(r#"[^"'<>{}]"#, jscompat::WS_CHARS);
    Regex::new(&format!(r"(?i-u:https?://(?:identifiers\.org|registry\.identifiers\.org)/)({not}+){not}*")).expect("pattern")
});

/// Pull every `identifiers.org` resource out of an annotation block:
/// `https://identifiers.org/kegg.reaction/R00010` → `{"kegg.reaction": ["R00010"]}`.
/// Fails as `decodeURIComponent` does on a malformed `%` escape.
pub fn extract_identifiers_org(body: &str) -> Result<Map<String, Value>> {
    let mut out: IndexMap<String, Vec<String>> = IndexMap::new();
    for m in IDENTIFIERS_ORG.captures_iter(body) {
        let decoded = decode_uri_component(&m[1]).ok_or_else(|| Error::new(ErrorKind::UriMalformed, "URI malformed"))?;
        let path = decoded.trim_matches('/');
        let (ns, id) = match path.split_once(':') {
            Some((ns, id)) => (ns.to_string(), id.to_string()),
            None => match path.split_once('/') {
                Some((ns, rest)) => (ns.to_string(), rest.to_string()),
                None => (path.to_string(), String::new()),
            },
        };
        if !ns.is_empty() && !id.is_empty() {
            out.entry(normalize_namespace(&ns)).or_default().push(id);
        }
    }
    let mut map = Map::new();
    for (k, ids) in out {
        let mut distinct: Vec<String> = Vec::new();
        for id in ids {
            if !distinct.contains(&id) {
                distinct.push(id);
            }
        }
        map.insert(k, Value::Array(distinct.into_iter().map(Value::String).collect()));
    }
    Ok(js_order(map))
}

/// `annotationObject(raw)`: an object (or array) as is, anything else `{}`.
pub fn annotation_object(raw: &Value) -> Value {
    match raw {
        Value::Object(_) | Value::Array(_) => raw.clone(),
        _ => Value::Object(Map::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // From test/provenance.test.mjs.
    #[test]
    fn database_links_cover_reaction_metabolite_and_gene_namespaces() {
        assert!(annotation_links(&json!({"chebi": "CHEBI:15377"}), &json!({}), "metabolite")[0].url.as_ref().unwrap().contains("ebi.ac.uk"));
        assert!(annotation_links(&json!({"rhea": "RHEA:10000"}), &json!({}), "reaction")[0].url.as_ref().unwrap().contains("rhea-db.org"));
        assert!(annotation_links(&json!({"uniprot": "P12345"}), &json!({}), "gene")[0].url.as_ref().unwrap().contains("uniprot.org"));
    }

    #[test]
    fn identifiers_org_rdf_resources_are_normalized() {
        let a = extract_identifiers_org(
            r#"<rdf:li rdf:resource="https://identifiers.org/kegg.reaction/R00010"/><rdf:li rdf:resource="https://identifiers.org/chebi:15377"/>"#,
        )
        .unwrap();
        assert_eq!(a["kegg.reaction"], json!(["R00010"]));
        assert_eq!(a["chebi"], json!(["15377"]));
    }

    #[test]
    fn namespaces_and_splitting() {
        assert_eq!(normalize_namespace(" KEGG_Genes "), "kegg");
        assert_eq!(normalize_namespace("EC_Number"), "ec");
        assert_eq!(normalize_namespace("InChI_Key"), "inchikey");
        let n = normalize_annotations(&json!({"kegg.compound": "C00001; C00002,C00003", "x": ["a", ["b", null]]}), &json!({"KEGG.compound": "nope", "pmid": 12}));
        assert_eq!(n["kegg.compound"], ["C00001", "C00002", "C00003"]);
        assert_eq!(n["x"], ["a", "b", "null"]);
        assert_eq!(n["pmid"], ["12"]);
        assert_eq!(extract_identifiers_org("https://identifiers.org/a%ZZ").unwrap_err().message, "URI malformed");
    }
}
