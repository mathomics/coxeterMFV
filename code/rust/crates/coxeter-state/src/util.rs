//! The small helper functions at the bottom of `store.js`.

use coxeter_core::js;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

/// A number as `JSON.stringify` writes it: `null` when it is not finite.
pub fn num_json(v: f64) -> Value {
    if v.is_finite() { serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number) } else { Value::Null }
}

/// `clampNumber(value, min, max, fallback)`.
pub fn clamp_number(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if !value.is_finite() {
        return fallback;
    }
    js::min(max, js::max(min, value))
}

/// `normalizePfbaPercent(value)`.
pub fn normalize_pfba_percent(value: f64) -> f64 {
    coxeter_solver::fba::normalize_pfba_percent(value)
}

/// `normalizeSearch(value)`: accents off, trimmed, lower case.
pub fn normalize_search(value: &str) -> String {
    let stripped: String = value.nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect();
    js_trim(&stripped).to_lowercase()
}

/// `String.prototype.trim`: JavaScript white space and line terminators.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `uniquePathwayId(name, pathways)`: a slug of the name, numbered if taken.
pub fn unique_pathway_id(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let lowered = name.to_lowercase();
    let stripped: String = lowered.nfkd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect();
    let mut slug = String::new();
    let mut dash = false;
    for c in stripped.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
            dash = false;
        } else if !dash {
            slug.push('-');
            dash = true;
        }
    }
    let base = slug.trim_matches('-');
    let base = if base.is_empty() { "pathway".to_string() } else { base.to_string() };
    let mut id = base.clone();
    let mut index = 2;
    while taken(&id) {
        id = format!("{base}-{index}");
        index += 1;
    }
    id
}

/// `describeStatus(status)`.
pub fn describe_status(status: &str) -> String {
    match status {
        "infeasible" => "No solution: the constraints are incompatible.".into(),
        "unbounded" => "Unbounded objective: a constraint is missing.".into(),
        "iteration_limit" => "The iteration limit was reached.".into(),
        other => other.into(),
    }
}

/// `formatDuration(ms)`: `850 ms`, `4.2 s`, `2 min 05 s`.
pub fn format_duration(ms: f64) -> String {
    if !ms.is_finite() || ms < 0.0 {
        return "—".into();
    }
    if ms < 1000.0 {
        return format!("{} ms", js::number_to_string(js::round(ms)));
    }
    if ms < 60000.0 {
        return format!("{} s", js::to_fixed(ms / 1000.0, 1));
    }
    let s = js::round(ms / 1000.0) as i64;
    format!("{} min {:02} s", s / 60, s % 60)
}

/// `csv(s)`: a quoted CSV cell.
pub fn csv(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// `x.toExponential(digits)`.
pub fn to_exponential(x: f64, digits: usize) -> String {
    if !x.is_finite() {
        return js::number_to_string(x);
    }
    let s = format!("{x:.digits$e}");
    match s.split_once('e') {
        Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
        _ => s,
    }
}

/// `Math.round(x)` as the integer JS prints.
pub fn round_ms(x: f64) -> f64 {
    js::round(x)
}

/// `String(x)` of a JSON value, `''` for null/absent.
pub fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js::to_string(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_match_the_javascript() {
        assert_eq!(normalize_search("  Glucosa-6-fosfato Isomerása "), "glucosa-6-fosfato isomerasa");
        assert_eq!(unique_pathway_id("Glicólisis superior!", |_| false), "glicolisis-superior");
        assert_eq!(unique_pathway_id("***", |_| false), "pathway");
        assert_eq!(unique_pathway_id("A", |id| id == "a" || id == "a-2"), "a-3");
        assert_eq!(format_duration(850.4), "850 ms");
        assert_eq!(format_duration(4210.0), "4.2 s");
        assert_eq!(format_duration(125_000.0), "2 min 05 s");
        assert_eq!(format_duration(f64::NAN), "—");
        assert_eq!(csv("a\"b"), "\"a\"\"b\"");
        assert_eq!(to_exponential(0.0, 2), "0.00e+0");
        assert_eq!(to_exponential(1.234e-7, 2), "1.23e-7");
        assert_eq!(to_exponential(12345.0, 2), "1.23e+4");
        assert_eq!(clamp_number(f64::NAN, 1.0, 2.0, 1.5), 1.5);
        assert_eq!(clamp_number(5.0, 1.0, 2.0, 1.5), 2.0);
        assert_eq!(describe_status("infeasible"), "No solution: the constraints are incompatible.");
    }
}
