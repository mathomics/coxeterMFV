//! The map as a file: the SVG the app exports and the PNG made from it
//! (`exportableSVG`, `exportPNG` and `rasterize` in `src/ui/app.js`).
//!
//! The exported SVG is drawn afresh rather than copied from the screen, so the
//! file holds the map and not the editor: no selection, handles, pointer
//! targets or resize grips, and no zoom-dependent hiding of labels. The
//! stylesheet rules that style the map go in with it, in a `<style>` element.
//!
//! The PNG is that picture rasterised by resvg. resvg reads neither `var()`
//! nor `calc()`, so the styles are worked out first ([`crate::css`]) and
//! written onto each element as plain presentation attributes
//! ([`inline_styles`]).

use std::sync::{Arc, LazyLock};

use coxeter_core::js;
use coxeter_core::mapdata::MapGraph;
use regex::Regex;
use resvg::tiny_skia;
use resvg::usvg;

use crate::css::{APP_CSS, MAP_CSS, PROPERTIES, ResolveOptions, StyleSheet};
use crate::mapview::render_map;
use crate::raster_size::{PREFERRED_SCALE, raster_scales};
use crate::state::RenderState;
use crate::svg::{Element, Node};

/// Extra content of an exported SVG.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Text of a `<metadata>` element put first in the SVG (the Zuñiga study
    /// writes its scenario there as JSON).
    pub metadata: Option<String>,
    /// What the file says about itself (`exportMetadata()`, as JSON): a
    /// `<metadata>` element before the study's.
    pub about: Option<String>,
    /// The file's `<title>`, first of all (the scenario's name).
    pub title: Option<String>,
}

/// The stylesheets of the page, in document order (app.css, then map.css).
static PAGE_SHEETS: LazyLock<(StyleSheet, StyleSheet)> = LazyLock::new(|| (StyleSheet::parse(APP_CSS), StyleSheet::parse(MAP_CSS)));

/// The rules an exported SVG carries: every rule of the page's stylesheets
/// that mentions `.emc-`, as `exportableSVG` collects them.
pub fn export_css() -> String {
    let (app, map) = &*PAGE_SHEETS;
    let mut rules = app.rules_text_containing(".emc-");
    rules.extend(map.rules_text_containing(".emc-"));
    rules.join("\n")
}

/// The stylesheet an exported SVG is styled by once opened on its own.
pub fn export_stylesheet() -> &'static StyleSheet {
    static SHEET: LazyLock<StyleSheet> = LazyLock::new(|| StyleSheet::parse(&export_css()));
    &SHEET
}

/// The map drawn for export, with its `<style>` (and metadata) in front:
/// the element tree of the exported file.
pub fn exportable_tree(map: &MapGraph, state: &RenderState, opts: &ExportOptions) -> Element {
    let state = RenderState { for_export: true, ..state.clone() };
    let mut svg = render_map(map, &state);
    svg.children.insert(0, Node::Element(Element::paired("style").text(export_css())));
    if let Some(meta) = &opts.metadata {
        svg.children.insert(0, Node::Element(Element::paired("metadata").text(meta.clone())));
    }
    if let Some(about) = &opts.about {
        svg.children.insert(0, Node::Element(Element::paired("metadata").text(about.clone())));
    }
    if let Some(title) = &opts.title {
        svg.children.insert(0, Node::Element(Element::paired("title").text(title.clone())));
    }
    svg
}

/// `exportableSVG()`: the SVG file the app exports, as `XMLSerializer` writes
/// it after the browser has parsed the renderer's markup.
pub fn exportable_svg(map: &MapGraph, state: &RenderState, opts: &ExportOptions) -> String {
    exportable_tree(map, state, opts).to_xml()
}

/// How styles are written onto the elements for a renderer without CSS.
#[derive(Debug, Clone)]
pub struct InlineOptions<'a> {
    pub resolve: ResolveOptions<'a>,
    /// Device pixels per map unit of the picture, for strokes the stylesheet
    /// asks not to scale (`vector-effect: non-scaling-stroke`), which resvg
    /// does not support: their width is divided by it.
    pub device_scale: f64,
}

impl Default for InlineOptions<'_> {
    fn default() -> Self {
        InlineOptions { resolve: ResolveOptions::default(), device_scale: 1.0 }
    }
}

/// Properties written as attributes; the rest only matter to a browser.
fn inlined(name: &str) -> bool {
    !matches!(name, "color" | "pointer-events" | "text-decoration" | "font-variant-numeric" | "text-transform" | "vector-effect")
}

static TRANSFORM_UNITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(-?[\d.]+(?:e[-+]?\d+)?)(px|deg)").unwrap());

/// A copy of `tree` whose every element carries its computed style as plain
/// presentation attributes, with no `style` attributes and no `<style>`:
/// what resvg (or any renderer without CSS) needs to draw what a browser
/// draws. Only what differs from what the element would inherit anyway is
/// written, so the file stays small.
pub fn inline_styles(tree: &Element, sheet: &StyleSheet, opts: &InlineOptions) -> Element {
    let styles = sheet.resolve(tree, &opts.resolve);
    let mut k = 0;
    inline_element(tree, None, &styles, &mut k, opts)
}

fn inline_element(
    el: &Element,
    parent: Option<&crate::css::ComputedStyle>,
    styles: &[crate::css::ComputedStyle],
    k: &mut usize,
    opts: &InlineOptions,
) -> Element {
    let style = &styles[*k];
    *k += 1;
    let mut out = Element { name: el.name, attrs: Vec::new(), children: Vec::new(), close_ws: "", paired: el.paired };
    for a in &el.attrs {
        let presentation = PROPERTIES.iter().any(|p| p.attribute && p.name == a.name);
        if !presentation && a.name != "style" {
            out.attrs.push(crate::svg::Attr { lead: " ", name: a.name, value: Some(a.value.clone().unwrap_or_default()) });
        }
    }
    let non_scaling = style.get("vector-effect") == "non-scaling-stroke";
    for p in PROPERTIES.iter() {
        // A pattern keeps its own patternTransform attribute.
        if !inlined(p.name) || (p.name == "transform" && matches!(el.name, "pattern" | "linearGradient" | "radialGradient")) {
            continue;
        }
        let value = style.get(p.name);
        let write = match parent {
            // The root states every inherited value: resvg's defaults are not a browser's.
            None => p.inherited || value != p.initial,
            Some(ps) if p.inherited => value != ps.get(p.name) || (p.name == "stroke-width" && non_scaling),
            Some(_) => value != p.initial,
        };
        if !write {
            continue;
        }
        // Lengths are written as SVG writes them: user units, no `px`.
        let length = |v: &str| {
            let px = crate::css::parse_length(v, style.font_size());
            if px.is_nan() { v.to_string() } else { js::number_to_string(px) }
        };
        let text = match p.name {
            "font-size" => js::number_to_string(style.font_size()),
            "stroke-width" if non_scaling && opts.device_scale > 0.0 => js::number_to_string(style.stroke_width() / opts.device_scale),
            "stroke-width" => length(value),
            "letter-spacing" if value == "normal" => "0".to_string(),
            "letter-spacing" => length(value),
            "transform" => TRANSFORM_UNITS.replace_all(value, "$1").into_owned(),
            _ => value.to_string(),
        };
        out.attrs.push(crate::svg::Attr { lead: " ", name: p.name, value: Some(text.into()) });
    }
    let case = style.text_transform();
    for c in &el.children {
        match c {
            Node::Element(e) if e.name == "style" => {
                *k += count_elements(e);
            }
            Node::Element(e) => out.children.push(Node::Element(inline_element(e, Some(style), styles, k, opts))),
            Node::Text(t) => out.children.push(Node::Text(match case {
                "uppercase" => t.to_uppercase().into(),
                "lowercase" => t.to_lowercase().into(),
                _ => t.clone(),
            })),
        }
    }
    out
}

fn count_elements(e: &Element) -> usize {
    1 + e.elements().map(count_elements).sum::<usize>()
}

/// Why a PNG could not be made.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExportError {
    #[error("the map is too large to rasterize. Export the SVG instead; it keeps full resolution.")]
    TooLarge,
    #[error("The SVG could not be rasterized: {0}")]
    Svg(String),
    #[error("The PNG could not be written: {0}")]
    Encode(String),
}

/// A PNG of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct PngExport {
    pub png: Vec<u8>,
    /// The scale it was made at; below [`PREFERRED_SCALE`] when the map is too
    /// large for that (the app then says so).
    pub scale: f64,
    pub width: u32,
    pub height: u32,
}

/// The system fonts, read once.
fn fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    });
    DB.clone()
}

/// A pixmap of this size, or `None` when its memory cannot be had.
fn pixmap(width: u32, height: u32) -> Option<tiny_skia::Pixmap> {
    let size = tiny_skia::IntSize::from_wh(width, height)?;
    let len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    let mut data: Vec<u8> = Vec::new();
    data.try_reserve_exact(len).ok()?;
    data.resize(len, 0);
    tiny_skia::Pixmap::from_vec(data, size)
}

/// `exportPNG()`: the exported SVG rasterised at the largest scale (up to 2×)
/// in the ladder of [`raster_scales`] that can be made, on the canvas colour
/// of the scheme.
pub fn export_png(map: &MapGraph, state: &RenderState) -> Result<PngExport, ExportError> {
    let tree = exportable_tree(map, state, &ExportOptions::default());
    // `image.naturalWidth || store.map.width`: the size the SVG states.
    let stated = |name: &str| tree.get(name).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
    let width = stated("width").unwrap_or(map.width());
    let height = stated("height").unwrap_or(map.height());
    let background = if state.dark { tiny_skia::Color::from_rgba8(0x0b, 0x30, 0x46, 0xff) } else { tiny_skia::Color::WHITE };
    for scale in raster_scales(width, height, PREFERRED_SCALE) {
        let pw = js::max(1.0, js::round(width * scale)) as u32;
        let ph = js::max(1.0, js::round(height * scale)) as u32;
        let Some(mut canvas) = pixmap(pw, ph) else { continue };
        canvas.fill(background);
        let sx = pw as f64 / width;
        let sy = ph as f64 / height;
        let inlined = inline_styles(&tree, export_stylesheet(), &InlineOptions { device_scale: sx.min(sy), ..Default::default() });
        let svg = inlined.to_xml();
        let opt = usvg::Options { fontdb: fontdb(), font_family: "DejaVu Sans".into(), ..usvg::Options::default() };
        let parsed = usvg::Tree::from_str(&svg, &opt).map_err(|e| ExportError::Svg(e.to_string()))?;
        resvg::render(&parsed, tiny_skia::Transform::from_scale(sx as f32, sy as f32), &mut canvas.as_mut());
        let png = canvas.encode_png().map_err(|e| ExportError::Encode(e.to_string()))?;
        return Ok(PngExport { png, scale, width: pw, height: ph });
    }
    Err(ExportError::TooLarge)
}
