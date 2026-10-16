//! JavaScript behaviours this crate needs beyond `coxeter_core::js`.
//!
//! The readers ported here are written with JavaScript regular expressions and
//! string methods, and their output is defined by those: `\s` includes U+FEFF,
//! `.` stops at U+2028, `\b` and `\w` are ASCII, an object lists its
//! integer-like keys first. Everything that has to agree with that lives here.
//! Candidates for `coxeter_core::js` once a second crate needs them.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use serde_json::{Map, Value};

use coxeter_core::js;

/// The characters of JavaScript's `\s`, ready to go inside `[...]`.
pub const WS_CHARS: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// JavaScript `\s` as a Rust regex class.
pub static S: LazyLock<String> = LazyLock::new(|| format!("[{WS_CHARS}]"));
/// JavaScript `\S`.
pub static NS: LazyLock<String> = LazyLock::new(|| format!("[^{WS_CHARS}]"));
/// JavaScript `\w` (ASCII only).
pub const W: &str = "[0-9A-Za-z_]";
/// JavaScript `\b` (an ASCII word boundary).
pub const B: &str = r"(?-u:\b)";
/// JavaScript `.` without the `s` flag: anything but a line terminator.
pub const DOT: &str = r"[^\n\r\x{2028}\x{2029}]";

/// Is `c` whitespace as JavaScript's `\s` and `String#trim` see it?
pub fn is_js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

/// A compiled regex, cached by pattern (the readers build a few per tag name).
pub fn regex(pattern: &str) -> Arc<Regex> {
    static CACHE: LazyLock<Mutex<HashMap<String, Arc<Regex>>>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = cache.get(pattern) {
        return r.clone();
    }
    let r = Arc::new(Regex::new(pattern).unwrap_or_else(|e| panic!("bad built-in pattern {pattern}: {e}")));
    cache.insert(pattern.to_string(), r.clone());
    r
}

/// Reorder an object's keys as JavaScript lists them: array-index keys first,
/// ascending, then the rest in insertion order.
pub fn js_order(map: Map<String, Value>) -> Map<String, Value> {
    if !map.keys().any(|k| js::is_array_index(k)) {
        return map;
    }
    let mut entries: Vec<(String, Value)> = map.into_iter().collect();
    entries.sort_by(|(a, _), (b, _)| match (js::is_array_index(a), js::is_array_index(b)) {
        (true, true) => a.parse::<u64>().unwrap_or(0).cmp(&b.parse::<u64>().unwrap_or(0)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => std::cmp::Ordering::Equal,
    });
    entries.into_iter().collect()
}

/// `Object.entries(v || {})` for a JSON value: an object's entries in JS key
/// order, an array's by index, a string's by character; nothing otherwise.
pub fn object_entries(v: &Value) -> Vec<(String, Value)> {
    match v {
        Value::Object(m) => js_order(m.clone()).into_iter().collect(),
        Value::Array(a) => a.iter().enumerate().map(|(i, x)| (i.to_string(), x.clone())).collect(),
        Value::String(s) => s.chars().enumerate().map(|(i, c)| (i.to_string(), Value::String(c.to_string()))).collect(),
        _ => Vec::new(),
    }
}

/// `String(v ?? '')`.
pub fn string_or_empty(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js::to_string(v),
    }
}

/// Length in UTF-16 code units (`s.length`).
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(n)` for `n` code units, never splitting a character.
pub fn utf16_skip(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        if units >= n {
            return &s[i..];
        }
        units += c.len_utf16();
    }
    ""
}

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `decodeURIComponent`, or `None` where JavaScript throws `URIError: URI malformed`.
pub fn decode_uri_component(s: &str) -> Option<String> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let byte_at = |k: usize| -> Option<u8> {
        if bytes.get(k) != Some(&b'%') {
            return None;
        }
        Some(hex(*bytes.get(k + 1)?)? * 16 + hex(*bytes.get(k + 2)?)?)
    };
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let first = byte_at(i)?;
        i += 3;
        if first < 0x80 {
            out.push(first);
            continue;
        }
        let n = first.leading_ones() as usize;
        if n == 1 || n > 4 {
            return None;
        }
        let mut seq = vec![first];
        for _ in 1..n {
            let b = byte_at(i)?;
            if b & 0xC0 != 0x80 {
                return None;
            }
            seq.push(b);
            i += 3;
        }
        // Overlong forms, surrogates and code points past U+10FFFF are malformed
        // to JavaScript as well as to Rust.
        let text = std::str::from_utf8(&seq).ok()?;
        out.extend_from_slice(text.as_bytes());
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uri_components() {
        assert_eq!(encode_uri_component("a b/ü?(x)"), "a%20b%2F%C3%BC%3F(x)");
        assert_eq!(decode_uri_component("chebi%3A15377").as_deref(), Some("chebi:15377"));
        assert_eq!(decode_uri_component("%C3%BC%2f").as_deref(), Some("ü/"));
        assert_eq!(decode_uri_component("100%"), None);
        assert_eq!(decode_uri_component("%E0%A4%A"), None);
        assert_eq!(decode_uri_component("%C0%80"), None);
        assert_eq!(decode_uri_component("%ED%A0%80"), None);
        assert_eq!(decode_uri_component("%80"), None);
    }

    #[test]
    fn whitespace_and_order() {
        assert_eq!(js_trim("\u{FEFF} x \u{2028}"), "x");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x");
        let m = js_order(json!({"b": 1, "10": 2, "a": 3, "2": 4}).as_object().unwrap().clone());
        assert_eq!(m.keys().cloned().collect::<Vec<_>>(), ["2", "10", "b", "a"]);
        assert_eq!(utf16_skip("ab😀c", 2), "😀c");
        assert_eq!(utf16_len("😀"), 2);
    }
}
