//! The few places where JavaScript and Rust disagree about numbers and text.
//!
//! Coxeter was written in JavaScript, and its maps, exports and layouts are
//! defined by what that code does. Where the two languages give a different
//! answer to the same question (how to round 2.5, how to print 1e-7, which of
//! `Ba` and `ac` comes first), every crate asks this module, so a map drawn by
//! the Rust build is the map the JavaScript build draws.

use std::cmp::Ordering;
use std::sync::LazyLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use serde_json::Value;

/// `Math.round`: halves go up, towards +∞ (`-2.5` → `-2`), unlike `f64::round`.
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = x.floor();
    // `x - f` is exact for every double, so a true half is seen as one.
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    // Math.round(-0.4) is -0, which prints as "0" anyway. Keep the sign rule simple.
    if r == 0.0 && x < 0.0 { -0.0 } else { r }
}

/// `Math.round(v * 10^digits) / 10^digits`, the rounding helper the JS code uses.
pub fn round_to(v: f64, digits: i32) -> f64 {
    let f = 10f64.powi(digits);
    round(v * f) / f
}

/// `Number.prototype.toString()`: the shortest digits that read back as `x`,
/// in plain notation from 1e-6 up to 1e21 and in exponent notation outside it.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // `{:e}` prints the shortest round-trip digits as d.ddde±n.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').expect("LowerExp always has an exponent");
    let exp: i32 = exp.parse().expect("exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp + 1; // position of the decimal point relative to the digits
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let e_sign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{e_sign}{}", e.abs())
        } else {
            format!("{}.{}e{e_sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    };
    format!("{sign}{body}")
}

/// `Number.prototype.toFixed(digits)`: the nearest value with that many
/// decimals, taking the larger one on an exact tie (`0.125.toFixed(2)` is
/// `"0.13"`, where Rust's `{:.2}` would print `"0.12"`).
pub fn to_fixed(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.abs() >= 1e21 || x.is_infinite() {
        return number_to_string(x);
    }
    let negative = x < 0.0;
    let ax = x.abs();
    // The exact decimal expansion of a double ends within 1074 places; 40 extra
    // places are enough to tell a true tie from a near one at this magnitude.
    let exact = format!("{:.*}", digits + 40, ax);
    let (int_part, frac_part) = exact.split_once('.').unwrap_or((&exact, ""));
    let kept = &frac_part[..digits];
    let rest = &frac_part[digits..];
    let round_up = match rest.as_bytes().first() {
        Some(b) if *b > b'5' => true,
        Some(b'5') => true, // a tie or above: JS takes the larger value
        _ => false,
    };
    let mut number: Vec<u8> = format!("{int_part}{kept}").into_bytes();
    if round_up {
        let mut i = number.len();
        loop {
            if i == 0 {
                number.insert(0, b'1');
                break;
            }
            i -= 1;
            if number[i] == b'9' {
                number[i] = b'0';
            } else {
                number[i] += 1;
                break;
            }
        }
    }
    let s = String::from_utf8(number).expect("ascii digits");
    let split = s.len() - digits;
    let body = if digits == 0 { s } else { format!("{}.{}", &s[..split], &s[split..]) };
    if negative { format!("-{body}") } else { body }
}

/// JS `ToNumber` for a JSON value: `Number(v)`.
pub fn to_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => {
            if *b { 1.0 } else { 0.0 }
        }
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => string_to_number(s),
        Value::Array(a) => match a.len() {
            0 => 0.0,
            1 => to_number(&a[0]),
            _ => f64::NAN,
        },
        Value::Object(_) => f64::NAN,
    }
}

/// `Number("…")`: blank is 0, otherwise the whole trimmed text must be a number.
pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let lower = t.to_ascii_lowercase();
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return u64::from_str_radix(rest, radix).map(|n| n as f64).unwrap_or(f64::NAN);
        }
    }
    // Rust accepts "inf", "nan" and "1e5" alike, JS only the last.
    if lower.contains("inf") || lower.contains("nan") {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number(v)` when it is finite, as the JS `numOr` helpers test it.
pub fn finite_number(v: &Value) -> Option<f64> {
    let n = to_number(v);
    n.is_finite().then_some(n)
}

/// JavaScript truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `String(v)` for a JSON value, as template literals and `String()` print it.
pub fn to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(|x| if x.is_null() { String::new() } else { to_string(x) }).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `value || fallback` for a string field: a non-empty string, or the fallback.
pub fn str_or<'a>(v: Option<&'a Value>, fallback: &'a str) -> &'a str {
    match v {
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => fallback,
    }
}

static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(Default::default(), CollatorOptions::default())
        .expect("compiled collation data for the root locale")
});

/// `a.localeCompare(b)`: ICU's root collation, which is what a browser or Node
/// without a locale argument applies. Case is a tertiary difference, so `ac`
/// sorts before `Ba`. Byte order would put `Ba` first.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    COLLATOR.compare(a, b)
}

/// `Array.prototype.sort()` with no comparator: UTF-16 code-unit order.
pub fn code_unit_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Is this string an array index (`"0"`, `"42"`)? JS objects list such keys
/// first, in numeric order, before every other key in insertion order.
pub fn is_array_index(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().all(|b| b.is_ascii_digit())
        && (key == "0" || !key.starts_with('0'))
        && key.parse::<u64>().is_ok_and(|n| n < u32::MAX as u64)
}

/// Reorder an object's keys the way `Object.keys` / `Object.entries` would
/// list them. Only needed where integer-like keys can occur.
pub fn js_key_order<V>(map: &mut indexmap::IndexMap<String, V>) {
    if !map.keys().any(|k| is_array_index(k)) {
        return;
    }
    let mut entries: Vec<(String, V)> = map.drain(..).collect();
    // Stable: string keys keep insertion order, and index keys go first by value.
    entries.sort_by(|(a, _), (b, _)| match (is_array_index(a), is_array_index(b)) {
        (true, true) => a.parse::<u64>().unwrap().cmp(&b.parse::<u64>().unwrap()),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => Ordering::Equal,
    });
    map.extend(entries);
}

/// The same reordering for a JSON object: integer-like keys first, by value.
pub fn js_object_key_order(map: &mut serde_json::Map<String, Value>) {
    if !map.keys().any(|k| is_array_index(k)) {
        return;
    }
    let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
    entries.sort_by(|(a, _), (b, _)| match (is_array_index(a), is_array_index(b)) {
        (true, true) => a.parse::<u64>().unwrap().cmp(&b.parse::<u64>().unwrap()),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => Ordering::Equal,
    });
    map.extend(entries);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_round_goes_up_on_halves() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert_eq!(round(-0.5), -0.0);
        assert_eq!(round(1.4999), 1.0);
    }

    #[test]
    fn numbers_print_like_javascript() {
        assert_eq!(number_to_string(5.0), "5");
        assert_eq!(number_to_string(-0.0), "0");
        assert_eq!(number_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(number_to_string(1e-7), "1e-7");
        assert_eq!(number_to_string(1.5e-7), "1.5e-7");
        assert_eq!(number_to_string(0.000001), "0.000001");
        assert_eq!(number_to_string(1e21), "1e+21");
        assert_eq!(number_to_string(123456789012345680000.0), "123456789012345680000");
        assert_eq!(number_to_string(-1234.5), "-1234.5");
        assert_eq!(number_to_string(f64::INFINITY), "Infinity");
    }

    #[test]
    fn to_fixed_rounds_ties_up() {
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(1.005, 2), "1.00"); // 1.005 is really 1.00499…
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(-2.5, 0), "-3");
        assert_eq!(to_fixed(9.995, 2), "9.99");
        assert_eq!(to_fixed(99.995, 2), "100.00");
        assert_eq!(to_fixed(0.0, 3), "0.000");
        assert_eq!(to_fixed(-0.001, 2), "-0.00");
        assert_eq!(to_fixed(1234.5678, 1), "1234.6");
    }

    #[test]
    fn to_number_follows_js() {
        assert_eq!(to_number(&Value::from("")), 0.0);
        assert_eq!(to_number(&Value::from(" 12 ")), 12.0);
        assert!(to_number(&Value::from("12px")).is_nan());
        assert!(to_number(&Value::from("inf")).is_nan());
        assert_eq!(to_number(&Value::from("-Infinity")), f64::NEG_INFINITY);
        assert_eq!(to_number(&Value::Bool(true)), 1.0);
        assert_eq!(to_number(&Value::from("1e3")), 1000.0);
    }

    #[test]
    fn locale_compare_is_case_insensitive_first() {
        assert_eq!(locale_compare("ac", "Ba"), Ordering::Less);
        assert_eq!(locale_compare("a", "A"), Ordering::Less);
        assert_eq!(locale_compare("c", "m"), Ordering::Less);
        assert_eq!(locale_compare("_x", "a"), Ordering::Less);
        assert_eq!(code_unit_compare("Ba", "ac"), Ordering::Less);
        let mut ids = ["glc__D_e", "glc_D_e", "Glc", "g6p", "G6P", "10fthf", "2pg", "h2o", "H2O"];
        ids.sort_by(|a, b| locale_compare(a, b));
        // What Node prints for the same sort.
        assert_eq!(ids.join(" "), "10fthf 2pg g6p G6P Glc glc__D_e glc_D_e h2o H2O");
    }

    #[test]
    fn integer_keys_come_first() {
        let mut m: indexmap::IndexMap<String, i32> = indexmap::IndexMap::new();
        m.insert("b".into(), 1);
        m.insert("10".into(), 2);
        m.insert("a".into(), 3);
        m.insert("2".into(), 4);
        js_key_order(&mut m);
        assert_eq!(m.keys().cloned().collect::<Vec<_>>(), ["2", "10", "b", "a"]);
    }
}

/// Write whole numbers the way `JSON.stringify` does (`1`, not `1.0`), so a
/// file written by the Rust build reads the same as one written by the JS app.
pub fn normalize_json_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if n.is_f64() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
                    // -0 prints as 0 in JSON.stringify as well.
                    *v = Value::from(f as i64);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_json_numbers),
        Value::Object(map) => map.values_mut().for_each(normalize_json_numbers),
        _ => {}
    }
}

/// `JSON.stringify(value)` / `JSON.stringify(value, null, indent)`.
pub fn stringify(value: &Value, indent: Option<usize>) -> String {
    let mut v = value.clone();
    normalize_json_numbers(&mut v);
    match indent {
        None => serde_json::to_string(&v).expect("JSON values always serialise"),
        Some(n) => {
            let pad = vec![b' '; n];
            let fmt = serde_json::ser::PrettyFormatter::with_indent(&pad);
            let mut out = Vec::new();
            let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
            serde::Serialize::serialize(&v, &mut ser).expect("JSON values always serialise");
            String::from_utf8(out).expect("JSON is UTF-8")
        }
    }
}

// Math.* functions
//
// V8 computes sin, cos, atan2, exp, log… with fdlibm. Rust's `f64` methods
// call the platform libm, which can differ in the last bit. Over hundreds of
// layout iterations one bit becomes a visible difference. The `libm` crate is
// musl's port of the same fdlibm algorithms, so these agree with V8.

/// `Math.hypot(a, b)`, as V8 computes it (scaled, Kahan-summed).
pub fn hypot(a: f64, b: f64) -> f64 {
    let (x, y) = (a.abs(), b.abs());
    if x == f64::INFINITY || y == f64::INFINITY {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let max = if x > y { x } else { y };
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for v in [x, y] {
        let n = v / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// `Math.hypot(...values)` for any number of arguments.
pub fn hypot_n(values: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for v in values {
        let a = v.abs();
        if a.is_nan() {
            nan = true;
        } else if a > max {
            max = a;
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut compensation) = (0.0, 0.0);
    for v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}
pub fn log(x: f64) -> f64 {
    libm::log(x)
}
pub fn log10(x: f64) -> f64 {
    libm::log10(x)
}
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}
pub fn pow(x: f64, y: f64) -> f64 {
    // `Math.pow` spec cases libm answers differently.
    if y.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        return 1.0;
    }
    if (x == 1.0 || x == -1.0) && y.is_infinite() {
        return f64::NAN;
    }
    libm::pow(x, y)
}

/// `Math.min(a, b)`: NaN wins, and -0 is less than +0.
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

/// `Math.max(a, b)`: NaN wins, and +0 is greater than -0.
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { b } else { a };
    }
    if a > b { a } else { b }
}

/// `Math.min(...values)`, `Infinity` for none.
pub fn min_n(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(f64::INFINITY, min)
}

/// `Math.max(...values)`, `-Infinity` for none.
pub fn max_n(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(f64::NEG_INFINITY, max)
}

/// `Math.sign(x)`.
pub fn sign(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 { x } else if x > 0.0 { 1.0 } else { -1.0 }
}

/// `a % b` (sign of the dividend), as `f64 %` already is.
pub fn rem(a: f64, b: f64) -> f64 {
    a % b
}

#[cfg(test)]
mod math_tests {
    use super::*;

    #[test]
    fn hypot_matches_v8_on_awkward_values() {
        // Values printed by `node -e "console.log(Math.hypot(a, b))"`.
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, 0.0), 0.0);
        assert_eq!(hypot(-1e-200, 1e-200), 1.414213562373095e-200);
        assert_eq!(hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
        assert_eq!(hypot(1.1, 2.2), 2.459674775249769);
        assert_eq!(hypot(123.456, 0.001), 123.45600000405001);
        assert_eq!(hypot(1e308, 1e308), 1.4142135623730951e308);
    }

    #[test]
    fn min_max_propagate_nan() {
        assert!(min(1.0, f64::NAN).is_nan());
        assert!(max(f64::NAN, 1.0).is_nan());
        assert_eq!(max_n([] as [f64; 0]), f64::NEG_INFINITY);
    }
}
