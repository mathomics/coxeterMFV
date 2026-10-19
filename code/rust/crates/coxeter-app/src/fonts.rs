//! The interface's fonts.
//!
//! `app.css` sets `font-family: "Inter", "Segoe UI", system-ui, sans-serif`,
//! and its buttons carry symbols egui's bundled fonts do not have
//! (⌕ ◐ ↶ ↷ ⤢ ✥ ⌘ ⌁ ⬡ ▣ ‹ › ⌖ ⟲ ⟳ ≋ …). The system's fonts are read with
//! `fontdb`: a text face first (the CSS's list, then the usual system faces of
//! each platform), then symbol faces as fallbacks, then egui's own fonts, so
//! nothing renders as a missing-glyph box where some installed font has it.

use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily};

/// Every symbol the interface draws outside plain text.
pub const UI_GLYPHS: &str = "⌕◐↶↷⤢✥⌘⌁⬡▣‹›⌄►☷−⬉○⇄⟲⟳⌖◇◌≋⚙↗✓●×←→↑↓…µ⁻¹₂Δ⬌⬍✕⇖⇑⇗⇐⇒⇙⇓⇘☰↺▾▸▴–";

/// Symbols whose first choice in the JavaScript app has no outline in the
/// usual system fonts: Windows 11 keeps ▶ ↖ ↘ ↕ (characters with an emoji
/// form) only in its colour emoji font. These stand in for them.
pub mod glyph {
    /// `▶` of the Simulate menu.
    pub const RUN: &str = "►";
    /// `↖` of the Select tool.
    pub const SELECT: &str = "⬉";
    /// `↔` / `↕` of Distribute.
    pub const DISTRIBUTE_H: &str = "⬌";
    pub const DISTRIBUTE_V: &str = "⬍";
    /// The reaction compass (`↖ ↑ ↗ ← → ↙ ↓ ↘`), one family of arrows.
    pub const COMPASS: [(&str, &str); 8] =
        [("NW", "⇖"), ("N", "⇑"), ("NE", "⇗"), ("W", "⇐"), ("E", "⇒"), ("SW", "⇙"), ("S", "⇓"), ("SE", "⇘")];
}

/// Text faces, best first: what `app.css` asks for, then what each platform has.
const TEXT_FACES: &[&str] = &["Inter", "Segoe UI", "Helvetica Neue", "Cantarell", "Noto Sans", "DejaVu Sans", "Liberation Sans", "Arial"];
/// Faces with the symbols, in the order they are tried.
const SYMBOL_FACES: &[&str] = &[
    "Segoe UI Symbol",
    "Apple Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Noto Sans Math",
    "DejaVu Sans",
    "Cambria Math",
    "STIX Two Math",
    "Arial Unicode MS",
    // Windows 11 moved the arrows and triangles that have an emoji form
    // (▶ ↖ ↘ ↕) out of Segoe UI Symbol; its emoji face still outlines them.
    "Segoe UI Emoji",
];
const MONO_FACES: &[&str] = &["Cascadia Mono", "Consolas", "SF Mono", "Menlo", "DejaVu Sans Mono", "Liberation Mono", "Courier New"];

fn database() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// The bytes and face index of the first face of `families` installed, at `weight`.
fn face(families: &[&str], weight: u16) -> Option<(String, Arc<Vec<u8>>, u32)> {
    let db = database();
    for name in families {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let Some(id) = db.query(&query) else { continue };
        // A query falls back on the nearest weight; the family must be the one asked for.
        let Some(info) = db.face(id) else { continue };
        if !info.families.iter().any(|(f, _)| f.eq_ignore_ascii_case(name)) {
            continue;
        }
        if let Some((bytes, index)) = db.with_face_data(id, |data, index| (data.to_vec(), index)) {
            return Some(((*name).to_string(), Arc::new(bytes), index));
        }
    }
    None
}

/// The font definitions of the interface (worked out once per process).
pub fn definitions() -> FontDefinitions {
    static DEFS: OnceLock<FontDefinitions> = OnceLock::new();
    DEFS.get_or_init(build).clone()
}

fn build() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let defaults_prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let defaults_mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let add = |defs: &mut FontDefinitions, key: &str, (_, bytes, index): (String, Arc<Vec<u8>>, u32)| {
        let data = FontData { font: std::borrow::Cow::Owned((*bytes).clone()), index, tweak: Default::default() };
        defs.font_data.insert(key.to_string(), Arc::new(data));
        key.to_string()
    };

    let mut proportional = Vec::new();
    let mut bold = Vec::new();
    let mut mono = Vec::new();
    if let Some(f) = face(TEXT_FACES, 400) {
        proportional.push(add(&mut defs, "ui-text", f));
    }
    if let Some(f) = face(TEXT_FACES, 700) {
        bold.push(add(&mut defs, "ui-bold", f));
    } else if let Some(f) = face(TEXT_FACES, 600) {
        bold.push(add(&mut defs, "ui-bold", f));
    }
    if let Some(f) = face(MONO_FACES, 400) {
        mono.push(add(&mut defs, "ui-mono", f));
    }
    let mut symbols = Vec::new();
    for (i, name) in SYMBOL_FACES.iter().enumerate() {
        if let Some(f) = face(&[name], 400) {
            symbols.push(add(&mut defs, &format!("ui-symbols-{i}"), f));
        }
    }

    let chain = |first: &[String], rest: &[String]| {
        let mut out: Vec<String> = first.to_vec();
        out.extend(rest.iter().cloned());
        out.extend(symbols.iter().cloned());
        out
    };
    let prop = chain(&proportional, &defaults_prop);
    let mut bold_chain = bold.clone();
    bold_chain.extend(prop.iter().cloned());
    let mut mono_chain = chain(&mono, &defaults_mono);
    mono_chain.extend(proportional.iter().cloned());
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Monospace, mono_chain);
    defs.families.insert(crate::theme::bold(), bold_chain);
    defs
}

/// Install the fonts in a context.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// The interface glyphs no installed font can draw (empty when all can).
pub fn missing_glyphs(ctx: &egui::Context) -> Vec<char> {
    let id = egui::FontId::proportional(13.0);
    ctx.fonts_mut(|f| UI_GLYPHS.chars().filter(|c| !f.has_glyph(&id, *c)).collect())
}
