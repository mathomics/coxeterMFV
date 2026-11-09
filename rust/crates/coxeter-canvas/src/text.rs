//! Text, drawn as glyph outlines in map coordinates: the fonts of the system
//! (`fontdb`), shaped with `harfrust`, outlined with `skrifa` (the same three
//! libraries, and the same choices, as usvg/resvg, which draw the exported PNG),
//! so a label scales with the map like any other shape and looks the same on
//! screen and in the export.
//!
//! Layout follows usvg (and Chrome): one line, `text-anchor` on the advance
//! width, `dominant-baseline` from the font's ascent and descent,
//! `letter-spacing` between clusters (not after the last). `textLength` with
//! `lengthAdjust="spacingAndGlyphs"` stretches the whole run, glyphs and gaps,
//! to the given width, as browsers do (usvg stretches the glyphs only).
//!
//! Glyphs are tessellated once per font and glyph, at one em, and placed by
//! copying; a halo stroke once per glyph and width. A line is shaped once per
//! string and setting: a label drawn again (moved, restyled, the map
//! repainted) is not shaped again.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use lyon_tessellation::math::point;
use lyon_tessellation::path::Path;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};

use crate::style::{Anchor, Baseline, LineJoin};
use crate::tess::{self, MeshBuf, StrokeSpec};

/// A face loaded for drawing.
pub struct Face {
    pub db_id: fontdb::ID,
    data: Arc<Vec<u8>>,
    index: u32,
    pub units_per_em: f64,
    /// In font units, rounded as usvg rounds them.
    pub ascent: f64,
    pub descent: f64,
    pub x_height: f64,
    pub underline_position: f64,
    pub underline_thickness: f64,
    shaper: harfrust::ShaperData,
}

impl Face {
    fn skrifa(&self) -> Option<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(&self.data, self.index).ok()
    }

    fn has_char(&self, c: char) -> bool {
        self.skrifa().is_some_and(|f| f.charmap().map(c).is_some())
    }

    /// `dominantBaselineShift`: how far the baseline moves down, in px.
    pub fn baseline_shift(&self, baseline: Baseline, size: f64) -> f64 {
        let s = size / self.units_per_em;
        let ascent = self.ascent * s;
        let descent = self.descent * s;
        match baseline {
            Baseline::Auto | Baseline::Alphabetic => 0.0,
            Baseline::TextBeforeEdge => ascent,
            Baseline::Middle => self.x_height * s * 0.5,
            Baseline::Central => ascent - (ascent - descent) * 0.5,
            Baseline::TextAfterEdge | Baseline::Ideographic => descent,
            Baseline::Hanging => ascent * 0.8,
            Baseline::Mathematical => ascent * 0.5,
        }
    }
}

/// An index into the book's faces.
pub type FaceKey = u32;

/// What a run of text is set in.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpec<'a> {
    /// The CSS `font-family` list.
    pub families: &'a str,
    pub weight: u16,
    pub size: f64,
    pub letter_spacing: f64,
    pub anchor: Anchor,
    pub baseline: Baseline,
    /// `textLength` with `lengthAdjust="spacingAndGlyphs"`.
    pub text_length: Option<f64>,
    pub tabular: bool,
}

/// One glyph, placed: `x`, `y` in px from the text's anchor point (`x`/`y`
/// attributes), `y` down; the outline is scaled by `size` (and by `sx`
/// horizontally).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    pub face: FaceKey,
    pub gid: u32,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub sx: f64,
}

/// A laid-out line of text, relative to its anchor point.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextRun {
    pub glyphs: Vec<PlacedGlyph>,
    /// Left edge and width of the run (after anchoring).
    pub left: f64,
    pub width: f64,
    /// Top and bottom of the character cells (y down, from the anchor point).
    pub top: f64,
    pub bottom: f64,
    /// The underline: y and thickness.
    pub underline: (f64, f64),
}

/// The fonts of the system, and what has been made of them.
pub struct FontBook {
    db: fontdb::Database,
    faces: Vec<Face>,
    by_db: HashMap<fontdb::ID, FaceKey>,
    queries: HashMap<(String, u16), Option<FaceKey>>,
    fallbacks: HashMap<(FaceKey, char), Option<FaceKey>>,
    fills: HashMap<(FaceKey, u32, usize), Option<Arc<MeshBuf>>>,
    strokes: HashMap<(FaceKey, u32, u32, u8, usize), Option<Arc<MeshBuf>>>,
    outlines: HashMap<(FaceKey, u32), Option<Arc<Path>>>,
    layouts: HashMap<LayoutKey, Option<TextRun>>,
    /// Lines shaped (not found in `layouts`), for tests and diagnostics.
    pub shaped: u64,
}

/// What a laid-out line depends on: the string and everything of its
/// [`TextSpec`] (numbers by their bits).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LayoutKey {
    text: String,
    families: String,
    weight: u16,
    size: u64,
    letter_spacing: u64,
    anchor: u8,
    baseline: u8,
    text_length: Option<u64>,
    tabular: bool,
}

/// Lines kept laid out, at most (the cache is emptied past it).
const LAYOUTS_KEPT: usize = 50_000;

static BOOK: LazyLock<Mutex<FontBook>> = LazyLock::new(|| Mutex::new(FontBook::system()));

/// The process's font book.
pub fn font_book() -> std::sync::MutexGuard<'static, FontBook> {
    BOOK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Tessellation tolerance of glyph outlines at each level of detail, in em.
pub const GLYPH_TOLERANCE: [f32; 3] = [0.002, 0.008, 0.04];

/// The level of detail for text drawn `px` pixels high: within about a
/// quarter of a pixel at every level.
pub fn lod_for_pixels(px: f64) -> usize {
    if px >= 30.0 {
        0
    } else if px >= 6.0 {
        1
    } else {
        2
    }
}

impl FontBook {
    /// The system's fonts, and egui's own as a last resort.
    pub fn system() -> FontBook {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        if db.is_empty() {
            for data in egui::FontDefinitions::default().font_data.values() {
                db.load_font_data(data.font.to_vec());
            }
            let family = db.faces().next().and_then(|face| face.families.first().map(|f| f.0.clone()));
            if let Some(family) = family {
                db.set_sans_serif_family(family.clone());
                db.set_serif_family(family);
            }
        }
        FontBook::with_database(db)
    }

    /// A font book over the faces of `db`.
    pub fn with_database(db: fontdb::Database) -> FontBook {
        FontBook {
            db,
            faces: Vec::new(),
            by_db: HashMap::new(),
            queries: HashMap::new(),
            fallbacks: HashMap::new(),
            fills: HashMap::new(),
            strokes: HashMap::new(),
            outlines: HashMap::new(),
            layouts: HashMap::new(),
            shaped: 0,
        }
    }

    pub fn face(&self, key: FaceKey) -> &Face {
        &self.faces[key as usize]
    }

    fn load(&mut self, id: fontdb::ID) -> Option<FaceKey> {
        if let Some(k) = self.by_db.get(&id) {
            return Some(*k);
        }
        let (data, index) = self.db.with_face_data(id, |data, index| (data.to_vec(), index))?;
        let data = Arc::new(data);
        let font = skrifa::FontRef::from_index(&data, index).ok()?;
        let m = font.metrics(Size::unscaled(), LocationRef::default());
        if !(16..=16384).contains(&m.units_per_em) {
            return None;
        }
        let upem = m.units_per_em as f64;
        let ascent = (m.ascent.round()) as f64;
        let descent = (m.descent.round()) as f64;
        let x_height = m
            .x_height
            .filter(|x| *x > 0.0)
            .map(|x| x.round() as f64)
            .unwrap_or_else(|| ((ascent - descent) * 0.45).round());
        let (underline_position, underline_thickness) = match m.underline {
            Some(u) => (u.offset.round() as f64, if u.thickness.round() > 0.0 { u.thickness.round() as f64 } else { (upem / 12.0).floor() }),
            None => (-(upem / 9.0).trunc(), (upem / 12.0).floor()),
        };
        let shaper = {
            let hr = harfrust::FontRef::from_index(&data, index).ok()?;
            harfrust::ShaperData::new(&hr)
        };
        let key = self.faces.len() as FaceKey;
        self.faces.push(Face {
            db_id: id,
            data,
            index,
            units_per_em: upem,
            ascent,
            descent,
            x_height,
            underline_position,
            underline_thickness,
            shaper,
        });
        self.by_db.insert(id, key);
        Some(key)
    }

    /// The face a `font-family` list and weight select, as usvg selects it:
    /// the first family the system has, and serif last.
    pub fn select(&mut self, families: &str, weight: u16) -> Option<FaceKey> {
        let cache_key = (families.to_string(), weight);
        if let Some(k) = self.queries.get(&cache_key) {
            return *k;
        }
        let names = parse_families(families);
        let mut list: Vec<fontdb::Family> = names
            .iter()
            .map(|n| match n.as_str() {
                "serif" => fontdb::Family::Serif,
                "sans-serif" => fontdb::Family::SansSerif,
                "monospace" => fontdb::Family::Monospace,
                "cursive" => fontdb::Family::Cursive,
                "fantasy" => fontdb::Family::Fantasy,
                other => fontdb::Family::Name(other),
            })
            .collect();
        list.push(fontdb::Family::Serif);
        let query = fontdb::Query {
            families: &list,
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let found = self.db.query(&query).or_else(|| self.db.faces().next().map(|f| f.id));
        let key = found.and_then(|id| self.load(id));
        self.queries.insert(cache_key, key);
        key
    }

    /// A face that has `c`, other than those already tried (usvg's fallback).
    fn fallback(&mut self, base: FaceKey, c: char, tried: &[FaceKey]) -> Option<FaceKey> {
        if let Some(k) = self.fallbacks.get(&(base, c)) {
            return k.filter(|k| !tried.contains(k));
        }
        let base_info = self.db.face(self.faces[base as usize].db_id)?.clone();
        let tried_ids: Vec<fontdb::ID> = tried.iter().map(|k| self.faces[*k as usize].db_id).collect();
        let mut found = None;
        let candidates: Vec<fontdb::ID> = self
            .db
            .faces()
            .filter(|f| !tried_ids.contains(&f.id))
            .filter(|f| !(base_info.style != f.style && base_info.weight != f.weight && base_info.stretch != f.stretch))
            .map(|f| f.id)
            .collect();
        for id in candidates {
            let has = self
                .db
                .with_face_data(id, |data, index| {
                    skrifa::FontRef::from_index(data, index).ok().is_some_and(|f| f.charmap().map(c).is_some())
                })
                .unwrap_or(false);
            if has {
                found = self.load(id);
                if found.is_some() {
                    break;
                }
            }
        }
        self.fallbacks.insert((base, c), found);
        found
    }

    /// Shape `text` with one face: (face, glyph id, x advance, x offset,
    /// y offset, cluster) in font units.
    fn shape_with(&self, key: FaceKey, text: &str, tabular: bool) -> Vec<RawGlyph> {
        let face = &self.faces[key as usize];
        let Ok(font) = harfrust::FontRef::from_index(&face.data, face.index) else { return Vec::new() };
        let shaper = face.shaper.shaper(&font).build();
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.set_direction(harfrust::Direction::LeftToRight);
        buffer.guess_segment_properties();
        let features = if tabular { vec![harfrust::Feature::new(harfrust::Tag::new(b"tnum"), 1, ..)] } else { Vec::new() };
        let output = shaper.shape(buffer, harfrust::ShapeOptions::new().features(&features));
        output
            .glyph_infos()
            .iter()
            .zip(output.glyph_positions())
            .map(|(info, pos)| RawGlyph {
                face: key,
                gid: info.glyph_id,
                advance: pos.x_advance as f64,
                dx: pos.x_offset as f64,
                dy: pos.y_offset as f64,
                cluster: info.cluster as usize,
            })
            .collect()
    }

    /// Shape with font fallback, as usvg does: glyphs the first face lacks are
    /// taken from the first face that has them, when it shapes to as many glyphs.
    fn shape(&mut self, key: FaceKey, text: &str, tabular: bool) -> Vec<RawGlyph> {
        let mut glyphs = self.shape_with(key, text, tabular);
        let mut tried = vec![key];
        while let Some(missing) = glyphs.iter().find(|g| g.gid == 0).map(|g| text[g.cluster..].chars().next().unwrap_or(' ')) {
            let Some(other) = self.fallback(key, missing, &tried) else { break };
            let alt = self.shape_with(other, text, tabular);
            if alt.iter().all(|g| g.gid != 0) {
                glyphs = alt;
                break;
            }
            if alt.len() != glyphs.len() {
                break;
            }
            for (g, a) in glyphs.iter_mut().zip(alt) {
                if g.gid == 0 && a.gid != 0 {
                    *g = a;
                }
            }
            tried.push(other);
        }
        glyphs
    }

    /// Lay a line of text out (once per string and setting).
    pub fn layout(&mut self, text: &str, spec: &TextSpec) -> Option<TextRun> {
        let key = LayoutKey {
            text: text.to_string(),
            families: spec.families.to_string(),
            weight: spec.weight,
            size: spec.size.to_bits(),
            letter_spacing: spec.letter_spacing.to_bits(),
            anchor: spec.anchor as u8,
            baseline: spec.baseline as u8,
            text_length: spec.text_length.map(f64::to_bits),
            tabular: spec.tabular,
        };
        if let Some(run) = self.layouts.get(&key) {
            return run.clone();
        }
        let run = self.shape_line(text, spec);
        if self.layouts.len() >= LAYOUTS_KEPT {
            self.layouts.clear();
        }
        self.layouts.insert(key, run.clone());
        self.shaped += 1;
        run
    }

    fn shape_line(&mut self, text: &str, spec: &TextSpec) -> Option<TextRun> {
        let key = self.select(spec.families, spec.weight)?;
        if text.is_empty() {
            return None;
        }
        let glyphs = self.shape(key, text, spec.tabular);
        if glyphs.is_empty() {
            return None;
        }
        // Group into clusters; letter-spacing goes between clusters.
        let mut x = 0.0;
        let mut placed: Vec<PlacedGlyph> = Vec::with_capacity(glyphs.len());
        let n = glyphs.len();
        let mut i = 0;
        let mut natural = 0.0;
        while i < n {
            let cluster = glyphs[i].cluster;
            let mut j = i;
            let mut cluster_advance = 0.0;
            while j < n && glyphs[j].cluster == cluster {
                let g = &glyphs[j];
                let face = &self.faces[g.face as usize];
                let s = spec.size / face.units_per_em;
                placed.push(PlacedGlyph { face: g.face, gid: g.gid, x: x + cluster_advance + g.dx * s, y: -g.dy * s, size: spec.size, sx: 1.0 });
                cluster_advance += g.advance * s;
                j += 1;
            }
            natural += cluster_advance;
            x += cluster_advance;
            if j < n {
                x += spec.letter_spacing;
            }
            i = j;
        }
        let mut width = x;
        if let Some(target) = spec.text_length.filter(|t| t.is_finite() && *t > 0.0) {
            // Browsers stretch the run as a whole: glyphs and the gaps between them.
            let factor = if natural > 0.0 { target / width } else { 1.0 };
            if factor >= 0.001 {
                for g in &mut placed {
                    g.x *= factor;
                    g.sx = factor;
                }
                width = target;
            }
        }
        let shift_x = match spec.anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => -width / 2.0,
            Anchor::End => -width,
        };
        let face = &self.faces[key as usize];
        let shift_y = face.baseline_shift(spec.baseline, spec.size);
        for g in &mut placed {
            g.x += shift_x;
            g.y += shift_y;
        }
        let s = spec.size / face.units_per_em;
        Some(TextRun {
            glyphs: placed,
            left: shift_x,
            width,
            top: shift_y - face.ascent * s,
            bottom: shift_y - face.descent * s,
            underline: (shift_y - face.underline_position * s, face.underline_thickness * s),
        })
    }

    /// A glyph's outline at one em, y up.
    pub fn outline(&mut self, face: FaceKey, gid: u32) -> Option<Arc<Path>> {
        if let Some(p) = self.outlines.get(&(face, gid)) {
            return p.clone();
        }
        let f = &self.faces[face as usize];
        let upem = f.units_per_em as f32;
        let path = f.skrifa().and_then(|font| {
            let glyph = font.outline_glyphs().get(skrifa::GlyphId::new(gid))?;
            let mut pen = LyonPen { b: Path::builder(), open: false, scale: 1.0 / upem, any: false };
            glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
            if pen.open {
                pen.b.end(true);
            }
            pen.any.then(|| Arc::new(pen.b.build()))
        });
        self.outlines.insert((face, gid), path.clone());
        path
    }

    /// A glyph filled, at one em, y up, at a level of detail.
    pub fn glyph_fill(&mut self, face: FaceKey, gid: u32, level: usize) -> Option<Arc<MeshBuf>> {
        let level = level.min(2);
        if let Some(m) = self.fills.get(&(face, gid, level)) {
            return m.clone();
        }
        let mesh = self.outline(face, gid).map(|path| {
            let mut out = MeshBuf::default();
            tess::fill(&path, egui::Color32::WHITE, 0, GLYPH_TOLERANCE[level], &mut out);
            Arc::new(out)
        });
        self.fills.insert((face, gid, level), mesh.clone());
        mesh
    }

    /// A glyph's outline stroked `width_em` wide (quantised to 1/4096 em), at
    /// one em, y up.
    /// At the coarsest level round joins become bevels: at a few pixels the
    /// difference cannot be seen and the vertices cost.
    pub fn glyph_stroke(&mut self, face: FaceKey, gid: u32, width_em: f64, join: LineJoin, level: usize) -> Option<Arc<MeshBuf>> {
        let level = level.min(2);
        let q = (width_em * 4096.0).round().clamp(1.0, u32::MAX as f64) as u32;
        let join = if level == 2 && join == LineJoin::Round { LineJoin::Bevel } else { join };
        let jk = join as u8;
        if let Some(m) = self.strokes.get(&(face, gid, q, jk, level)) {
            return m.clone();
        }
        let mesh = self.outline(face, gid).map(|path| {
            let mut out = MeshBuf::default();
            let spec = StrokeSpec { width: q as f32 / 4096.0, join, ..StrokeSpec::solid(0.0) };
            tess::stroke(&path, &spec, egui::Color32::WHITE, 0, GLYPH_TOLERANCE[level], &mut out);
            Arc::new(out)
        });
        self.strokes.insert((face, gid, q, jk, level), mesh.clone());
        mesh
    }

    /// Whether a face has a character (for tests and diagnostics).
    pub fn face_has_char(&self, face: FaceKey, c: char) -> bool {
        self.faces[face as usize].has_char(c)
    }

    /// The family name of a face.
    pub fn family_name(&self, face: FaceKey) -> String {
        self.db
            .face(self.faces[face as usize].db_id)
            .and_then(|f| f.families.first().map(|f| f.0.clone()))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy)]
struct RawGlyph {
    face: FaceKey,
    gid: u32,
    advance: f64,
    dx: f64,
    dy: f64,
    cluster: usize,
}

struct LyonPen {
    b: lyon_tessellation::path::path::Builder,
    open: bool,
    scale: f32,
    any: bool,
}

impl OutlinePen for LyonPen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.b.end(true);
        }
        self.b.begin(point(x * self.scale, y * self.scale));
        self.open = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.b.line_to(point(x * self.scale, y * self.scale));
            self.any = true;
        }
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        if self.open {
            let s = self.scale;
            self.b.quadratic_bezier_to(point(cx0 * s, cy0 * s), point(x * s, y * s));
            self.any = true;
        }
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if self.open {
            let s = self.scale;
            self.b.cubic_bezier_to(point(cx0 * s, cy0 * s), point(cx1 * s, cy1 * s), point(x * s, y * s));
            self.any = true;
        }
    }
    fn close(&mut self) {
        if self.open {
            self.b.end(true);
            self.open = false;
        }
    }
}

/// A CSS `font-family` list: names unquoted, generic names lower-cased.
pub fn parse_families(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in list.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ',' => {
                out.push(std::mem::take(&mut cur));
            }
            None => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            if matches!(lower.as_str(), "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy") { lower } else { s }
        })
        .collect()
}

/// SVG's default white-space handling: new lines dropped, tabs made spaces,
/// runs of spaces collapsed, the ends trimmed.
pub fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        let c = match c {
            '\n' | '\r' => continue,
            '\t' => ' ',
            c => c,
        };
        if c == ' ' {
            if !space && !out.is_empty() {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_parse() {
        assert_eq!(
            parse_families(r#""Inter", "Segoe UI", system-ui, -apple-system, Sans-Serif"#),
            vec!["Inter", "Segoe UI", "system-ui", "-apple-system", "sans-serif"]
        );
        assert_eq!(collapse_whitespace("  a \n b\t\tc  "), "a b c");
    }

    #[test]
    fn a_label_is_laid_out() {
        let mut book = font_book();
        let spec = TextSpec {
            families: r#""Inter", "Segoe UI", system-ui, sans-serif"#,
            weight: 600,
            size: 12.5,
            letter_spacing: 0.0,
            anchor: Anchor::Middle,
            baseline: Baseline::Central,
            text_length: None,
            tabular: false,
        };
        let run = book.layout("glc__D", &spec).expect("a font");
        assert_eq!(run.glyphs.len(), 6);
        assert!(run.width > 20.0 && run.width < 60.0, "{}", run.width);
        assert!((run.left + run.width / 2.0).abs() < 1e-9, "centred");
        assert!(run.top < 0.0 && run.bottom > 0.0);
        let g = run.glyphs[0];
        assert!(book.glyph_fill(g.face, g.gid, 0).is_some_and(|m| !m.is_empty()));
        assert!(book.glyph_stroke(g.face, g.gid, 3.2 / 12.5, LineJoin::Round, 1).is_some_and(|m| !m.is_empty()));
        let stretched = book.layout("glc__D", &TextSpec { text_length: Some(100.0), anchor: Anchor::Start, ..spec }).unwrap();
        assert!((stretched.width - 100.0).abs() < 1e-9);
    }
}
