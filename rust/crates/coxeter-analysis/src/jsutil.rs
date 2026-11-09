//! JavaScript behaviours this crate needs beyond `coxeter_core::js`.
//!
//! The modules ported here split text with JavaScript regular expressions and
//! clean it with `String#trim`, whose idea of whitespace is not Rust's: it
//! includes U+FEFF and leaves U+0085 alone. `coxeter-io` keeps the same
//! helpers private. Both are candidates for `coxeter_core::js`.

use std::cmp::Ordering;

use coxeter_core::js;
use serde_json::Value;

/// The characters of JavaScript's `\s`, ready to go inside `[...]`.
pub(crate) const WS_CHARS: &str =
    r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// Is `c` whitespace as JavaScript's `\s` and `String#trim` see it?
pub(crate) fn is_js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

/// `text.split(/\s+/)`: the pieces between runs of JavaScript whitespace,
/// with the empty first and last pieces a leading or trailing run leaves.
pub(crate) fn split_ws(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_ws = false;
    for (i, c) in s.char_indices() {
        if is_js_ws(c) {
            if !in_ws {
                out.push(&s[start..i]);
                in_ws = true;
            }
            start = i + c.len_utf8();
        } else {
            in_ws = false;
        }
    }
    out.push(&s[start..]);
    out
}

/// `Number(text)`: blank is 0, otherwise the whole trimmed text must be a
/// number. (`coxeter_core::js::string_to_number` trims as Rust does, which
/// would also drop a U+0085 that JavaScript keeps and then refuses.)
pub(crate) fn number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.starts_with('\u{85}') || t.ends_with('\u{85}') {
        return f64::NAN;
    }
    js::string_to_number(t)
}

/// The comparator `(a, b) => a - b`: ascending, a NaN difference leaving the
/// pair as it is.
pub(crate) fn ascending(a: &f64, b: &f64) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

/// The comparator `(a, b) => b - a`.
pub(crate) fn descending(a: &f64, b: &f64) -> Ordering {
    b.partial_cmp(a).unwrap_or(Ordering::Equal)
}

/// A JSON number for a flux or a score: JSON has no NaN, and `JSON.stringify`
/// writes `null` for it.
pub(crate) fn json_number(v: f64) -> Value {
    if v.is_finite() { serde_json::json!(v) } else { Value::Null }
}

/// `text.split(/\r?\n/)`.
pub(crate) fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// `JSON.stringify(s)` for a string.
pub(crate) fn json_quote(s: &str) -> String {
    serde_json::to_string(s).expect("a string serialises")
}

/// `JSON.stringify(s.slice(0, n))`: the first `n` UTF-16 code units, quoted.
/// A pair cut in half leaves its first half, which JSON writes as `\udXXX`.
pub(crate) fn json_quote_prefix(s: &str, n: usize) -> String {
    let mut prefix = String::new();
    let mut units = 0;
    let mut dangling = None;
    for c in s.chars() {
        let width = c.len_utf16();
        if units + width > n {
            if units < n {
                let mut pair = [0u16; 2];
                c.encode_utf16(&mut pair);
                dangling = Some(pair[0]);
            }
            break;
        }
        units += width;
        prefix.push(c);
    }
    let quoted = json_quote(&prefix);
    match dangling {
        Some(high) => format!("{}\\u{high:04x}\"", &quoted[..quoted.len() - 1]),
        None => quoted,
    }
}

/// `text.split(/\s+/).map(Number)`.
pub(crate) fn numbers_split_ws(text: &str) -> Vec<f64> {
    split_ws(text).into_iter().map(number).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_splits_as_javascript_does() {
        assert_eq!(js_trim("\u{FEFF} a b\u{A0}"), "a b");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
        assert_eq!(split_ws(" a  b "), ["", "a", "b", ""]);
        assert_eq!(split_ws("a"), ["a"]);
        assert_eq!(split_ws(""), [""]);
        assert_eq!(split_ws("a\t\u{3000}b"), ["a", "b"]);
    }

    #[test]
    fn quotes_a_prefix_as_json_stringify_does() {
        assert_eq!(json_quote_prefix("abc", 2), r#""ab""#);
        assert_eq!(json_quote_prefix("a\"b\tc", 40), r#""a\"b\tc""#);
        assert_eq!(json_quote_prefix("a😀b", 2), r#""a\ud83d""#);
        assert_eq!(json_quote_prefix("a😀b", 3), r#""a😀""#);
        assert_eq!(json_quote_prefix("\u{1}", 9), r#""\u0001""#);
    }

    #[test]
    fn numbers_read_as_javascript_reads_them() {
        assert_eq!(number(""), 0.0);
        assert_eq!(number(" 2.5e+00 "), 2.5);
        assert_eq!(number("\u{FEFF}7"), 7.0);
        assert!(number("\u{85}7").is_nan());
        assert!(number("7 mmol").is_nan());
        assert_eq!(number("-1.0e+00"), -1.0);
    }
}
