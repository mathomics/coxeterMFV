//! Visual scales (`src/render/theme.js`).
//!
//! Flux magnitude is carried by stroke width and by a sequential colour ramp;
//! direction is carried by the arrowhead. Compartment fills are deliberately
//! low-chroma pastels so that saturated flux lines stay readable on top of any
//! of them, in both colour schemes.

// `!(a > b)` is not `a <= b` when a value is NaN, and the JavaScript tests it
// that way round on purpose; the port keeps the comparisons as they are.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use coxeter_core::compartments::{Names, compartment_colors};
use coxeter_core::js;
use serde_json::Value;

use crate::format::{expm1, log1p, to_exponential};

/// Sequential ramp for |flux|. Stops are (position, colour).
const FLUX_RAMP_LIGHT: [(f64, &str); 5] =
    [(0.00, "#cbd5e1"), (0.12, "#38bdf8"), (0.35, "#2563eb"), (0.65, "#7c3aed"), (1.00, "#be123c")];

const FLUX_RAMP_DARK: [(f64, &str); 5] =
    [(0.00, "#475569"), (0.12, "#38bdf8"), (0.35, "#60a5fa"), (0.65, "#a78bfa"), (1.00, "#fb7185")];

/// The automatic ramp of a colour scheme.
pub fn flux_ramp(dark: bool) -> &'static [(f64, &'static str)] {
    if dark { &FLUX_RAMP_DARK } else { &FLUX_RAMP_LIGHT }
}

/// `hexToRgb`: `#rrggbb` to channels (`parseInt` of each pair; NaN where a
/// pair is not hexadecimal).
pub fn hex_to_rgb(hex: &str) -> [f64; 3] {
    let pair = |i: usize| -> f64 {
        hex.get(i..i + 2).and_then(parse_int_hex_prefix).unwrap_or(f64::NAN)
    };
    [pair(1), pair(3), pair(5)]
}

/// `parseInt(s, 16)`: the longest hexadecimal prefix.
fn parse_int_hex_prefix(s: &str) -> Option<f64> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if digits.is_empty() { None } else { u64::from_str_radix(&digits, 16).ok().map(|v| v as f64) }
}

/// `rgbToHex`: each channel rounded (`Math.round`) and written as two hex digits.
pub fn rgb_to_hex(rgb: [f64; 3]) -> String {
    let mut out = String::from("#");
    for v in rgb {
        let r = js::round(v);
        // `Math.round(v).toString(16).padStart(2, '0')`; channels stay in 0..=255.
        let text = if r.is_nan() { "NaN".to_string() } else { format!("{:x}", r as i64) };
        if text.len() < 2 {
            out.push('0');
        }
        out.push_str(&text);
    }
    out
}

/// One point of a user-defined flux colour scale.
#[derive(Debug, Clone, PartialEq)]
pub struct FluxColorStop {
    pub value: f64,
    /// `#rrggbb`, lower case once normalised.
    pub color: String,
}

/// Why a flux colour scale was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ThemeError {
    #[error("Flux colours need at least one point, or null for the automatic palette.")]
    NoStops,
    #[error("Flux colour point {0} needs a finite numeric value.")]
    BadValue(usize),
    #[error("Flux colour point {0} needs a colour in #rrggbb format.")]
    BadColor(usize),
    #[error("Flux colour values must be unique; {0} appears more than once.")]
    Duplicate(String),
}

fn is_hex_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `normalizeFluxColorStops(stops)` for stops already typed: validated,
/// owned copies sorted by value.
pub fn normalize_flux_color_stops(stops: &[FluxColorStop]) -> Result<Vec<FluxColorStop>, ThemeError> {
    if stops.is_empty() {
        return Err(ThemeError::NoStops);
    }
    let mut seen: Vec<f64> = Vec::new();
    let mut normalized = Vec::with_capacity(stops.len());
    for (i, stop) in stops.iter().enumerate() {
        if !stop.value.is_finite() {
            return Err(ThemeError::BadValue(i + 1));
        }
        let color = crate::format::js_trim(&stop.color).to_lowercase();
        if !is_hex_color(&color) {
            return Err(ThemeError::BadColor(i + 1));
        }
        let value = if stop.value == 0.0 { 0.0 } else { stop.value };
        if seen.contains(&value) {
            return Err(ThemeError::Duplicate(js::number_to_string(value)));
        }
        seen.push(value);
        normalized.push(FluxColorStop { value, color });
    }
    normalized.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    Ok(normalized)
}

/// `normalizeFluxColorStops(stops)` for stops as a saved file holds them:
/// `null` is the automatic palette (`Ok(None)`); anything else must be a
/// non-empty array of `{value, color}`.
pub fn normalize_flux_color_stops_value(stops: &Value) -> Result<Option<Vec<FluxColorStop>>, ThemeError> {
    let list = match stops {
        Value::Null => return Ok(None),
        Value::Array(a) if !a.is_empty() => a,
        _ => return Err(ThemeError::NoStops),
    };
    let mut typed = Vec::with_capacity(list.len());
    for (i, stop) in list.iter().enumerate() {
        // `Number.isFinite(stop.value)`: a number, not a numeric string.
        let value = match stop.get("value") {
            Some(Value::Number(v)) => v.as_f64().unwrap_or(f64::NAN),
            _ => return Err(ThemeError::BadValue(i + 1)),
        };
        let color = match stop.get("color") {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        typed.push(FluxColorStop { value, color });
    }
    normalize_flux_color_stops(&typed).map(Some)
}

/// Linear position without overflowing for finite endpoints of opposite sign.
fn signed_position(value: f64, lower: f64, upper: f64) -> f64 {
    if !(value > lower) {
        return 0.0;
    }
    if value >= upper {
        return 1.0;
    }
    let span = upper - lower;
    if span.is_finite() { (value - lower) / span } else { (value / 2.0 - lower / 2.0) / (upper / 2.0 - lower / 2.0) }
}

/// The channels of a well-formed `#rrggbb`, without going through strings.
fn hex_channels(hex: &str) -> Option<[f64; 3]> {
    let b = hex.as_bytes();
    if b.len() < 7 || b[0] != b'#' {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16);
    let pair = |i: usize| Some((digit(b[i])? * 16 + digit(b[i + 1])?) as f64);
    Some([pair(1)?, pair(3)?, pair(5)?])
}

fn mix_hex(from: &str, to: &str, t: f64) -> String {
    let a = hex_channels(from).unwrap_or_else(|| hex_to_rgb(from));
    let b = hex_channels(to).unwrap_or_else(|| hex_to_rgb(to));
    let mixed = [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t);
    // Two hex digits per channel, written directly when every channel is one byte.
    let rounded = mixed.map(js::round);
    if rounded.iter().all(|c| (0.0..=255.0).contains(c)) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(7);
        out.push('#');
        for c in rounded {
            let c = c as usize;
            out.push(HEX[c >> 4] as char);
            out.push(HEX[c & 15] as char);
        }
        return out;
    }
    rgb_to_hex(mixed)
}

fn sample_flux_color_stops(stops: &[FluxColorStop], value: f64) -> String {
    if stops.len() == 1 || !(value > stops[0].value) {
        return stops[0].color.clone();
    }
    let last = stops.len() - 1;
    if value >= stops[last].value {
        return stops[last].color.clone();
    }
    let mut lower = 0;
    let mut upper = last;
    while upper - lower > 1 {
        let middle = (lower + upper) / 2;
        if value < stops[middle].value {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let from = &stops[lower];
    let to = &stops[upper];
    mix_hex(&from.color, &to.color, signed_position(value, from.value, to.value))
}

/// `sampleRamp(ramp, t)`: the colour at `t` (clamped to 0..1) along a ramp.
pub fn sample_ramp(ramp: &[(f64, &str)], t: f64) -> String {
    let p = js::min(js::max(t, 0.0), 1.0);
    for i in 1..ramp.len() {
        if p <= ramp[i].0 {
            let (p0, c0) = ramp[i - 1];
            let (p1, c1) = ramp[i];
            let span = p1 - p0;
            let f = (p - p0) / if span == 0.0 || span.is_nan() { 1.0 } else { span };
            return mix_hex(c0, c1, f);
        }
    }
    ramp[ramp.len() - 1].1.to_string()
}

/// What the renderer asks of a flux scale. [`FluxScale`] is the one the app
/// uses; tests and callers with their own idea of width and colour can supply
/// another.
pub trait Scale {
    /// The magnitude the ramp reaches its top at; labels are drawn for
    /// reactions above a fraction of it.
    fn reference(&self) -> f64;
    /// Stroke width for a flux.
    fn width(&self, v: f64) -> f64;
    /// Colour for a flux.
    fn color(&self, v: f64) -> String;
}

/// Options of [`FluxScale::new`]; unset fields take the JavaScript defaults.
#[derive(Debug, Clone, Default)]
pub struct FluxScaleOptions {
    pub color_stops: Option<Vec<FluxColorStop>>,
    pub dark: bool,
    pub min_width: Option<f64>,
    pub max_width: Option<f64>,
    pub zero_width: Option<f64>,
}

/// Scale for a whole flux map. Uses a log-ish compression so that a map with one
/// huge flux does not render everything else as a hairline.
#[derive(Debug, Clone, PartialEq)]
pub struct FluxScale {
    pub color_stops: Option<Vec<FluxColorStop>>,
    pub count: usize,
    pub min: f64,
    pub max: f64,
    /// Can be overridden after construction (the Zuñiga study pins it).
    pub reference: f64,
    pub dark: bool,
    pub min_width: f64,
    pub max_width: f64,
    pub zero_width: f64,
}

/// One swatch of the legend.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendStop {
    pub t: f64,
    pub value: f64,
    pub color: String,
    pub width: f64,
}

impl FluxScale {
    /// `new FluxScale(values, opts)`. Fails only on invalid colour stops.
    pub fn new(values: impl IntoIterator<Item = f64>, opts: FluxScaleOptions) -> Result<FluxScale, ThemeError> {
        let color_stops = match opts.color_stops {
            None => None,
            Some(stops) => Some(normalize_flux_color_stops(&stops)?),
        };
        let mut magnitudes: Vec<f64> = values.into_iter().map(f64::abs).filter(|m| m.is_finite() && *m > 1e-9).collect();
        magnitudes.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let len = magnitudes.len();
        // A high percentile rather than the maximum: one runaway water flux should
        // not flatten the rest of the map.
        let reference = if len > 0 {
            magnitudes[(len - 1).min((len as f64 * 0.93).floor() as usize)]
        } else {
            1.0
        };
        Ok(FluxScale {
            color_stops,
            count: len,
            min: magnitudes.first().copied().unwrap_or(0.0),
            max: magnitudes.last().copied().unwrap_or(1.0),
            reference,
            dark: opts.dark,
            min_width: opts.min_width.unwrap_or(1.6),
            max_width: opts.max_width.unwrap_or(15.0),
            zero_width: opts.zero_width.unwrap_or(1.1),
        })
    }

    /// The automatic palette, which cannot fail.
    pub fn automatic(values: impl IntoIterator<Item = f64>, dark: bool) -> FluxScale {
        FluxScale::new(values, FluxScaleOptions { dark, ..Default::default() }).expect("no colour stops to refuse")
    }

    /// Normalised position of |v| on the ramp, 0..1.
    pub fn position(&self, v: f64) -> f64 {
        let m = v.abs();
        if !(m > 1e-9) {
            return 0.0;
        }
        let r = js::max(self.reference, 1e-9);
        js::min(1.0, log1p(m / r * 9.0) / js::log(10.0))
    }

    pub fn width(&self, v: f64) -> f64 {
        if !(v.abs() > 1e-9) {
            return self.zero_width;
        }
        self.min_width + self.position(v) * (self.max_width - self.min_width)
    }

    pub fn color(&self, v: f64) -> String {
        if let Some(stops) = &self.color_stops {
            return sample_flux_color_stops(stops, v);
        }
        let ramp = flux_ramp(self.dark);
        if !(v.abs() > 1e-9) {
            return ramp[0].1.to_string();
        }
        sample_ramp(ramp, self.position(v))
    }

    /// Swatches for the legend (`legendStops(n = 6)`).
    pub fn legend_stops(&self, n: usize) -> Vec<LegendStop> {
        if let Some(stops) = &self.color_stops {
            let first = stops[0].value;
            let last = stops[stops.len() - 1].value;
            return stops
                .iter()
                .map(|s| LegendStop {
                    t: if stops.len() == 1 { 0.0 } else { signed_position(s.value, first, last) },
                    value: s.value,
                    color: s.color.clone(),
                    width: self.width(s.value),
                })
                .collect();
        }
        (0..n)
            .map(|i| {
                let t = i as f64 / (n as f64 - 1.0);
                let value = if t == 0.0 { 0.0 } else { (expm1(t * js::log(10.0)) / 9.0) * self.reference };
                let shown = if value.is_nan() { 0.0 } else { value };
                LegendStop { t, value, color: self.color(shown), width: self.width(value) }
            })
            .collect()
    }
}

impl Scale for FluxScale {
    fn reference(&self) -> f64 {
        self.reference
    }
    fn width(&self, v: f64) -> f64 {
        FluxScale::width(self, v)
    }
    fn color(&self, v: f64) -> String {
        FluxScale::color(self, v)
    }
}

/// Colours of a compartment in the active scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompartmentStyle {
    pub fill: &'static str,
    pub stroke: &'static str,
    pub text: &'static str,
}

/// `compartmentStyle(id, dark)`.
pub fn compartment_style(id: &str, dark: bool) -> CompartmentStyle {
    let c = compartment_colors(id, dark, &Names::new());
    CompartmentStyle { fill: c.fill, stroke: c.stroke, text: c.text }
}

/// `formatFlux(v, digits = 3)`; `None` is a missing value (`—`).
pub fn format_flux(v: Option<f64>, digits: usize) -> String {
    let Some(v) = v.filter(|v| !v.is_nan()) else { return "—".into() };
    if v.abs() < 1e-9 {
        return "0".into();
    }
    if v.abs() >= 1000.0 {
        return to_exponential(v, 2);
    }
    js::number_to_string(js::string_to_number(&js::to_fixed(v, digits)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex_to_rgb("#0a1B2c"), [10.0, 27.0, 44.0]);
        assert_eq!(rgb_to_hex([10.0, 27.4, 44.5]), "#0a1b2d");
    }

    #[test]
    fn flux_format() {
        assert_eq!(format_flux(Some(0.0), 3), "0");
        assert_eq!(format_flux(Some(1.23456), 3), "1.235");
        assert_eq!(format_flux(Some(-0.0004), 3), "0");
        assert_eq!(format_flux(Some(2.5), 3), "2.5");
        assert_eq!(format_flux(Some(12345.0), 3), "1.23e+4");
        assert_eq!(format_flux(None, 3), "—");
        assert_eq!(format_flux(Some(f64::NAN), 3), "—");
    }
}
