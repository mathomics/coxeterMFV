//! How the JavaScript renderer prints numbers and text into the SVG.
//!
//! These are the small helpers every drawing module shares: `n()` and `esc()`
//! of `src/render/mapview.js` (and the same `n()` of `reaction-path.js`), plus
//! the few JavaScript string and number behaviours (`String#length`,
//! `String#trim`, `toExponential`, `Math.log1p`) the renderer relies on and
//! `coxeter_core::js` does not cover.

use coxeter_core::js;

/// `n(v)`: a coordinate rounded to two decimals and printed as JavaScript
/// prints it (`(Math.round(v * 100) / 100).toString()`).
pub fn n(v: f64) -> String {
    let hundredths = js::round(v * 100.0);
    // The renderer prints a couple of hundred thousand of these for a large
    // map, so the common case is written out directly. `hundredths / 100` is
    // the double nearest to that decimal, and a decimal of at most 15
    // significant digits is the shortest text that reads back as its nearest
    // double: what JavaScript prints is the decimal itself, without its
    // trailing zeros.
    if hundredths.abs() < 1e11 {
        let k = hundredths as i64;
        let a = k.unsigned_abs();
        let (int, frac) = (a / 100, (a % 100) as u8);
        let mut digits = [0u8; 20];
        let mut at = digits.len();
        let mut rest = int;
        loop {
            at -= 1;
            digits[at] = b'0' + (rest % 10) as u8;
            rest /= 10;
            if rest == 0 {
                break;
            }
        }
        let mut out = String::with_capacity(digits.len() - at + 4);
        if k < 0 {
            out.push('-');
        }
        out.extend(digits[at..].iter().map(|d| *d as char));
        if frac != 0 {
            out.push('.');
            out.push((b'0' + frac / 10) as char);
            if frac % 10 != 0 {
                out.push((b'0' + frac % 10) as char);
            }
        }
        return out;
    }
    js::number_to_string(hundredths / 100.0)
}

/// `esc(s)`: the four characters markup cannot carry as they are.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    esc_into(&mut out, s);
    out
}

/// [`esc`] appended to a buffer.
pub fn esc_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// `String#length`: UTF-16 code units, which is what the plate widths count.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Is this a character `String#trim` removes (WhiteSpace or LineTerminator)?
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String#trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// `Math.log1p`, as V8 computes it (fdlibm).
pub fn log1p(x: f64) -> f64 {
    libm::log1p(x)
}

/// `Math.expm1`, as V8 computes it (fdlibm).
pub fn expm1(x: f64) -> f64 {
    libm::expm1(x)
}

/// `Number.prototype.toExponential(digits)`: one digit before the point,
/// `digits` after it, ties taking the larger value (`1125 → "1.13e+3"`).
pub fn to_exponential(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let ax = x.abs();
    if ax == 0.0 {
        let zeros = "0".repeat(digits);
        return if digits == 0 { "0e+0".into() } else { format!("0.{zeros}e+0") };
    }
    // Enough digits for the exact decimal value of any double: a double has at
    // most 767 significant decimal digits.
    let exact = format!("{:.800e}", ax);
    let (mantissa, exp) = exact.split_once('e').expect("LowerExp has an exponent");
    let mut exp: i32 = exp.parse().expect("integer exponent");
    let all: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    let mut kept: Vec<u8> = all[..=digits].to_vec();
    // JS picks the larger candidate on a tie, so any remainder starting with 5
    // rounds up.
    if all.get(digits + 1).is_some_and(|d| *d >= b'5') {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                kept.pop();
                exp += 1;
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let s = String::from_utf8(kept).expect("ascii digits");
    let body = if digits == 0 { s } else { format!("{}.{}", &s[..1], &s[1..]) };
    let e_sign = if exp < 0 { '-' } else { '+' };
    format!("{sign}{body}e{e_sign}{}", exp.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_print_like_the_renderer() {
        assert_eq!(n(1.005), "1");
        assert_eq!(n(-0.004), "0");
        assert_eq!(n(12.345), "12.35");
        assert_eq!(n(-12.345), "-12.34");
        assert_eq!(n(0.1 + 0.2), "0.3");
        assert_eq!(n(f64::NAN), "NaN");
        assert_eq!(n(27400.0), "27400");
    }

    #[test]
    fn the_short_way_prints_what_javascript_prints() {
        // Against the general printer, over magnitudes and awkward fractions.
        let mut x = 0.000_123_456_789_f64;
        let mut checked = 0;
        while x < 5e12 {
            for v in [x, -x, x + 0.005, x - 0.005, (x * 100.0).round() / 100.0 + 0.004_999_999, x.floor() + 0.1, x.floor() + 0.07] {
                assert_eq!(n(v), js::number_to_string(js::round(v * 100.0) / 100.0), "{v}");
                checked += 1;
            }
            x *= 1.37;
        }
        assert!(checked > 700);
        for v in [0.0, -0.0, 0.004, -0.004, 0.005, -0.005, 0.01, -0.01, 0.1, 0.99, 0.995, 1e9, 99_999_999.99, 1e11, 1e12, 1e21, -1e25, f64::INFINITY] {
            assert_eq!(n(v), js::number_to_string(js::round(v * 100.0) / 100.0), "{v}");
        }
    }

    #[test]
    fn escaping_and_lengths() {
        assert_eq!(esc(r#"a<b>&"c"'"#), "a&lt;b&gt;&amp;&quot;c&quot;'");
        assert_eq!(utf16_len("µ = h⁻¹"), 7);
        assert_eq!(utf16_len("𝔸"), 2);
        assert_eq!(js_trim("\u{feff} x \u{a0}"), "x");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x");
    }

    #[test]
    fn exponentials_round_ties_up() {
        assert_eq!(to_exponential(1234.5, 2), "1.23e+3");
        assert_eq!(to_exponential(1125.0, 2), "1.13e+3");
        assert_eq!(to_exponential(-99999.0, 2), "-1.00e+5");
        assert_eq!(to_exponential(1000.0, 2), "1.00e+3");
        assert_eq!(to_exponential(0.000123, 1), "1.2e-4");
        assert_eq!(to_exponential(5.0, 0), "5e+0");
    }
}
