//! Comparing what the Rust port produces with what the JavaScript app wrote.
//!
//! Parity fixtures are JSON written by `rust/tools/*.mjs` from the JS modules.
//! JSON written by JavaScript prints `1` where serde writes `1.0`, so values
//! are compared as JavaScript would read them: every number as a double.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The repository root (the folder that holds `src/`, `models/` and `rust/`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repo root")
}

/// `rust/fixtures/<rel>`.
pub fn fixture_path(rel: &str) -> PathBuf {
    repo_root().join("rust/fixtures").join(rel)
}

/// Read a JSON fixture, or `None` when it has not been generated (large ones are
/// not committed and are skipped when absent).
pub fn read_fixture(rel: &str) -> Option<Value> {
    let path = fixture_path(rel);
    let text = std::fs::read_to_string(&path).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// Where two JSON values first differ, or `None` when they are the same as
/// JavaScript sees them. Numbers within `tol` (relative to their size, with
/// an absolute floor of `tol`) are equal, and `tol = 0.0` asks for identity.
pub fn json_diff(a: &Value, b: &Value, tol: f64) -> Option<String> {
    fn walk(a: &Value, b: &Value, tol: f64, path: &mut String) -> Option<String> {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
                let same = if tol == 0.0 { x == y } else { (x - y).abs() <= tol * x.abs().max(y.abs()).max(1.0) };
                (!same).then(|| format!("{path}: {x} != {y}"))
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    return Some(format!("{path}: array length {} != {}", x.len(), y.len()));
                }
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    if let Some(d) = walk(p, q, tol, path) {
                        return Some(d);
                    }
                    path.truncate(len);
                }
                None
            }
            (Value::Object(x), Value::Object(y)) => {
                for k in x.keys() {
                    if !y.contains_key(k) {
                        return Some(format!("{path}.{k}: only on the left ({})", short(&x[k])));
                    }
                }
                for k in y.keys() {
                    if !x.contains_key(k) {
                        return Some(format!("{path}.{k}: only on the right ({})", short(&y[k])));
                    }
                }
                for (k, p) in x {
                    let len = path.len();
                    path.push('.');
                    path.push_str(k);
                    if let Some(d) = walk(p, &y[k], tol, path) {
                        return Some(d);
                    }
                    path.truncate(len);
                }
                None
            }
            _ => (a != b).then(|| format!("{path}: {} != {}", short(a), short(b))),
        }
    }
    walk(a, b, tol, &mut String::from("$"))
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 120 { format!("{}…", &s[..s.char_indices().nth(117).map_or(s.len(), |(i, _)| i)]) } else { s }
}

/// Panic with the path of the first difference.
#[track_caller]
pub fn assert_json_eq(actual: &Value, expected: &Value, tol: f64, what: &str) {
    if let Some(d) = json_diff(actual, expected, tol) {
        panic!("{what} differs from the JavaScript output at {d}");
    }
}
