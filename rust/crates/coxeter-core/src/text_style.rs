//! Styles a free map text can have: a size and a font, from a short list every
//! system draws the same way, stored by key so a map looks the same elsewhere.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextFont {
    pub id: &'static str,
    pub label: &'static str,
    /// CSS font stack, as written into SVG exports.
    pub stack: &'static str,
    /// Average width of one character, in em, used for text hit boxes.
    pub advance: f64,
}

pub const TEXT_FONTS: [TextFont; 6] = [
    TextFont { id: "sans", label: "Sans serif", stack: "\"Inter\", \"Segoe UI\", system-ui, -apple-system, sans-serif", advance: 0.58 },
    TextFont { id: "serif", label: "Serif", stack: "Georgia, \"Times New Roman\", Times, serif", advance: 0.56 },
    TextFont { id: "mono", label: "Monospace", stack: "\"JetBrains Mono\", Consolas, \"Courier New\", monospace", advance: 0.64 },
    TextFont { id: "arial", label: "Arial", stack: "Arial, Helvetica, sans-serif", advance: 0.57 },
    TextFont { id: "times", label: "Times New Roman", stack: "\"Times New Roman\", Times, serif", advance: 0.52 },
    TextFont { id: "courier", label: "Courier New", stack: "\"Courier New\", Courier, monospace", advance: 0.64 },
];

pub const DEFAULT_TEXT_FONT: &str = "sans";

#[derive(Debug, Clone, Copy)]
pub struct TextSize {
    pub min: f64,
    pub max: f64,
    pub default: f64,
}
pub const TEXT_SIZE: TextSize = TextSize { min: 8.0, max: 144.0, default: 22.0 };

fn font(id: &str) -> &'static TextFont {
    TEXT_FONTS.iter().find(|f| f.id == id).unwrap_or(&TEXT_FONTS[0])
}

/// A known font key as it is, anything else the default font.
pub fn normalize_text_font(id: &str) -> &'static str {
    if TEXT_FONTS.iter().any(|f| f.id == id) { font(id).id } else { DEFAULT_TEXT_FONT }
}

/// The CSS font stack of a font key (the default font's for an unknown key).
pub fn text_font_stack(id: &str) -> &'static str {
    font(id).stack
}

/// Average character width of a font, in em.
pub fn text_font_advance(id: &str) -> f64 {
    font(id).advance
}

/// The size rounded to whole px and kept within the [`TEXT_SIZE`] limits. A
/// value that is not a finite number gets the default.
pub fn clamp_text_size(value: f64) -> f64 {
    if !value.is_finite() {
        return TEXT_SIZE.default;
    }
    TEXT_SIZE.max.min(TEXT_SIZE.min.max(crate::js::round(value)))
}

/// `clampTextSize(v)` for a JSON field: absent is `undefined`, which is NaN
/// and so the default; `null` is 0 and so the minimum, as in JS.
pub fn clamp_text_size_value(value: Option<&serde_json::Value>) -> f64 {
    match value {
        None => TEXT_SIZE.default,
        Some(v) => clamp_text_size(crate::js::to_number(v)),
    }
}

/// The box a free text is selected, moved and aligned by, estimated from the
/// font's average character width (`textBox` in `multi-selection.js`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBox {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// `textBox(annotation)`. `font_size` absent means 22, as `?? 22` reads it.
pub fn text_box(text: &str, x: f64, y: f64, font_size: Option<f64>, font_family: Option<&str>) -> TextBox {
    let size = font_size.unwrap_or(22.0);
    // `String#length` counts UTF-16 code units.
    let len = text.encode_utf16().count() as f64;
    let width = len * size * text_font_advance(font_family.unwrap_or(""));
    TextBox { left: x, right: x + width, top: y - size, bottom: y }
}

/// [`text_box`] of a map text.
pub fn text_box_of(t: &crate::mapdata::MapText) -> TextBox {
    text_box(&t.text, t.x, t.y, t.font_size, t.font_family.as_deref())
}

/// The centre of a text's box (`textCentre`).
pub fn text_centre(t: &crate::mapdata::MapText) -> crate::mapdata::Point {
    let b = text_box_of(t);
    crate::mapdata::Point::new((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0)
}
