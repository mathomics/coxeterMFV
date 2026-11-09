//! The paint style of each element, as the painter and the hit-tester need it:
//! the computed style `coxeter_render::css` works out from `assets/map.css`
//! (what the browser does for the SVG on the stage), boiled down to numbers.
//!
//! The stylesheet depends on the zoom through three things `applyCamera()`
//! sets on the map: `--label-scale` (label sizes), `--frame-label-scale` (the
//! name plates' `transform: scale(…)`) and the `is-far` class (small type
//! hidden). Rather than run the cascade again at every zoom, the tree is
//! resolved twice (at the stylesheet's defaults and at other values), and
//! each element is told how it depends on the zoom ([`ZoomDep`]).

use std::collections::HashMap;
use std::sync::LazyLock;

use coxeter_core::js;
use coxeter_render::css::{ComputedStyle, Paint as CssPaint, ResolveOptions, Rgba, map_stylesheet};
use coxeter_render::Element;

use crate::camera::ZoomVars;

/// How a shape is filled or stroked.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    None,
    Color(Rgba),
    /// A paint server: `url(#emc-wall)` is the only one the map uses.
    Url(String),
}

impl Paint {
    pub fn is_none(&self) -> bool {
        matches!(self, Paint::None)
    }
}

/// `stroke-linecap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// `stroke-linejoin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// `text-anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Start,
    Middle,
    End,
}

/// `dominant-baseline`, as usvg (and Chrome) read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Baseline {
    #[default]
    Auto,
    Alphabetic,
    Central,
    Middle,
    Hanging,
    TextBeforeEdge,
    TextAfterEdge,
    Ideographic,
    Mathematical,
}

/// `pointer-events`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerEvents {
    /// `auto` / `visiblePainted`, the SVG default.
    #[default]
    VisiblePainted,
    VisibleFill,
    VisibleStroke,
    Visible,
    Painted,
    Fill,
    Stroke,
    All,
    None,
}

/// The computed style of one element, as numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub fill: Paint,
    pub stroke: Paint,
    pub stroke_width: f64,
    pub fill_opacity: f64,
    pub stroke_opacity: f64,
    /// The element's own `opacity` (not multiplied by its ancestors').
    pub opacity: f64,
    pub dash: Option<Vec<f64>>,
    pub linecap: LineCap,
    pub linejoin: LineJoin,
    pub font_size: f64,
    pub font_weight: u16,
    pub font_family: String,
    /// In px (0 for `normal`).
    pub letter_spacing: f64,
    pub tabular_nums: bool,
    pub displayed: bool,
    pub visible: bool,
    pub anchor: Anchor,
    pub baseline: Baseline,
    /// `text-transform`: `uppercase`, `lowercase` or none.
    pub text_transform: TextTransform,
    /// `paint-order: stroke`: the stroke goes under the fill.
    pub stroke_first: bool,
    /// `filter: url(#id)`, the id.
    pub filter: Option<String>,
    /// The CSS `transform` (or the `transform` attribute), as a matrix.
    pub transform: Option<[f64; 6]>,
    pub non_scaling_stroke: bool,
    pub pointer_events: PointerEvents,
    pub underline: bool,
}

/// `text-transform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: Paint::Color(Rgba { r: 0, g: 0, b: 0, a: 1.0 }),
            stroke: Paint::None,
            stroke_width: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            opacity: 1.0,
            dash: None,
            linecap: LineCap::Butt,
            linejoin: LineJoin::Miter,
            font_size: 16.0,
            font_weight: 400,
            font_family: "serif".into(),
            letter_spacing: 0.0,
            tabular_nums: false,
            displayed: true,
            visible: true,
            anchor: Anchor::Start,
            baseline: Baseline::Auto,
            text_transform: TextTransform::None,
            stroke_first: false,
            filter: None,
            transform: None,
            non_scaling_stroke: false,
            pointer_events: PointerEvents::VisiblePainted,
            underline: false,
        }
    }
}

fn paint(p: CssPaint) -> Paint {
    match p {
        CssPaint::None => Paint::None,
        CssPaint::Color(c) => Paint::Color(c),
        CssPaint::Url(u) => Paint::Url(u.trim_start_matches('#').to_string()),
    }
}

impl Style {
    /// Read a computed style.
    pub fn from_computed(cs: &ComputedStyle) -> Style {
        let dash = cs.stroke_dasharray().and_then(|d| {
            // An all-zero, negative or invalid pattern draws a solid line.
            if d.is_empty() || d.iter().any(|v| !v.is_finite() || *v < 0.0) || d.iter().all(|v| *v == 0.0) {
                None
            } else if d.len() % 2 == 1 {
                // An odd count is repeated to make it even.
                Some(d.iter().chain(d.iter()).copied().collect())
            } else {
                Some(d)
            }
        });
        let letter_spacing = match cs.get("letter-spacing") {
            "normal" | "" => 0.0,
            v => {
                let px = coxeter_render::css::parse_length(v, cs.font_size());
                if px.is_finite() { px } else { 0.0 }
            }
        };
        let filter = match cs.get("filter") {
            "none" | "" => None,
            v => {
                let v = v.trim();
                v.strip_prefix("url(")
                    .and_then(|r| r.strip_suffix(')'))
                    .map(|r| r.trim().trim_matches(|c| c == '"' || c == '\'').trim_start_matches('#').to_string())
            }
        };
        let transform = cs.transform().and_then(|_| cs.transform_matrix()).filter(|m| *m != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        Style {
            fill: paint(cs.fill()),
            stroke: paint(cs.stroke()),
            stroke_width: {
                let w = cs.stroke_width();
                if w.is_finite() && w >= 0.0 { w } else { 1.0 }
            },
            fill_opacity: cs.fill_opacity().clamp(0.0, 1.0),
            stroke_opacity: cs.stroke_opacity().clamp(0.0, 1.0),
            opacity: cs.opacity().clamp(0.0, 1.0),
            dash,
            linecap: match cs.get("stroke-linecap") {
                "round" => LineCap::Round,
                "square" => LineCap::Square,
                _ => LineCap::Butt,
            },
            linejoin: match cs.get("stroke-linejoin") {
                "round" => LineJoin::Round,
                "bevel" => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            font_size: cs.font_size(),
            font_weight: cs.font_weight(),
            font_family: cs.font_family().to_string(),
            letter_spacing,
            tabular_nums: cs.get("font-variant-numeric").contains("tabular-nums"),
            displayed: cs.is_displayed(),
            visible: cs.is_visible(),
            anchor: match cs.text_anchor() {
                "middle" => Anchor::Middle,
                "end" => Anchor::End,
                _ => Anchor::Start,
            },
            baseline: match cs.dominant_baseline() {
                "alphabetic" => Baseline::Alphabetic,
                "central" => Baseline::Central,
                "middle" => Baseline::Middle,
                "hanging" => Baseline::Hanging,
                "text-before-edge" | "text-top" => Baseline::TextBeforeEdge,
                "text-after-edge" | "text-bottom" => Baseline::TextAfterEdge,
                "ideographic" => Baseline::Ideographic,
                "mathematical" => Baseline::Mathematical,
                _ => Baseline::Auto,
            },
            text_transform: match cs.text_transform() {
                "uppercase" => TextTransform::Uppercase,
                "lowercase" => TextTransform::Lowercase,
                _ => TextTransform::None,
            },
            stroke_first: cs.get("paint-order").trim_start().starts_with("stroke"),
            filter,
            transform,
            non_scaling_stroke: cs.get("vector-effect") == "non-scaling-stroke",
            pointer_events: match cs.get("pointer-events") {
                "none" => PointerEvents::None,
                "visibleFill" => PointerEvents::VisibleFill,
                "visibleStroke" => PointerEvents::VisibleStroke,
                "visible" => PointerEvents::Visible,
                "painted" => PointerEvents::Painted,
                "fill" => PointerEvents::Fill,
                "stroke" => PointerEvents::Stroke,
                "all" => PointerEvents::All,
                _ => PointerEvents::VisiblePainted,
            },
            underline: cs.get("text-decoration").contains("underline"),
        }
    }

    /// Whether the fill takes part in hit-testing (`pointer-events`).
    pub fn hits_fill(&self) -> bool {
        match self.pointer_events {
            PointerEvents::None | PointerEvents::Stroke | PointerEvents::VisibleStroke => false,
            PointerEvents::VisiblePainted => self.visible && !self.fill.is_none(),
            PointerEvents::Painted => !self.fill.is_none(),
            PointerEvents::VisibleFill | PointerEvents::Visible => self.visible,
            PointerEvents::Fill | PointerEvents::All => true,
        }
    }

    /// Whether the stroke takes part in hit-testing (`pointer-events`).
    pub fn hits_stroke(&self) -> bool {
        match self.pointer_events {
            PointerEvents::None | PointerEvents::Fill | PointerEvents::VisibleFill => false,
            PointerEvents::VisiblePainted => self.visible && !self.stroke.is_none(),
            PointerEvents::Painted => !self.stroke.is_none(),
            PointerEvents::VisibleStroke | PointerEvents::Visible => self.visible,
            PointerEvents::Stroke | PointerEvents::All => true,
        }
    }
}

/// The page around the map: what the map inherits from the stage it is drawn
/// on (`color`, the font size, and the app's `--accent` and `--ink`), in
/// light or dark (the values of `assets/app.css`).
#[derive(Debug, Clone, PartialEq)]
pub struct PageTheme {
    pub root_color: String,
    pub root_font_size: f64,
    pub custom: Vec<(String, String)>,
}

impl PageTheme {
    pub fn light() -> PageTheme {
        PageTheme {
            root_color: "#0f172a".into(),
            root_font_size: 13.0,
            custom: vec![("--accent".into(), "#2563eb".into()), ("--ink".into(), "#0f172a".into())],
        }
    }

    pub fn dark() -> PageTheme {
        PageTheme {
            root_color: "#ecf1f8".into(),
            root_font_size: 13.0,
            custom: vec![("--accent".into(), "#60a5fa".into()), ("--ink".into(), "#ecf1f8".into())],
        }
    }

    /// An exported SVG opened on its own: no page around it (the stylesheet's
    /// fallbacks apply), what resvg sees.
    pub fn standalone() -> PageTheme {
        PageTheme { root_color: "#000000".into(), root_font_size: 16.0, custom: Vec::new() }
    }

    pub fn for_dark(dark: bool) -> PageTheme {
        if dark { PageTheme::dark() } else { PageTheme::light() }
    }

    /// The resolver's options at a zoom.
    pub fn options<'a>(&self, zoom: ZoomVars, hovered: Option<&'a Element>) -> ResolveOptions<'a> {
        ResolveOptions {
            label_scale: Some(zoom.label_scale),
            frame_label_scale: Some(zoom.frame_label_scale),
            is_far: zoom.is_far,
            hovered,
            root_color: self.root_color.clone(),
            root_font_size: self.root_font_size,
            custom: self.custom.clone(),
        }
    }
}

/// The zoom values the second resolve is made at: inside the range the app
/// uses, so a plate that reaches its own `--fit` is told apart from one that
/// does not.
pub fn probe_zoom() -> ZoomVars {
    ZoomVars { label_scale: 2.0, frame_label_scale: 2.0, is_far: true }
}

/// How an element's style depends on the zoom.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ZoomDep {
    /// Not at all.
    #[default]
    None,
    /// Only through `--label-scale` (its font size, linear in it) and/or
    /// `is-far` (whether it is displayed).
    Label {
        /// Font size at label scale 1 and at 2: `fs(ls) = a + (b − a)(ls − 1)`.
        font: Option<(f64, f64)>,
        /// Displayed when near (`false`: only when far), and when far.
        display: Option<(bool, bool)>,
    },
    /// Only through a uniform `transform: scale(…)` (a name plate). The scale
    /// at a zoom is worked out exactly, for these few elements.
    Scale,
    /// Some other way: styled and drawn again whenever the zoom changes.
    Other,
}

/// Compare an element's style at the defaults with its style at the probe.
pub fn zoom_dependence(base: &Style, probe: &Style) -> ZoomDep {
    if base == probe {
        return ZoomDep::None;
    }
    let mut a = probe.clone();
    // Font size and display only?
    a.font_size = base.font_size;
    a.displayed = base.displayed;
    a.letter_spacing = base.letter_spacing;
    if a == *base {
        let font = (probe.font_size != base.font_size).then_some((base.font_size, probe.font_size));
        let display = (probe.displayed != base.displayed).then_some((base.displayed, probe.displayed));
        // letter-spacing in em follows the font size; the em-relative part
        // scales with it, which is what the group scaling does.
        return ZoomDep::Label { font, display };
    }
    let mut b = probe.clone();
    b.transform = base.transform;
    if b == *base && uniform_scale(base.transform).is_some() && uniform_scale(probe.transform).is_some() {
        return ZoomDep::Scale;
    }
    ZoomDep::Other
}

/// The factor of a transform that is a uniform scale about the origin.
pub fn uniform_scale(m: Option<[f64; 6]>) -> Option<f64> {
    let m = m.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    (m[1] == 0.0 && m[2] == 0.0 && m[4] == 0.0 && m[5] == 0.0 && m[0] == m[3]).then_some(m[0])
}

/// Whether an element is drawn at all at a zoom (`display`), given its
/// dependence.
pub fn displayed_at(style: &Style, dep: &ZoomDep, zoom: &ZoomVars) -> bool {
    match dep {
        ZoomDep::Label { display: Some((near, far)), .. } => {
            if zoom.is_far {
                *far
            } else {
                *near
            }
        }
        _ => style.displayed,
    }
}

/// The font size at a zoom, given the element's dependence.
pub fn font_size_at(style: &Style, dep: &ZoomDep, zoom: &ZoomVars) -> f64 {
    match dep {
        ZoomDep::Label { font: Some((a, b)), .. } => a + (b - a) * (zoom.label_scale - 1.0),
        _ => style.font_size,
    }
}

/// Resolve every element of the tree, in document order.
///
/// The cascade is memoised: an element's computed style is a function of its
/// parent's inherited values, the tags, classes, ids and selector-tested
/// attributes of it and its ancestors, and its own presentation attributes
/// and `style`, not of its geometry. Elements alike in all of that (most of
/// a map: every segment of one colour, every node, every label of one kind)
/// share one cascade. Each new case is computed by the resolver itself
/// (`computed_style` on the element's path), so the answer is the resolver's.
pub fn resolve_all(root: &Element, theme: &PageTheme, zoom: ZoomVars, hovered: Option<&Element>) -> Vec<Style> {
    let opts = theme.options(zoom, hovered);
    if hovered.is_some() || *SIBLING_SENSITIVE {
        return map_stylesheet().resolve(root, &opts).iter().map(Style::from_computed).collect();
    }
    let mut memo = Memo::default();
    let mut out = Vec::with_capacity(1024);
    let mut path: Vec<&Element> = Vec::with_capacity(16);
    memo.walk(root, &mut path, 0, 0, &opts, &mut out);
    out
}

/// The same, without memoising (the resolver's own `resolve`), for tests.
pub fn resolve_all_plain(root: &Element, theme: &PageTheme, zoom: ZoomVars) -> Vec<Style> {
    let opts = theme.options(zoom, None);
    map_stylesheet().resolve(root, &opts).iter().map(Style::from_computed).collect()
}

/// Whether some selector of the stylesheet tests an attribute: if none does,
/// a change of it leaves every computed style as it was (bar what the
/// attribute itself presents, such as `transform`).
pub fn selectors_test(name: &str) -> bool {
    SELECTOR_ATTRS.iter().any(|a| a == name)
}

/// Attribute names some selector of the stylesheet tests.
static SELECTOR_ATTRS: LazyLock<Vec<String>> = LazyLock::new(|| {
    use coxeter_render::css::{Compound, Pseudo, Rule};
    fn collect(c: &Compound, out: &mut Vec<String>) {
        for (name, _) in &c.attrs {
            out.push(name.clone());
        }
        for p in &c.pseudos {
            if let Pseudo::Not(list) | Pseudo::Is(list, _) = p {
                for x in list {
                    collect(x, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for rule in &map_stylesheet().rules {
        if let Rule::Style(r) = rule {
            for sel in &r.selectors {
                for (_, c) in &sel.parts {
                    collect(c, &mut out);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
});

/// Whether an element's style can depend on its siblings ([`SIBLING_SENSITIVE`]):
/// then a tree cut down to what is in view is not styled as the whole is.
pub fn sibling_sensitive() -> bool {
    *SIBLING_SENSITIVE
}

/// Whether a rule that can match without `:hover` looks at siblings (`+`, `~`):
/// then an element's style depends on its siblings and is not memoised.
static SIBLING_SENSITIVE: LazyLock<bool> = LazyLock::new(|| {
    use coxeter_render::css::{Combinator, Pseudo, Rule};
    map_stylesheet().rules.iter().any(|rule| match rule {
        Rule::Style(r) => r.selectors.iter().any(|sel| {
            let hover = sel.parts.iter().any(|(_, c)| c.pseudos.iter().any(|p| matches!(p, Pseudo::Hover)));
            !hover && sel.parts.iter().any(|(comb, _)| matches!(comb, Combinator::Adjacent | Combinator::Sibling))
        }),
        Rule::At(_) => false,
    })
});

/// Attributes that are CSS properties (presentation attributes), and `style`.
fn is_presentation(name: &str) -> bool {
    coxeter_render::css::PROPERTIES.iter().any(|p| p.attribute && p.name == name)
        || matches!(name, "style" | "patternTransform" | "gradientTransform")
}

#[derive(Default)]
struct Memo {
    /// Interned strings: match chains and inherited-value sets.
    ids: HashMap<String, usize>,
    /// Cascade key → computed style and the id of what it hands down.
    styles: HashMap<String, (Style, usize)>,
}

impl Memo {
    fn intern(&mut self, key: String) -> usize {
        let n = self.ids.len();
        *self.ids.entry(key).or_insert(n + 1)
    }

    fn walk<'a>(
        &mut self,
        e: &'a Element,
        path: &mut Vec<&'a Element>,
        parent_match: usize,
        parent_inherit: usize,
        opts: &ResolveOptions,
        out: &mut Vec<Style>,
    ) {
        use std::fmt::Write;
        path.push(e);
        // What selectors can see of the element (and, through the parent's
        // id, of its ancestors).
        let mut m = String::with_capacity(64);
        let _ = write!(m, "{parent_match}/{}", e.name);
        for c in e.classes() {
            m.push('.');
            m.push_str(c);
        }
        if let Some(id) = e.get("id") {
            m.push('#');
            m.push_str(id);
        }
        for name in SELECTOR_ATTRS.iter() {
            if let Some(v) = e.get(name) {
                let _ = write!(m, "[{name}={v}]");
            }
        }
        let match_id = self.intern(m);
        let mut key = String::with_capacity(64);
        let _ = write!(key, "{parent_inherit}|{match_id}");
        for a in &e.attrs {
            if is_presentation(a.name) {
                let _ = write!(key, "|{}={}", a.name, a.value.as_deref().unwrap_or(""));
            }
        }
        let (style, inherit) = match self.styles.get(&key) {
            Some(v) => v.clone(),
            None => {
                let cs = map_stylesheet().computed_style(path, opts);
                let mut handed = String::with_capacity(256);
                for (p, (name, value)) in coxeter_render::css::PROPERTIES.iter().zip(cs.iter()) {
                    if p.inherited {
                        let _ = write!(handed, "{name}:{value};");
                    }
                }
                for (k, v) in &cs.custom {
                    let _ = write!(handed, "{k}:{v};");
                }
                let inherit = self.intern(handed);
                let v = (Style::from_computed(&cs), inherit);
                self.styles.insert(key, v.clone());
                v
            }
        };
        out.push(style);
        for c in e.elements() {
            self.walk(c, path, match_id, inherit, opts, out);
        }
        path.pop();
    }
}

/// Resolve one element by its path from the root.
pub fn resolve_one(path: &[&Element], theme: &PageTheme, zoom: ZoomVars, hovered: Option<&Element>) -> Style {
    let opts = theme.options(zoom, hovered);
    Style::from_computed(&map_stylesheet().computed_style(path, &opts))
}

/// `rgba` with an extra alpha factor, as egui's premultiplied colour.
pub fn color32(c: Rgba, alpha: f64) -> egui::Color32 {
    let a = (c.a as f64 * alpha).clamp(0.0, 1.0);
    let a8 = js::round(a * 255.0).clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, a8)
}
