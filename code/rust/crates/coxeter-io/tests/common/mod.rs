//! Shared by the parity tests: how the fixture script describes things, said
//! again from the Rust side.
#![allow(dead_code)]

use std::path::PathBuf;

use coxeter_core::model::Model;
use coxeter_core::testkit::{assert_json_eq, fixture_path, repo_root};
use coxeter_io::mat5::{MatVariables, variables_to_json};
use coxeter_io::model_import::PreparedImport;
use coxeter_io::provenance::annotation_links;
use coxeter_io::sbml::SbmlReadOptions;
use coxeter_io::xlsx::Workbook;
use serde_json::{Map, Value, json};

/// `JSON.parse(JSON.stringify(model.toJSON()))`.
pub fn model_json(m: &Model) -> Value {
    m.to_json()
}

pub fn describe_prepared(p: &PreparedImport) -> Value {
    json!({
        "entries": p.entries.iter().map(|e| json!({
            "filename": e.filename,
            "modelKey": e.model_key,
            "speciesId": e.species_id,
            "abundance": e.abundance,
            "enabled": e.enabled,
            "summary": {
                "reactions": e.summary.reactions,
                "metabolites": e.summary.metabolites,
                "compartments": e.summary.compartments,
                "biomass": e.summary.biomass,
            },
            "model": model_json(&e.model),
        })).collect::<Vec<_>>(),
        "medium": p.medium,
    })
}

/// `{ok: value}` or `{error: message}`, as the fixture script's `attempt` writes.
pub fn attempt<T>(r: coxeter_io::Result<T>, describe: impl FnOnce(T) -> Value) -> Value {
    match r {
        Ok(v) => json!({"ok": describe(v)}),
        Err(e) => json!({"error": e.message}),
    }
}

/// Messages that quote a JavaScript engine error (JSON.parse, a TypeError)
/// cannot match word for word; the part the app writes itself must.
pub fn same_error(rust: &str, js: &str) -> bool {
    for marker in ["invalid JSON (", "Invalid stoichiometry JSON: "] {
        if let Some(i) = js.find(marker) {
            return rust.starts_with(&js[..i + marker.len()]);
        }
    }
    rust == js
}

/// Compare an `attempt` result with the fixture's.
#[track_caller]
pub fn assert_attempt(actual: &Value, expected: &Value, what: &str) {
    match (actual.get("error"), expected.get("error")) {
        (Some(a), Some(e)) => {
            let (a, e) = (a.as_str().unwrap(), e.as_str().unwrap());
            assert!(same_error(a, e), "{what}: error {a:?}, the JS says {e:?}");
        }
        (None, None) => assert_json_eq(&actual["ok"], &expected["ok"], 0.0, what),
        _ => panic!("{what}: Rust gave {}, the JS gave {}", short(actual), short(expected)),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 300 { format!("{}…", &s[..s.char_indices().nth(300).map_or(s.len(), |(i, _)| i)]) } else { s }
}

pub fn read_options(opts: &Value) -> SbmlReadOptions {
    let mut o = SbmlReadOptions::default();
    if let Some(v) = opts.get("normaliseCompartment").and_then(Value::as_bool) {
        o.normalise_compartment = v;
    }
    if let Some(v) = opts.get("sentinelBound").and_then(Value::as_f64) {
        o.sentinel_bound = v;
    }
    if let Some(v) = opts.get("defaultBound").and_then(Value::as_f64) {
        o.default_bound = v;
    }
    if let Some(v) = opts.get("id").and_then(Value::as_str) {
        o.id = Some(v.to_string());
    }
    o
}

pub fn links_of(model: &Model) -> Value {
    let links = |a: &Value, n: &Value, kind: &str| serde_json::to_value(annotation_links(a, n, kind)).unwrap();
    let empty = json!({});
    json!({
        "metabolites": model.metabolites.values().map(|m| links(&m.annotation, &m.notes, "metabolite")).collect::<Vec<_>>(),
        "reactions": model.reactions.values().map(|r| links(&r.annotation, &r.notes, "reaction")).collect::<Vec<_>>(),
        "genes": model.genes.iter().map(|g| links(g.get("annotation").unwrap_or(&empty), g.get("notes").unwrap_or(&empty), "gene")).collect::<Vec<_>>(),
    })
}

pub fn describe_workbook(bytes: Vec<u8>) -> Value {
    let wb: Workbook = match coxeter_io::read_workbook(bytes) {
        Ok(wb) => wb,
        Err(e) => return json!({"error": e.message}),
    };
    let mut sheets = Map::new();
    let mut names = wb.sheet_names();
    names.push("nope".into());
    for name in &names {
        let rows = wb.rows(name).and_then(|rows| rows.map(|r| r.map(|row| row.to_json())).collect::<Result<Vec<_>, _>>());
        sheets.insert(name.clone(), attempt(rows, Value::from));
    }
    json!({"sheetNames": wb.sheet_names(), "sheets": sheets})
}

pub fn describe_mat(r: coxeter_io::Result<MatVariables>) -> Value {
    match r {
        Ok(vars) => json!({"variables": variables_to_json(&vars)}),
        Err(e) => json!({"error": e.message}),
    }
}

/// A path in the repository.
pub fn repo(rel: &str) -> PathBuf {
    repo_root().join(rel)
}

pub fn fixture_text(rel: &str) -> String {
    std::fs::read_to_string(fixture_path(rel)).unwrap_or_else(|e| panic!("{rel}: {e} (run: node rust/tools/fixtures-io.mjs)"))
}

pub fn fixture_bytes(rel: &str) -> Vec<u8> {
    std::fs::read(fixture_path(rel)).unwrap_or_else(|e| panic!("{rel}: {e} (run: node rust/tools/fixtures-io.mjs)"))
}

/// Where two texts first differ, for a readable failure on a large document.
#[track_caller]
pub fn assert_same_text(actual: &str, expected: &str, what: &str) {
    if actual == expected {
        return;
    }
    let at = actual.bytes().zip(expected.bytes()).position(|(a, b)| a != b).unwrap_or(actual.len().min(expected.len()));
    let line = expected[..at.min(expected.len())].matches('\n').count() + 1;
    let around = |s: &str| {
        let start = s[..at.min(s.len())].rfind('\n').map_or(0, |i| i + 1);
        let end = s[at.min(s.len())..].find('\n').map_or(s.len(), |i| at + i);
        s[start..end.min(s.len())].to_string()
    };
    panic!(
        "{what} differs from the JavaScript output at line {line} (lengths {} and {}):\n  rust: {}\n  js:   {}",
        actual.len(),
        expected.len(),
        around(actual),
        around(expected)
    );
}
