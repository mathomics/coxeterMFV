//! The map as the painter and the hit-tester see it: the SVG element tree of
//! `render_map` (what `paintMap()` puts on the stage), styled by
//! `assets/map.css`, every element tessellated once into a mesh in map
//! coordinates, with its hit geometry.
//!
//! What depends on the zoom is kept apart so that a zoom redraws without
//! rebuilding (`applyCamera()` of `app.js` does the same to the DOM):
//!
//! - labels sized by `--label-scale` are tessellated at their base size and
//!   scaled about their anchor at draw time ([`GroupKind::Font`]); their halo
//!   keeps its width;
//! - name plates scaled by `--frame-label-scale` are scaled whole about their
//!   corner ([`GroupKind::Scale`]);
//! - the small type `is-far` hides is drawn or not by a flag per element;
//! - the edit handles that `updateEditHandles()` keeps a constant size on
//!   screen are scaled about their centre ([`GroupKind::Handle`]);
//! - the few elements whose stroke does not scale (`vector-effect:
//!   non-scaling-stroke`) or whose style depends on the zoom some other way
//!   are styled and tessellated again when the zoom changes.
//!
//! [`Scene::update`] takes a new tree and redoes only the elements that
//! changed (and what is under them), which is what keeps a drag cheap. An
//! element that only moved (the same element, every number that places it
//! shifted by one delta) is not tessellated again: its meshes and hit
//! geometry stay as they were made and it is drawn and hit-tested `shift`
//! away ([`ElemData::shift`]), so dragging a compartment with hundreds of
//! metabolites, reactions and labels costs about what dragging one does.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, mpsc};

use coxeter_render::Element;
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Event, Path};

use crate::camera::{MIN_HIT_PX, ZoomVars};
use crate::doc::Doc;
use crate::geom::{self, IDENTITY, Mat, ShapeKind};
use crate::style::{self, Anchor, Paint, PageTheme, Style, ZoomDep};
use crate::tess::{self, MeshBuf, StrokeSpec, TOLERANCE};
use crate::text::{self, FontBook, TextSpec};

/// How a group of vertices moves with the zoom.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupKind {
    /// The map itself.
    Static,
    /// A label whose font size is `a + (b − a)(ls − 1)` at label scale `ls`:
    /// scaled by `size / a` about its anchor, its strokes kept.
    Font { a: f64, b: f64 },
    /// A name plate: `transform: scale(s)` with `s` worked out from the
    /// stylesheet at each zoom; `base` is the scale it was tessellated at.
    Scale { elem: u32, base: f64 },
    /// An edit handle (`updateEditHandles`): radius `min(28, max(base, 7/scale))`
    /// and stroke `min(6, max(1.8, 1.8/scale))`, tessellated at `r0`, `w0`.
    Handle { base: f64, r0: f64, w0: f64, rotate: Option<RotateLever> },
}

/// Where a rotation handle stands: `origin + u · rotationArm(MIN_HIT_PX / scale)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotateLever {
    pub ox: f64,
    pub oy: f64,
    pub ux: f64,
    pub uy: f64,
    /// The arm it was tessellated at.
    pub arm0: f64,
}

/// A group of vertices: drawn at `anchor + t + k·(pos − anchor) + m·offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub kind: GroupKind,
    pub anchor: [f64; 2],
    pub k: f64,
    pub m: f64,
    pub t: [f64; 2],
}

impl Group {
    fn new(kind: GroupKind, anchor: [f64; 2]) -> Group {
        Group { kind, anchor, k: 1.0, m: 1.0, t: [0.0, 0.0] }
    }

    /// A map point of the group's base geometry, where it is drawn now.
    pub fn place(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.anchor[0] + self.t[0] + self.k * (x - self.anchor[0]),
            self.anchor[1] + self.t[1] + self.k * (y - self.anchor[1]),
        )
    }

    /// The inverse of [`Group::place`] (for hit-testing).
    pub fn unplace(&self, x: f64, y: f64) -> (f64, f64) {
        let k = if self.k != 0.0 { self.k } else { 1.0 };
        (
            self.anchor[0] + (x - self.anchor[0] - self.t[0]) / k,
            self.anchor[1] + (y - self.anchor[1] - self.t[1]) / k,
        )
    }
}

/// What the hit-tester tests against.
#[derive(Debug, Clone, PartialEq)]
pub enum HitGeom {
    /// A shape in its own coordinates, with the transform to the map.
    Shape {
        kind: ShapeKind,
        matrix: Mat,
        inv: Mat,
        /// Flattened sub-paths in map coordinates (paths and polygons), with
        /// whether each is closed.
        flat: Vec<(Vec<[f64; 2]>, bool)>,
        /// Whether a fill would paint anything (not a `line`).
        fillable: bool,
    },
    /// A text's character cells, as a quadrilateral in map coordinates, and
    /// the scale of its transform (for the width of its stroke).
    Text { quad: [[f64; 2]; 4], scale: f64 },
}

/// Everything known about one element.
#[derive(Debug, Clone)]
pub struct ElemData {
    /// The style it is drawn with now (hover and zoom included).
    pub style: Style,
    /// Its style at the stylesheet's defaults, unhovered.
    pub base_style: Style,
    pub dep: ZoomDep,
    /// To map coordinates (at the base zoom for a group's members).
    pub matrix: Mat,
    pub group: u32,
    /// Displayed when not far / when far (its ancestors included).
    pub near: bool,
    pub far: bool,
    /// Its opacity times its ancestors'.
    pub opacity: f64,
    /// Not drawn at all: inside `defs`, a `title`, …
    pub hidden: bool,
    /// Styled and tessellated again whenever the zoom changes.
    pub zoom_live: bool,
    /// Its frame (transform, opacity, display) follows the zoom, so its
    /// children are live too.
    pub frame_live: bool,
    /// The group it heads (a label, a name plate, an edit handle).
    pub own_group: Option<u32>,
    pub mesh: MeshBuf,
    /// A text's glyphs and paints, placed at a level of detail on demand.
    pub text: Option<Box<TextPaint>>,
    /// A text's meshes at each level of detail (fine, middle, coarse).
    pub lods: [Option<MeshBuf>; 3],
    /// The mesh's bounds in base map coordinates.
    pub bounds: Option<[f32; 4]>,
    pub hit: Option<HitGeom>,
    /// The hit geometry's bounds (base map coordinates, strokes included).
    pub hit_bounds: Option<[f64; 4]>,
    /// How its pointer target follows the zoom.
    pub live_hit: crate::hit::LiveHit,
    /// How far it has moved, in base map coordinates, since its meshes,
    /// bounds and hit geometry were made: it is drawn and hit-tested this far
    /// from them. Zero once it is tessellated again.
    pub shift: [f64; 2],
    /// Where it was tessellated, while it is shifted.
    pub origin: Option<Box<Origin>>,
}

/// What an element that moved without being tessellated again was made from.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    /// The numbers that placed it then ([`placement`]).
    pub placement: Vec<(Axis, f64)>,
    /// Its transform to the map then.
    pub matrix: Mat,
    /// How far its own numbers have moved since, in its own coordinates.
    pub local: [f64; 2],
}

impl ElemData {
    fn root() -> ElemData {
        ElemData {
            style: Style::default(),
            base_style: Style::default(),
            dep: ZoomDep::None,
            matrix: IDENTITY,
            group: 0,
            near: true,
            far: true,
            opacity: 1.0,
            hidden: false,
            zoom_live: false,
            frame_live: false,
            own_group: None,
            mesh: MeshBuf::default(),
            text: None,
            lods: Default::default(),
            bounds: None,
            hit: None,
            hit_bounds: None,
            live_hit: crate::hit::LiveHit::Fixed,
            shift: [0.0, 0.0],
            origin: None,
        }
    }

    /// Whether it is displayed at a zoom.
    pub fn shown(&self, zoom: &ZoomVars) -> bool {
        !self.hidden && if zoom.is_far { self.far } else { self.near }
    }
}

/// Elements whose content is never drawn where it stands.
fn never_drawn(name: &str) -> bool {
    matches!(
        name,
        "defs" | "title" | "desc" | "style" | "metadata" | "pattern" | "filter" | "clipPath" | "mask" | "symbol" | "marker" | "linearGradient" | "radialGradient"
    ) || name.starts_with("fe")
}

/// Timings of a build, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BuildStats {
    pub elements: usize,
    pub style_ms: f64,
    pub tessellate_ms: f64,
    pub vertices: usize,
    pub triangles: usize,
    /// Elements redone (all of them on a full build).
    pub rebuilt: usize,
    /// Of those, the ones that only moved: shifted, not tessellated again.
    pub shifted: usize,
}

/// The styled, tessellated map.
#[derive(Debug, Clone)]
pub struct Scene {
    pub doc: Doc,
    pub elems: Vec<ElemData>,
    pub groups: Vec<Group>,
    pub theme: PageTheme,
    /// The camera scale the zoom-live parts were last built for.
    pub scale: f64,
    pub zoom: ZoomVars,
    pub hovered: Option<u32>,
    hover_scope: Vec<u32>,
    ids: HashMap<String, u32>,
    /// Bumped whenever a mesh or a group changes.
    pub revision: u64,
    pub stats: BuildStats,
}

impl Scene {
    /// Style and tessellate a whole tree, for a camera at `scale`.
    pub fn build(root: Element, theme: PageTheme, scale: f64) -> Scene {
        let doc = Doc::new(root);
        let n = doc.len();
        let mut scene = Scene {
            doc,
            elems: Vec::with_capacity(n),
            groups: vec![Group::new(GroupKind::Static, [0.0, 0.0])],
            theme,
            scale,
            zoom: ZoomVars::at(scale),
            hovered: None,
            hover_scope: Vec::new(),
            ids: HashMap::new(),
            revision: 1,
            stats: BuildStats::default(),
        };
        let t0 = std::time::Instant::now();
        let base = style::resolve_all(&scene.doc.root, &scene.theme, ZoomVars::base(), None);
        let probe = style::resolve_all(&scene.doc.root, &scene.theme, style::probe_zoom(), None);
        let t1 = std::time::Instant::now();
        for i in 0..n {
            if let Some(id) = scene.doc.element(i).get("id") {
                scene.ids.insert(id.to_string(), i as u32);
            }
        }
        scene.elems = (0..n).map(|_| ElemData::root()).collect();
        {
            let mut book = text::font_book();
            for i in 0..n {
                let dep = style::zoom_dependence(&base[i], &probe[i]);
                scene.prepare(i, base[i].clone(), dep, None);
            }
            for i in 0..n {
                scene.tessellate(i, &mut book);
            }
        }
        scene.refresh_groups();
        let t2 = std::time::Instant::now();
        scene.stats = BuildStats {
            elements: n,
            style_ms: (t1 - t0).as_secs_f64() * 1e3,
            tessellate_ms: (t2 - t1).as_secs_f64() * 1e3,
            rebuilt: n,
            ..Default::default()
        };
        scene.count();
        scene
    }

    fn count(&mut self) {
        self.stats.vertices = self.elems.iter().map(|e| e.mesh.verts.len()).sum();
        self.stats.triangles = self.elems.iter().map(|e| e.mesh.indices.len() / 3).sum();
    }

    /// Take a new tree for the same map. When it has the same shape as the
    /// one drawn, only the elements that differ, and everything under them,
    /// are redone; otherwise the whole is rebuilt.
    ///
    /// An element whose only differences are the numbers that place it, all
    /// moved by one delta, keeps its style, its meshes and its hit geometry:
    /// it is shifted ([`ElemData::shift`]). Everything else that differs is
    /// styled and tessellated again.
    pub fn update(&mut self, root: Element) {
        let t0 = std::time::Instant::now();
        let mut changes: Vec<(usize, Change)> = Vec::new();
        if !diff_tree(&self.doc, &root, &mut changes) {
            let hovered = self.hovered;
            let mut fresh = Scene::build(root, self.theme.clone(), self.scale);
            if let Some(h) = hovered.filter(|h| (*h as usize) < fresh.doc.len()) {
                fresh.set_hover(Some(h));
            }
            fresh.revision = self.revision + 1;
            retire(std::mem::replace(self, fresh));
            return;
        }
        let old = std::mem::replace(&mut self.doc.root, root);
        if changes.is_empty() {
            retire(old);
            return;
        }
        // Every element under a changed one is redone too (its transform,
        // opacity or inherited style may have changed with it).
        const UNDER_OTHER: u8 = 1;
        const UNDER_MOVE: u8 = 2;
        const MOVED: u8 = 4;
        const NEW_TRANSFORM: u8 = 8;
        let n = self.doc.len();
        let mut flags = vec![0u8; n];
        for &(c, change) in &changes {
            let end = self.doc.nodes[c].end as usize;
            let under = if change == Change::Other { UNDER_OTHER } else { UNDER_MOVE };
            for f in &mut flags[c..end] {
                *f |= under;
            }
            if let Change::Moved { transform } = change {
                flags[c] |= MOVED | if transform { NEW_TRANSFORM } else { 0 };
            }
        }
        let hovered = self.hovered.map(|h| h as usize);
        let mut in_scope = vec![false; n];
        for &s in &self.hover_scope {
            in_scope[s as usize] = true;
        }
        // Styles and frames first, in document order (a parent's frame is
        // its children's), then meshes: a shape can read another element's
        // style (a pattern's lines).
        let mut moved: Vec<(usize, Before)> = Vec::new();
        let mut redo: Vec<usize> = Vec::new();
        for (i, &f) in flags.iter().enumerate() {
            if f == 0 {
                continue;
            }
            let hov = hovered.filter(|_| in_scope[i]);
            if f & UNDER_OTHER != 0 {
                if let Some(id) = self.doc.element(i).get("id") {
                    self.ids.insert(id.to_string(), i as u32);
                }
                self.restyle(i, hov);
                redo.push(i);
                continue;
            }
            let mut before = Before::of(&self.elems[i]);
            if f & NEW_TRANSFORM != 0 {
                before.style_stands = self.retransform(i, self.doc.element_in(&old, i), hov);
            } else {
                let style = std::mem::take(&mut self.elems[i].style);
                self.apply_style(i, style);
            }
            moved.push((i, before));
        }
        // How far each moved (its numbers parsed and compared: in parallel),
        // then shifted, or tessellated again where it did not just move.
        let measured: Vec<Option<Moved>> = {
            use rayon::prelude::*;
            let scene = &*self;
            moved.par_iter().map(|(i, before)| scene.measure_move(*i, flags[*i] & MOVED != 0, scene.doc.element_in(&old, *i), before)).collect()
        };
        let mut book = text::font_book();
        let mut shifted = 0;
        for ((i, _), m) in moved.iter().zip(measured) {
            match m {
                Some(m) => {
                    self.apply_move(*i, m);
                    shifted += 1;
                }
                None => self.tessellate(*i, &mut book),
            }
        }
        for &i in &redo {
            self.tessellate(i, &mut book);
        }
        drop(book);
        retire(old);
        // A name plate's scale is worked out from the stylesheet: only for
        // the plates redone.
        self.refresh_groups_where(|head| flags[head] != 0);
        self.revision += 1;
        self.stats.rebuilt = moved.len() + redo.len();
        self.stats.shifted = shifted;
        self.stats.style_ms = 0.0;
        self.stats.tessellate_ms = t0.elapsed().as_secs_f64() * 1e3;
        self.count();
    }

    /// Style an element again from the stylesheet, as a build does.
    fn restyle(&mut self, i: usize, hovered: Option<usize>) {
        let path = self.doc.path(i);
        let base = style::resolve_one(&path, &self.theme, ZoomVars::base(), None);
        let probe = style::resolve_one(&path, &self.theme, style::probe_zoom(), None);
        let dep = style::zoom_dependence(&base, &probe);
        self.prepare(i, base, dep, hovered);
    }

    /// An element whose `transform` attribute changed (with what places
    /// it, if anything): when its transform is the attribute's (no rule of
    /// the stylesheet sets one) the new attribute is taken as it is and
    /// nothing is resolved; otherwise it is styled again. Whether its style
    /// is otherwise what it was.
    fn retransform(&mut self, i: usize, old: &Element, hovered: Option<usize>) -> bool {
        let attr = |e: &Element| e.get("transform").and_then(coxeter_render::css::parse_transform).filter(|m| *m != IDENTITY);
        let d = &self.elems[i];
        let own = d.base_style.transform == attr(old)
            && d.style.transform == d.base_style.transform
            && matches!(d.dep, ZoomDep::None | ZoomDep::Label { .. });
        if own {
            let t = attr(self.doc.element(i));
            self.elems[i].base_style.transform = t;
            let mut style = std::mem::take(&mut self.elems[i].style);
            style.transform = t;
            self.apply_style(i, style);
            return true;
        }
        let (mut was, dep) = (d.style.clone(), d.dep.clone());
        self.restyle(i, hovered);
        let d = &self.elems[i];
        was.transform = d.style.transform;
        was == d.style && dep == d.dep
    }

    /// Whether an element only moved, and how far: the numbers that place it
    /// (when `moved_self`) and its transform to the map moved by one delta,
    /// and nothing else about it changed. `None` if it must be tessellated
    /// again.
    fn measure_move(&self, i: usize, moved_self: bool, old: &Element, before: &Before) -> Option<Moved> {
        let d = &self.elems[i];
        if !before.style_stands || !before.same_frame(d) {
            return None;
        }
        // A pattern stays where it is under a shape that moves, and the
        // heads of groups other than a label's scale about points of their own.
        if matches!(d.style.fill, Paint::Url(_)) {
            return None;
        }
        if let Some(g) = d.own_group
            && !matches!(self.groups[g as usize].kind, GroupKind::Font { .. })
        {
            return None;
        }
        let new = self.doc.element(i);
        let (matrix0, placement0, local) = match &d.origin {
            Some(o) => {
                let local = if moved_self { translation(&o.placement, &placement(new)?)? } else { o.local };
                (o.matrix, None, local)
            }
            None => {
                let p0 = placement(old)?;
                let local = if moved_self { translation(&p0, &placement(new)?)? } else { [0.0, 0.0] };
                (before.matrix, Some(p0), local)
            }
        };
        let m = d.matrix;
        if !same_linear(&matrix0, &m) {
            return None;
        }
        // A point `p` of the geometry it was made from is now at
        // `m · (p + local)`: `matrix0 · p` moved by this.
        let (lx, ly) = geom::apply_vec(&m, local[0], local[1]);
        let shift = [m[4] - matrix0[4] + lx, m[5] - matrix0[5] + ly];
        Some(Moved { shift, local, origin: placement0.map(|placement| Origin { placement, matrix: matrix0, local }) })
    }

    fn apply_move(&mut self, i: usize, m: Moved) {
        let d = &mut self.elems[i];
        d.shift = m.shift;
        match (&mut d.origin, m.origin) {
            (Some(o), _) => o.local = m.local,
            (None, origin) => d.origin = origin.map(Box::new),
        }
    }

    /// Settle an element's styles: `base` (at the stylesheet's defaults,
    /// unhovered), its dependence on the zoom, and the style it is drawn
    /// with now (at the current zoom where it follows it, hovered when the
    /// hovered element `hovered` reaches it).
    fn prepare(&mut self, i: usize, base: Style, dep: ZoomDep, hovered: Option<usize>) {
        let live = matches!(dep, ZoomDep::Other) || base.non_scaling_stroke;
        let style = if live || hovered.is_some() {
            let path = self.doc.path(i);
            let zoom = if live { self.zoom } else { ZoomVars::base() };
            let hov = hovered.map(|h| self.doc.element(h));
            style::resolve_one(&path, &self.theme, zoom, hov)
        } else {
            base.clone()
        };
        self.elems[i].base_style = base;
        self.elems[i].dep = dep;
        self.apply_style(i, style);
    }

    /// Work out an element's transform, group, display and opacity from its
    /// parent's and the style it is drawn with.
    fn apply_style(&mut self, i: usize, style: Style) {
        let e = self.doc.element(i);
        let parent = self.doc.parent(i);
        let p = parent.map(|p| self.elems[p].clone_frame()).unwrap_or_else(Frame::root);
        let dep = self.elems[i].dep.clone();
        let hidden = p.hidden || never_drawn(e.name);
        let own = style.transform.unwrap_or(IDENTITY);
        let mut matrix = geom::mul(&p.matrix, &own);
        // A transform the stylesheet gives a crossfeed arrow on hover is about
        // its own centre (`transform-box: fill-box; transform-origin: center`).
        if style.transform.is_some() && e.has_class("emc-crossfeed-arrow")
            && let Some(shape) = geom::shape_of(e) {
                let b = path_bounds(&shape.path);
                let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                let about = geom::mul(&geom::mul(&[1.0, 0.0, 0.0, 1.0, cx, cy], &own), &[1.0, 0.0, 0.0, 1.0, -cx, -cy]);
                matrix = geom::mul(&p.matrix, &about);
            }
        let mut own_group: Option<(GroupKind, [f64; 2])> = None;
        if !hidden {
            match &dep {
                ZoomDep::Scale => {
                    let s = style::uniform_scale(style.transform).unwrap_or(1.0);
                    let (ax, ay) = geom::apply(&p.matrix, 0.0, 0.0);
                    own_group = Some((GroupKind::Scale { elem: i as u32, base: s }, [ax, ay]));
                }
                ZoomDep::Label { font: Some((a, b)), .. } if e.name == "text" => {
                    let (x, y) = (geom::num_attr(e, "x"), geom::num_attr(e, "y"));
                    let (ax, ay) = geom::apply(&matrix, x, y);
                    own_group = Some((GroupKind::Font { a: *a, b: *b }, [ax, ay]));
                }
                _ => {}
            }
            if let Some(base_r) = e.get("data-edit-handle-base").map(js_number) {
                let base_r = if base_r > 0.0 && base_r.is_finite() { base_r } else { 5.5 };
                let (cx, cy) = (geom::num_attr(e, "cx"), geom::num_attr(e, "cy"));
                let r0 = geom::num_attr(e, "r");
                let rotate = e.get("data-rotate-handle").map(|_| {
                    let (ox, oy) = (geom::num_attr(e, "data-rotate-ox"), geom::num_attr(e, "data-rotate-oy"));
                    let (ux, uy) = (geom::num_attr(e, "data-rotate-ux"), geom::num_attr(e, "data-rotate-uy"));
                    RotateLever { ox, oy, ux, uy, arm0: (cx - ox) * ux + (cy - oy) * uy }
                });
                let (ax, ay) = geom::apply(&matrix, cx, cy);
                own_group = Some((GroupKind::Handle { base: base_r, r0, w0: style.stroke_width, rotate }, [ax, ay]));
            }
        }
        let rotate_guide = e.get("data-rotate-guide").is_some();
        let group = match own_group {
            Some((kind, anchor)) => match self.elems[i].own_group {
                Some(g) => {
                    self.groups[g as usize].kind = kind;
                    self.groups[g as usize].anchor = anchor;
                    g
                }
                None => {
                    let g = self.add_group(kind, anchor);
                    self.elems[i].own_group = Some(g);
                    g
                }
            },
            None => {
                self.elems[i].own_group = None;
                p.group
            }
        };
        let (near, far) = match &dep {
            ZoomDep::Label { display: Some((dn, df)), .. } => (p.near && *dn, p.far && *df),
            _ => (p.near && style.displayed, p.far && style.displayed),
        };
        // Styled again at each zoom: what the stylesheet ties to the zoom some
        // other way (and everything under it), strokes that do not scale, and
        // the rotation guides `updateEditHandles()` lengthens.
        let frame_live = !hidden && (matches!(dep, ZoomDep::Other) || p.frame_live);
        let zoom_live = !hidden && (frame_live || style.non_scaling_stroke || rotate_guide);
        let opacity = p.opacity * style.opacity;
        let d = &mut self.elems[i];
        d.style = style;
        d.matrix = matrix;
        d.group = group;
        d.near = near;
        d.far = far;
        d.opacity = opacity;
        d.hidden = hidden;
        d.zoom_live = zoom_live;
        d.frame_live = frame_live;
    }

    fn add_group(&mut self, kind: GroupKind, anchor: [f64; 2]) -> u32 {
        self.groups.push(Group::new(kind, anchor));
        (self.groups.len() - 1) as u32
    }

    /// Tessellate one element (its own paint only; children are elements of
    /// their own) and work out its hit geometry.
    fn tessellate(&mut self, i: usize, book: &mut FontBook) {
        let mut mesh = MeshBuf::default();
        let mut text_paint = None;
        let live_hit = {
            let e = self.doc.element(i);
            if e.has_class("emc-hit") {
                crate::hit::LiveHit::Halo(e.get("data-hit-base").map(js_number).filter(|v| v.is_finite()).unwrap_or(0.0))
            } else if e.has_class("emc-multi-drag-hit") {
                crate::hit::LiveHit::MultiBorder
            } else {
                crate::hit::LiveHit::Fixed
            }
        };
        let (hit, hit_bounds) = {
            let e = self.doc.element(i);
            let d = &self.elems[i];
            if d.hidden || (!d.near && !d.far) {
                (None, None)
            } else if e.name == "text" {
                text_paint = self.text_paint(e, d, book);
                match &text_paint {
                    Some(tp) => {
                        let (h, hb, _) = Scene::text_hit(tp);
                        (Some(h), Some(hb))
                    }
                    None => (None, None),
                }
            } else if let Some(shape) = geom::shape_of(e) {
                self.shape_mesh(i, e, d, shape, &mut mesh)
            } else {
                (None, None)
            }
        };
        let d = &mut self.elems[i];
        d.lods = Default::default();
        d.shift = [0.0, 0.0];
        d.origin = None;
        match text_paint {
            Some(tp) => {
                d.bounds = Some(Scene::text_hit(&tp).2);
                d.mesh = MeshBuf::default();
                d.text = Some(Box::new(tp));
            }
            None => {
                d.bounds = mesh.bounds();
                d.mesh = mesh;
                d.text = None;
            }
        }
        d.hit = hit;
        d.hit_bounds = hit_bounds;
        d.live_hit = live_hit;
        if d.text.is_some() {
            let level = self.text_level(i, self.scale);
            self.ensure_lod(i, level, book);
        }
    }

    /// The level of detail a text is drawn at: by its size on screen.
    pub fn text_level(&self, i: usize, scale: f64) -> usize {
        let d = &self.elems[i];
        let Some(tp) = &d.text else { return 0 };
        let k = if d.group == 0 { 1.0 } else { self.groups[d.group as usize].k };
        text::lod_for_pixels(tp.font_size * geom::mean_scale(&tp.matrix) * k * scale)
    }

    /// Place a text's glyphs at a level of detail, if not done yet.
    pub fn ensure_lod(&mut self, i: usize, level: usize, book: &mut FontBook) {
        let d = &self.elems[i];
        let Some(tp) = &d.text else { return };
        if d.lods[level].is_some() {
            return;
        }
        let mut out = MeshBuf::default();
        place_text(tp, level, book, &mut out);
        self.elems[i].lods[level] = Some(out);
    }

    /// Place the levels of detail the visible texts need at a camera scale
    /// (`view` in map coordinates). Glyphs are tessellated once per font, so
    /// this only copies them.
    pub fn ensure_lods(&mut self, scale: f64, view: [f64; 4]) {
        let wanted: Vec<(usize, usize)> = (0..self.elems.len())
            .filter(|&i| {
                let d = &self.elems[i];
                d.text.is_some() && d.shown(&self.zoom) && crate::painter::meets(self, i, &view)
            })
            .map(|i| (i, self.text_level(i, scale)))
            .filter(|&(i, l)| self.elems[i].lods[l].is_none())
            .collect();
        if wanted.is_empty() {
            return;
        }
        let mut book = text::font_book();
        for (i, l) in wanted {
            self.ensure_lod(i, l, &mut book);
        }
        self.revision += 1;
    }

    fn stroke_width_map(&self, d: &ElemData) -> f64 {
        if d.style.non_scaling_stroke {
            d.style.stroke_width / self.scale.max(1e-9)
        } else {
            d.style.stroke_width * geom::mean_scale(&d.matrix)
        }
    }

    fn shape_mesh(&self, i: usize, e: &Element, d: &ElemData, shape: geom::Shape, out: &mut MeshBuf) -> (Option<HitGeom>, Option<[f64; 4]>) {
        let style = &d.style;
        let mapped = tess::transform_path(&shape.path, &d.matrix);
        let sw = self.stroke_width_map(d);
        let group = d.group;
        let paint_it = style.visible;
        let fill_color = match &style.fill {
            Paint::Color(c) if paint_it && shape.fillable => Some(style::color32(*c, style.fill_opacity * d.opacity)),
            _ => None,
        };
        let stroke_color = match &style.stroke {
            Paint::Color(c) if paint_it && sw > 0.0 => Some(style::color32(*c, style.stroke_opacity * d.opacity)),
            _ => None,
        };
        let scale_dash = if style.non_scaling_stroke { 1.0 / self.scale.max(1e-9) } else { geom::mean_scale(&d.matrix) };
        let spec = StrokeSpec {
            width: sw as f32,
            cap: style.linecap,
            join: style.linejoin,
            miter_limit: 4.0,
            dash: style.dash.as_ref().map(|v| v.iter().map(|x| (x * scale_dash) as f32).collect()),
        };
        // Filters, approximated: a soft shadow or glow drawn under the shape.
        if paint_it && let Some(filter) = &style.filter {
            self.filter_pass(filter, &mapped, shape.fillable && !style.fill.is_none(), &style.fill, &style.stroke, sw, d, out);
        }
        let fill_pass = |out: &mut MeshBuf| {
            if let Some(c) = fill_color.filter(|c| c.a() > 0) {
                tess::fill(&mapped, c, group, TOLERANCE, out);
            } else if let Paint::Url(id) = &style.fill
                && paint_it && shape.fillable {
                    self.pattern_fill(id, &mapped, style.fill_opacity * d.opacity, group, out);
                }
        };
        let stroke_pass = |out: &mut MeshBuf| {
            if let Some(c) = stroke_color.filter(|c| c.a() > 0) {
                tess::stroke(&mapped, &spec, c, group, TOLERANCE, out);
            }
        };
        if style.stroke_first {
            stroke_pass(out);
            fill_pass(out);
        } else {
            fill_pass(out);
            stroke_pass(out);
        }
        let _ = (i, e);
        // Hit geometry.
        let flat = match shape.kind {
            ShapeKind::Path | ShapeKind::Poly { .. } => flatten(&mapped),
            _ => Vec::new(),
        };
        let inv = geom::invert(&d.matrix).unwrap_or(IDENTITY);
        let b = path_bounds(&mapped);
        let pad = (sw / 2.0).max(0.0);
        let hit_bounds = [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad];
        (Some(HitGeom::Shape { kind: shape.kind, matrix: d.matrix, inv, flat, fillable: shape.fillable }), Some(hit_bounds))
    }

    fn text_paint(&self, e: &Element, d: &ElemData, book: &mut FontBook) -> Option<TextPaint> {
        let style = &d.style;
        let raw = text::collapse_whitespace(&e.text_content());
        let content = match style.text_transform {
            style::TextTransform::Uppercase => raw.to_uppercase(),
            style::TextTransform::Lowercase => raw.to_lowercase(),
            style::TextTransform::None => raw,
        };
        if content.is_empty() {
            return None;
        }
        let x = geom::num_attr(e, "x");
        let y = geom::num_attr(e, "y");
        // `textLength` with `lengthAdjust="spacingAndGlyphs"` (all the map
        // uses); plain `spacing` is laid out the same way.
        let text_length = geom::opt_num_attr(e, "textLength");
        let spec = TextSpec {
            families: &style.font_family,
            weight: style.font_weight,
            size: style.font_size,
            letter_spacing: style.letter_spacing,
            anchor: style.anchor,
            baseline: style.baseline,
            text_length,
            tabular: style.tabular_nums,
        };
        let run = book.layout(&content, &spec)?;
        let paint_it = style.visible;
        let fill = match &style.fill {
            Paint::Color(c) if paint_it => Some(style::color32(*c, style.fill_opacity * d.opacity)).filter(|c| c.a() > 0),
            _ => None,
        };
        let stroke = match &style.stroke {
            Paint::Color(c) if paint_it && style.stroke_width > 0.0 => {
                Some(style::color32(*c, style.stroke_opacity * d.opacity)).filter(|c| c.a() > 0)
            }
            _ => None,
        };
        Some(TextPaint {
            run,
            x,
            y,
            matrix: d.matrix,
            group: d.group,
            fill,
            stroke,
            stroke_width: style.stroke_width,
            join: style.linejoin,
            stroke_first: style.stroke_first,
            underline: style.underline,
            font_size: style.font_size,
        })
    }

    /// The hit quad and bounds of a laid-out text.
    fn text_hit(tp: &TextPaint) -> (HitGeom, [f64; 4], [f32; 4]) {
        let (x, y, run, m) = (tp.x, tp.y, &tp.run, tp.matrix);
        let corners = [
            (x + run.left, y + run.top),
            (x + run.left + run.width, y + run.top),
            (x + run.left + run.width, y + run.bottom),
            (x + run.left, y + run.bottom),
        ];
        let quad = corners.map(|(cx, cy)| {
            let (a, b) = geom::apply(&m, cx, cy);
            [a, b]
        });
        let xs = quad.map(|p| p[0]);
        let ys = quad.map(|p| p[1]);
        // The cells, and the band a painted stroke adds around them.
        let half_stroke = if tp.stroke.is_some() { tp.stroke_width.max(0.0) * geom::mean_scale(&m) / 2.0 } else { 0.0 };
        let hb = [
            xs.iter().copied().fold(f64::INFINITY, f64::min) - half_stroke,
            ys.iter().copied().fold(f64::INFINITY, f64::min) - half_stroke,
            xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) + half_stroke,
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max) + half_stroke,
        ];
        // Glyphs reach past their cells a little (accents, descenders), and the halo further.
        let pad = tp.stroke_width.max(0.0) * geom::mean_scale(&m) / 2.0 + tp.font_size * geom::mean_scale(&m) * 0.3;
        let bounds = [(hb[0] - pad) as f32, (hb[1] - pad) as f32, (hb[2] + pad) as f32, (hb[3] + pad) as f32];
        (HitGeom::Text { quad, scale: geom::mean_scale(&m) }, hb, bounds)
    }

    /// `url(#emc-wall)` and its kind: a pattern of parallel lines, drawn as
    /// stripes cut to the (convex) shape.
    fn pattern_fill(&self, id: &str, shape: &Path, alpha: f64, group: u32, out: &mut MeshBuf) {
        let Some(&pi) = self.ids.get(id) else { return };
        let pe = self.doc.element(pi as usize);
        if pe.name != "pattern" {
            return;
        }
        let w = geom::num_attr(pe, "width");
        let h = geom::num_attr(pe, "height");
        if !(w > 0.0 && h > 0.0) {
            return;
        }
        let pt = pe.get("patternTransform").and_then(geom::parse_transform).unwrap_or(IDENTITY);
        let Some(inv) = geom::invert(&pt) else { return };
        // The shape in pattern space (map coordinates are the user space here:
        // the shape's own transform is already applied to it).
        let Some(poly) = flatten(shape).into_iter().next().map(|(p, _)| p) else { return };
        let local: Vec<[f64; 2]> = poly
            .iter()
            .map(|p| {
                let (a, b) = geom::apply(&inv, p[0], p[1]);
                [a, b]
            })
            .collect();
        for child in self.doc.children(pi as usize) {
            let ce = self.doc.element(child);
            if ce.name != "line" {
                continue;
            }
            let cs = &self.elems[child].style;
            let Paint::Color(c) = cs.stroke else { continue };
            let color = style::color32(c, cs.stroke_opacity * cs.opacity * alpha);
            let (x1, y1, x2, y2) = (geom::num_attr(ce, "x1"), geom::num_attr(ce, "y1"), geom::num_attr(ce, "x2"), geom::num_attr(ce, "y2"));
            let half = cs.stroke_width / 2.0;
            // A vertical line repeats across, a horizontal one down.
            let (axis, at, period) = if x1 == x2 && (y2 - y1).abs() >= h {
                (0, x1, w)
            } else if y1 == y2 && (x2 - x1).abs() >= w {
                (1, y1, h)
            } else {
                continue;
            };
            let lo = local.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
            let hi = local.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max);
            let k0 = ((lo - at - half) / period).floor() as i64;
            let k1 = ((hi - at + half) / period).ceil() as i64;
            for k in k0..=k1 {
                let centre = at + k as f64 * period;
                let mut clipped = clip_half_plane(&local, axis, centre - half, true);
                clipped = clip_half_plane(&clipped, axis, centre + half, false);
                if clipped.len() < 3 {
                    continue;
                }
                let pts: Vec<(f32, f32)> = clipped
                    .iter()
                    .map(|p| {
                        let (a, b) = geom::apply(&pt, p[0], p[1]);
                        (a as f32, b as f32)
                    })
                    .collect();
                if let Some(path) = tess::polygon_path(&pts) {
                    tess::fill(&path, color, group, TOLERANCE, out);
                }
            }
        }
    }

    /// The map's three filters, approximated by layers drawn under the shape:
    /// `emc-cell-shadow` (a soft drop shadow), `emc-glow` (a blur of the shape
    /// under it) and `emc-ribbon-soft` (a softened ribbon).
    #[allow(clippy::too_many_arguments)]
    fn filter_pass(&self, id: &str, mapped: &Path, filled: bool, fill: &Paint, stroke: &Paint, sw: f64, d: &ElemData, out: &mut MeshBuf) {
        let Some(&fi) = self.ids.get(id) else { return };
        let primitive = self.doc.children(fi as usize).next().map(|c| (c, self.doc.element(c)));
        let Some((_, fe)) = primitive else { return };
        let group = d.group;
        let scale = geom::mean_scale(&d.matrix);
        match fe.name {
            "feDropShadow" => {
                let dx = geom::num_attr(fe, "dx") * scale;
                let dy = geom::num_attr(fe, "dy") * scale;
                let sigma = geom::num_attr(fe, "stdDeviation") * scale;
                let flood = geom::opt_num_attr(fe, "flood-opacity").unwrap_or(1.0);
                let shifted = tess::transform_path(mapped, &[1.0, 0.0, 0.0, 1.0, dx, dy]);
                let steps = 6;
                let alpha = flood * d.opacity / steps as f64;
                let black = coxeter_render::css::Rgba { r: 0, g: 0, b: 0, a: 1.0 };
                for s in 0..steps {
                    // Layers from 1.5σ outside to 1.5σ inside the edge.
                    let grow = sigma * (1.5 - 3.0 * (s as f64 + 0.5) / steps as f64);
                    let c = style::color32(black, alpha);
                    if grow > 0.0 {
                        tess::fill(&shifted, c, group, TOLERANCE, out);
                        let spec = StrokeSpec { width: (2.0 * grow) as f32, join: style::LineJoin::Round, ..StrokeSpec::solid(0.0) };
                        tess::stroke(&shifted, &spec, c, group, TOLERANCE, out);
                    } else {
                        // Inside the edge: the shape shrunk, approximated by
                        // the shape itself (it is covered by the source).
                        tess::fill(&shifted, c, group, TOLERANCE, out);
                    }
                }
            }
            "feGaussianBlur" => {
                let sigma = geom::num_attr(fe, "stdDeviation") * scale;
                let merges_source = self.doc.children(fi as usize).any(|c| self.doc.element(c).name == "feMerge");
                let colour = |p: &Paint, a: f64| match p {
                    Paint::Color(c) => Some(style::color32(*c, a)),
                    _ => None,
                };
                let steps = 3;
                for s in 0..steps {
                    let grow = sigma * (2.0 - 1.5 * s as f64 / (steps - 1) as f64);
                    let a = if merges_source { 0.22 } else { 0.3 } * d.opacity;
                    if filled
                        && let Some(c) = colour(fill, a * d.style.fill_opacity) {
                            let spec = StrokeSpec { width: (2.0 * grow) as f32, join: style::LineJoin::Round, ..StrokeSpec::solid(0.0) };
                            tess::stroke(mapped, &spec, c, group, TOLERANCE, out);
                        }
                    if let Some(c) = colour(stroke, a * d.style.stroke_opacity)
                        && sw > 0.0 {
                            let spec = StrokeSpec {
                                width: (sw + 2.0 * grow) as f32,
                                cap: d.style.linecap,
                                join: style::LineJoin::Round,
                                ..StrokeSpec::solid(0.0)
                            };
                            tess::stroke(mapped, &spec, c, group, TOLERANCE, out);
                        }
                }
            }
            _ => {}
        }
    }

    // Zoom

    /// Bring the zoom-dependent parts up to a camera scale (`applyCamera()`).
    pub fn set_scale(&mut self, scale: f64) {
        if !(scale > 0.0) || scale == self.scale {
            return;
        }
        self.scale = scale;
        self.zoom = ZoomVars::at(scale);
        // Styles that follow the zoom some other way, and strokes that do not scale.
        let live: Vec<usize> = (0..self.elems.len()).filter(|i| self.elems[*i].zoom_live).collect();
        if !live.is_empty() {
            let hovered = self.hovered.map(|h| h as usize);
            let in_scope = |i: usize| self.hover_scope.binary_search(&(i as u32)).is_ok();
            let styles: Vec<(usize, Style)> = live
                .iter()
                .map(|&i| {
                    let path = self.doc.path(i);
                    let hov = hovered.filter(|_| in_scope(i)).map(|h| self.doc.element(h));
                    (i, style::resolve_one(&path, &self.theme, self.zoom, hov))
                })
                .collect();
            for (i, s) in styles {
                self.apply_style(i, s);
            }
            let mut book = text::font_book();
            for &i in &live {
                self.tessellate(i, &mut book);
            }
        }
        self.refresh_groups();
        self.revision += 1;
    }

    /// Work out every group's scale at the current zoom.
    pub fn refresh_groups(&mut self) {
        self.refresh_groups_where(|_| true);
    }

    /// The same, but a name plate's (which asks the stylesheet) only when
    /// `head(element)` says so.
    fn refresh_groups_where(&mut self, head: impl Fn(usize) -> bool) {
        let zoom = self.zoom;
        let scale = self.scale;
        for gi in 1..self.groups.len() {
            let kind = self.groups[gi].kind.clone();
            let (k, m, t) = match kind {
                GroupKind::Static => (1.0, 1.0, [0.0, 0.0]),
                GroupKind::Scale { elem, .. } if !head(elem as usize) => continue,
                GroupKind::Font { a, b } => {
                    let size = a + (b - a) * (zoom.label_scale - 1.0);
                    (if a > 0.0 { size / a } else { 1.0 }, 1.0, [0.0, 0.0])
                }
                GroupKind::Scale { elem, base } => {
                    let path = self.doc.path(elem as usize);
                    let s = style::resolve_one(&path, &self.theme, zoom, None);
                    let now = style::uniform_scale(s.transform).unwrap_or(1.0);
                    let k = if base != 0.0 { now / base } else { 1.0 };
                    (k, k, [0.0, 0.0])
                }
                GroupKind::Handle { base, r0, w0, rotate } => {
                    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
                    let (r, w) = handle_size(base, s);
                    let t = match rotate {
                        Some(lever) => {
                            let arm = coxeter_render::rotation_arm(MIN_HIT_PX / s);
                            [lever.ux * (arm - lever.arm0), lever.uy * (arm - lever.arm0)]
                        }
                        None => [0.0, 0.0],
                    };
                    (if r0 > 0.0 { r / r0 } else { 1.0 }, if w0 > 0.0 { w / w0 } else { 1.0 }, t)
                }
            };
            let g = &mut self.groups[gi];
            g.k = k;
            g.m = m;
            g.t = t;
        }
    }

    // Hover

    /// The element under the pointer, for `:hover`. Restyles what the
    /// stylesheet's hover rules reach and tessellates what changed.
    pub fn set_hover(&mut self, hovered: Option<u32>) {
        if hovered == self.hovered {
            return;
        }
        let old_scope = std::mem::take(&mut self.hover_scope);
        let new_scope = hovered.map(|h| hover_scope(&self.doc, h as usize)).unwrap_or_default();
        self.hovered = hovered;
        let mut all: Vec<u32> = old_scope.iter().chain(new_scope.iter()).copied().collect();
        all.sort_unstable();
        all.dedup();
        self.hover_scope = new_scope;
        if all.is_empty() {
            return;
        }
        let mut redo: Vec<usize> = Vec::new();
        for &i in &all {
            let i = i as usize;
            let live = self.elems[i].zoom_live;
            let hov = hovered.filter(|_| self.hover_scope.binary_search(&(i as u32)).is_ok()).map(|h| self.doc.element(h as usize));
            let target = if hov.is_none() && !live {
                self.elems[i].base_style.clone()
            } else {
                let zoom = if live { self.zoom } else { ZoomVars::base() };
                style::resolve_one(&self.doc.path(i), &self.theme, zoom, hov)
            };
            if target != self.elems[i].style {
                self.elems[i].style = target;
                redo.push(i);
            }
        }
        if redo.is_empty() {
            return;
        }
        // A restyled element's frame reaches everything under it.
        let mut book = text::font_book();
        let mut done_until = 0usize;
        for &i in &redo {
            if i < done_until {
                continue;
            }
            let end = self.doc.nodes[i].end as usize;
            for k in i..end {
                let s = self.elems[k].style.clone();
                self.apply_style(k, s);
                self.tessellate(k, &mut book);
            }
            done_until = end;
        }
        drop(book);
        self.refresh_groups();
        self.revision += 1;
    }

    /// The `<title>` text of an element or its nearest ancestor that has one.
    pub fn title_of(&self, i: usize) -> Option<String> {
        for a in self.doc.ancestors(i) {
            for c in self.doc.children(a) {
                let e = self.doc.element(c);
                if e.name == "title" {
                    return Some(e.text_content());
                }
            }
        }
        None
    }

    /// The index of an element by its `id`.
    pub fn by_id(&self, id: &str) -> Option<usize> {
        self.ids.get(id).map(|i| *i as usize)
    }

    /// The canvas: the root `<svg>`'s width and height.
    pub fn canvas_size(&self) -> (f64, f64) {
        (geom::num_attr(&self.doc.root, "width"), geom::num_attr(&self.doc.root, "height"))
    }
}

/// A laid-out text and how it is painted.
#[derive(Debug, Clone, PartialEq)]
pub struct TextPaint {
    pub run: text::TextRun,
    pub x: f64,
    pub y: f64,
    pub matrix: Mat,
    pub group: u32,
    pub fill: Option<egui::Color32>,
    pub stroke: Option<egui::Color32>,
    pub stroke_width: f64,
    pub join: style::LineJoin,
    pub stroke_first: bool,
    pub underline: bool,
    /// Its font size (at the base zoom for a label).
    pub font_size: f64,
}

/// Copy a text's glyph meshes into place, halo and fill in paint order.
pub fn place_text(tp: &TextPaint, level: usize, book: &mut FontBook, out: &mut MeshBuf) {
    let m = tp.matrix;
    let (x, y, group) = (tp.x, tp.y, tp.group);
    let place = |mesh: &MeshBuf, g: &text::PlacedGlyph, color: egui::Color32, out: &mut MeshBuf| {
        let base = out.verts.len() as u32;
        let sx = g.size * g.sx;
        let sy = g.size;
        out.verts.reserve(mesh.verts.len());
        for v in &mesh.verts {
            let lx = x + g.x + v.pos[0] as f64 * sx;
            let ly = y + g.y - v.pos[1] as f64 * sy;
            let (mx, my) = geom::apply(&m, lx, ly);
            let (ox, oy) = geom::apply_vec(&m, v.offset[0] as f64 * sx, -(v.offset[1] as f64) * sy);
            out.verts.push(tess::MapVertex { pos: [mx as f32, my as f32], offset: [ox as f32, oy as f32], color, group });
        }
        out.indices.extend(mesh.indices.iter().map(|i| i + base));
    };
    let stroke_pass = |out: &mut MeshBuf, book: &mut FontBook| {
        if let Some(c) = tp.stroke {
            for g in &tp.run.glyphs {
                if let Some(mesh) = book.glyph_stroke(g.face, g.gid, tp.stroke_width / g.size, tp.join, level) {
                    place(&mesh, g, c, out);
                }
            }
        }
    };
    let fill_pass = |out: &mut MeshBuf, book: &mut FontBook| {
        if let Some(c) = tp.fill {
            for g in &tp.run.glyphs {
                if let Some(mesh) = book.glyph_fill(g.face, g.gid, level) {
                    place(&mesh, g, c, out);
                }
            }
            if tp.underline {
                let (uy, thick) = tp.run.underline;
                let r = geom::rect_path(x + tp.run.left, y + uy - thick / 2.0, tp.run.width, thick.max(0.5), 0.0, 0.0);
                tess::fill(&tess::transform_path(&r, &m), c, group, TOLERANCE, out);
            }
        }
    };
    if tp.stroke_first {
        stroke_pass(out, book);
        fill_pass(out, book);
    } else {
        fill_pass(out, book);
        stroke_pass(out, book);
    }
}

/// What a child inherits from its parent.
#[derive(Debug, Clone, Copy)]
struct Frame {
    matrix: Mat,
    group: u32,
    near: bool,
    far: bool,
    opacity: f64,
    hidden: bool,
    frame_live: bool,
}

impl Frame {
    fn root() -> Frame {
        Frame { matrix: IDENTITY, group: 0, near: true, far: true, opacity: 1.0, hidden: false, frame_live: false }
    }
}

impl ElemData {
    fn clone_frame(&self) -> Frame {
        Frame {
            matrix: self.matrix,
            group: self.group,
            near: self.near,
            far: self.far,
            opacity: self.opacity,
            hidden: self.hidden,
            frame_live: self.frame_live,
        }
    }
}

fn js_number(v: &str) -> f64 {
    coxeter_core::js::string_to_number(v)
}

/// `updateEditHandles()`: an edit handle's radius and stroke width at a zoom,
/// kept roughly finger-sized on screen.
pub fn handle_size(base: f64, scale: f64) -> (f64, f64) {
    let visual = 7.0 / scale;
    (28f64.min(base.max(visual)), 6f64.min(1.8f64.max(1.8 / scale)))
}

/// Whether two trees have the same elements in the same places.
pub fn same_shape(a: &Element, b: &Element) -> bool {
    shape_size(a, b).is_some()
}

/// The number of elements of a subtree, if `a` and `b` have the same shape.
fn shape_size(a: &Element, b: &Element) -> Option<usize> {
    if a.name != b.name {
        return None;
    }
    let mut n = 1;
    let mut ai = a.elements();
    let mut bi = b.elements();
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Some(n),
            (Some(x), Some(y)) => n += shape_size(x, y)?,
            _ => return None,
        }
    }
}

/// How an element differs from the one drawn in its place.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Change {
    /// Only in attributes that place it ([`places`]), `transform` among
    /// them or not.
    Moved { transform: bool },
    /// In anything else.
    Other,
}

/// The attributes that only place an element: the numbers a translation
/// changes, and `transform`, when no selector of the stylesheet tests them.
fn places(name: &str) -> bool {
    static PLACING: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
        ["x", "y", "cx", "cy", "x1", "y1", "x2", "y2", "points", "d", "transform"]
            .into_iter()
            .filter(|a| !style::selectors_test(a))
            .collect()
    });
    PLACING.contains(&name)
}

fn text_node(c: &coxeter_render::Node) -> Option<&str> {
    match c {
        coxeter_render::Node::Text(t) => Some(t.as_ref()),
        _ => None,
    }
}

/// How `b` differs from `a`, if it does (in its own attributes and text).
fn compare(a: &Element, b: &Element) -> Option<Change> {
    let same_text = a.children.iter().filter_map(text_node).eq(b.children.iter().filter_map(text_node));
    if a.attrs == b.attrs {
        return (!same_text).then_some(Change::Other);
    }
    if !same_text || a.attrs.len() != b.attrs.len() {
        return Some(Change::Other);
    }
    let mut transform = false;
    for (x, y) in a.attrs.iter().zip(&b.attrs) {
        if x == y {
            continue;
        }
        if x.name != y.name || x.lead != y.lead || !places(x.name) {
            return Some(Change::Other);
        }
        transform |= x.name == "transform";
    }
    Some(Change::Moved { transform })
}

/// The elements of two trees whose attributes or text differ, and how;
/// false if the trees do not have the same shape. Under an element that
/// changed otherwise than in place nothing is compared: all of it is redone.
fn diff(a: &Element, b: &Element, index: usize, next: &mut usize, out: &mut Vec<(usize, Change)>) -> bool {
    *next = index + 1;
    if a.name != b.name {
        return false;
    }
    match compare(a, b) {
        None => {}
        Some(Change::Other) => {
            out.push((index, Change::Other));
            let Some(size) = shape_size(a, b) else { return false };
            *next = index + size;
            return true;
        }
        Some(moved) => out.push((index, moved)),
    }
    let mut ai = a.elements();
    let mut bi = b.elements();
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                let at = *next;
                if !diff(x, y, at, next, out) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// [`diff`] of the drawn document and a new tree, its layers (the root's
/// children) compared in parallel: a big map's tree is tens of thousands of
/// elements, and comparing them is reading memory. Each layer is numbered
/// as in the drawn document, which holds if the shapes are the same. If
/// they are not, the answer is false anyway.
fn diff_tree(doc: &Doc, root: &Element, out: &mut Vec<(usize, Change)>) -> bool {
    use rayon::prelude::*;
    let old = &doc.root;
    if old.name != root.name {
        return false;
    }
    match compare(old, root) {
        None => {}
        Some(Change::Other) => {
            out.push((0, Change::Other));
            return shape_size(old, root).is_some();
        }
        Some(moved) => out.push((0, moved)),
    }
    let a: Vec<&Element> = old.elements().collect();
    let b: Vec<&Element> = root.elements().collect();
    let starts: Vec<usize> = doc.children(0).collect();
    if a.len() != b.len() || a.len() != starts.len() {
        return false;
    }
    let parts: Vec<Option<Vec<(usize, Change)>>> = (0..a.len())
        .into_par_iter()
        .map(|k| {
            let mut part = Vec::new();
            diff(a[k], b[k], starts[k], &mut 0, &mut part).then_some(part)
        })
        .collect();
    for part in parts {
        let Some(part) = part else { return false };
        out.extend(part);
    }
    true
}

/// How far an element moved ([`Scene::measure_move`]).
#[derive(Debug, Clone)]
struct Moved {
    shift: [f64; 2],
    local: [f64; 2],
    /// Where it was made, the first time it moves.
    origin: Option<Origin>,
}

/// What an element was before an update, to tell a move from a change.
#[derive(Debug, Clone, Copy)]
struct Before {
    matrix: Mat,
    group: u32,
    near: bool,
    far: bool,
    opacity: f64,
    hidden: bool,
    zoom_live: bool,
    frame_live: bool,
    /// Its style is what it was, bar its transform.
    style_stands: bool,
}

impl Before {
    fn of(d: &ElemData) -> Before {
        Before {
            matrix: d.matrix,
            group: d.group,
            near: d.near,
            far: d.far,
            opacity: d.opacity,
            hidden: d.hidden,
            zoom_live: d.zoom_live,
            frame_live: d.frame_live,
            style_stands: true,
        }
    }

    fn same_frame(&self, d: &ElemData) -> bool {
        self.group == d.group
            && self.near == d.near
            && self.far == d.far
            && self.opacity == d.opacity
            && self.hidden == d.hidden
            && self.zoom_live == d.zoom_live
            && self.frame_live == d.frame_live
    }
}

/// What a number that places an element is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// An x coordinate: moves with a translation.
    X,
    /// A y coordinate.
    Y,
    /// Something a translation keeps (which command a path segment is).
    Fixed,
}

/// The renderer prints coordinates to hundredths (`n()` of `mapview.js`):
/// two points moved by one delta can differ by a hundredth once printed.
pub const PRINT_QUANTUM: f64 = 0.01;

/// The numbers that place an element, in its own coordinates and in the
/// order of its attributes: `x`, `y`, `cx`, … (lists included), `points`
/// and a path's `d` (made absolute, arcs as curves). `None` if one does
/// not read as numbers.
pub fn placement(e: &Element) -> Option<Vec<(Axis, f64)>> {
    let numbers = |v: &str| {
        v.split(|c: char| c == ',' || c.is_ascii_whitespace()).filter(|t| !t.is_empty()).map(|t| t.parse::<f64>().ok()).collect::<Option<Vec<f64>>>()
    };
    let mut out = Vec::new();
    for a in &e.attrs {
        if !places(a.name) {
            continue;
        }
        let v = a.value.as_deref().unwrap_or("");
        match a.name {
            "x" | "cx" | "x1" | "x2" => out.extend(numbers(v)?.into_iter().map(|n| (Axis::X, n))),
            "y" | "cy" | "y1" | "y2" => out.extend(numbers(v)?.into_iter().map(|n| (Axis::Y, n))),
            "points" => {
                let axis = |k: usize| if k % 2 == 0 { Axis::X } else { Axis::Y };
                out.extend(numbers(v)?.into_iter().enumerate().map(|(k, n)| (axis(k), n)));
            }
            "d" => {
                use svgtypes::SimplePathSegment as S;
                let mut kinds = Vec::new();
                for seg in svgtypes::SimplifyingPathParser::from(v) {
                    let (kind, points): (f64, &[(f64, f64)]) = match seg.ok()? {
                        S::MoveTo { x, y } => (0.0, &[(x, y)]),
                        S::LineTo { x, y } => (1.0, &[(x, y)]),
                        S::CurveTo { x1, y1, x2, y2, x, y } => (2.0, &[(x1, y1), (x2, y2), (x, y)]),
                        S::Quadratic { x1, y1, x, y } => (3.0, &[(x1, y1), (x, y)]),
                        S::ClosePath => (4.0, &[]),
                    };
                    kinds.push((Axis::Fixed, kind));
                    for &(x, y) in points {
                        out.push((Axis::X, x));
                        out.push((Axis::Y, y));
                    }
                }
                out.extend(kinds);
            }
            // `transform` is read as the element's matrix.
            _ => {}
        }
    }
    Some(out)
}

/// The delta that takes placement `a` to placement `b`, if one does: every
/// x moved by the same amount, and every y, within what printing the
/// numbers rounds ([`PRINT_QUANTUM`]), and the rest unchanged. Per axis, the
/// middle of the deltas found.
pub fn translation(a: &[(Axis, f64)], b: &[(Axis, f64)]) -> Option<[f64; 2]> {
    if a.len() != b.len() {
        return None;
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut big = 1.0f64;
    for (&(ka, va), &(kb, vb)) in a.iter().zip(b) {
        if ka != kb || !va.is_finite() || !vb.is_finite() {
            return None;
        }
        big = big.max(va.abs()).max(vb.abs());
        let k = match ka {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Fixed => {
                if (va - vb).abs() > 1e-9 * (1.0 + va.abs()) {
                    return None;
                }
                continue;
            }
        };
        lo[k] = lo[k].min(vb - va);
        hi[k] = hi[k].max(vb - va);
    }
    let tolerance = PRINT_QUANTUM + 1e-9 * big;
    let mut out = [0.0; 2];
    for k in 0..2 {
        if lo[k] <= hi[k] {
            if hi[k] - lo[k] > tolerance {
                return None;
            }
            out[k] = (lo[k] + hi[k]) / 2.0;
        }
    }
    Some(out)
}

/// Whether two transforms have the same linear part (to 1e-9, relative).
fn same_linear(a: &Mat, b: &Mat) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() <= 1e-9 * (1.0 + a[k].abs()))
}

/// Drop something big away from the interface thread: the tree a repaint
/// replaces is tens of thousands of small allocations.
fn retire<T: Send + 'static>(garbage: T) {
    static BIN: LazyLock<Option<Mutex<mpsc::Sender<Box<dyn Send>>>>> = LazyLock::new(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn Send>>();
        std::thread::Builder::new()
            .name("coxeter-canvas-retire".into())
            .spawn(move || {
                for garbage in rx {
                    drop(garbage);
                }
            })
            .ok()?;
        Some(Mutex::new(tx))
    });
    match BIN.as_ref() {
        Some(bin) => {
            // A closed bin hands the garbage back, dropped here.
            let _ = bin.lock().unwrap_or_else(|e| e.into_inner()).send(Box::new(garbage));
        }
        None => drop(garbage),
    }
}

/// The elements a hover on `h` can restyle: for each of `h` and its ancestors
/// that a `:hover` rule of the stylesheet could match, its subtree, and the
/// following siblings' subtrees when the rule goes on with `+` or `~`.
pub fn hover_scope(doc: &Doc, h: usize) -> Vec<u32> {
    use coxeter_render::css::{Combinator, Pseudo, Rule, map_stylesheet};
    let sheet = map_stylesheet();
    let mut out: Vec<u32> = Vec::new();
    for a in doc.ancestors(h) {
        if a == 0 {
            break;
        }
        let el = doc.element(a);
        let mut subtree = false;
        let mut siblings = false;
        for rule in &sheet.rules {
            let Rule::Style(rule) = rule else { continue };
            for sel in &rule.selectors {
                for (k, (_, compound)) in sel.parts.iter().enumerate() {
                    if !compound.pseudos.iter().any(|p| matches!(p, Pseudo::Hover)) {
                        continue;
                    }
                    if compound_matches(compound, el) {
                        subtree = true;
                        if let Some((comb, _)) = sel.parts.get(k + 1)
                            && matches!(comb, Combinator::Adjacent | Combinator::Sibling) {
                                siblings = true;
                            }
                    }
                }
            }
        }
        if subtree {
            let end = doc.nodes[a].end as usize;
            out.extend((a..end).map(|i| i as u32));
            if siblings
                && let Some(p) = doc.parent(a) {
                    for s in doc.children(p).filter(|s| *s > a) {
                        let end = doc.nodes[s].end as usize;
                        out.extend((s..end).map(|i| i as u32));
                    }
                }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether an element matches a compound selector, `:hover` taken as true.
fn compound_matches(c: &coxeter_render::css::Compound, e: &Element) -> bool {
    use coxeter_render::css::{AttrOp, Pseudo};
    if c.tag.as_deref().is_some_and(|t| t != e.name) {
        return false;
    }
    if !c.ids.iter().all(|id| e.get("id") == Some(id.as_str())) {
        return false;
    }
    if !c.classes.iter().all(|cl| e.has_class(cl)) {
        return false;
    }
    for (name, op) in &c.attrs {
        let Some(v) = e.get(name) else { return false };
        let ok = match op {
            AttrOp::Exists => true,
            AttrOp::Equals(x) => v == x,
            AttrOp::Includes(x) => v.split_ascii_whitespace().any(|w| w == x),
            AttrOp::DashMatch(x) => v == x || v.starts_with(&format!("{x}-")),
            AttrOp::Prefix(x) => !x.is_empty() && v.starts_with(x.as_str()),
            AttrOp::Suffix(x) => !x.is_empty() && v.ends_with(x.as_str()),
            AttrOp::Substring(x) => !x.is_empty() && v.contains(x.as_str()),
        };
        if !ok {
            return false;
        }
    }
    c.pseudos.iter().all(|p| match p {
        Pseudo::Hover => true,
        Pseudo::Root => false,
        Pseudo::Not(list) => !list.iter().any(|x| compound_matches(x, e)),
        Pseudo::Is(list, _) => list.iter().any(|x| compound_matches(x, e)),
        Pseudo::Never(_) => false,
    })
}

/// The bounds of a path's control polygon.
pub fn path_bounds(path: &Path) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut add = |p: lyon_tessellation::math::Point| {
        b[0] = b[0].min(p.x as f64);
        b[1] = b[1].min(p.y as f64);
        b[2] = b[2].max(p.x as f64);
        b[3] = b[3].max(p.y as f64);
    };
    for ev in path.iter() {
        match ev {
            Event::Begin { at } => add(at),
            Event::Line { to, .. } => add(to),
            Event::Quadratic { ctrl, to, .. } => {
                add(ctrl);
                add(to)
            }
            Event::Cubic { ctrl1, ctrl2, to, .. } => {
                add(ctrl1);
                add(ctrl2);
                add(to)
            }
            Event::End { .. } => {}
        }
    }
    b
}

/// A path's sub-paths as polylines (map coordinates), each with whether it is closed.
pub fn flatten(path: &Path) -> Vec<(Vec<[f64; 2]>, bool)> {
    let mut out = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for ev in path.iter().flattened(TOLERANCE) {
        match ev {
            Event::Begin { at } => {
                cur.clear();
                cur.push([at.x as f64, at.y as f64]);
            }
            Event::Line { to, .. } => cur.push([to.x as f64, to.y as f64]),
            Event::End { close, .. } => out.push((std::mem::take(&mut cur), close)),
            _ => {}
        }
    }
    out
}

/// Clip a convex polygon to `p[axis] >= at` (`keep_above`) or `<= at`.
fn clip_half_plane(poly: &[[f64; 2]], axis: usize, at: f64, keep_above: bool) -> Vec<[f64; 2]> {
    let inside = |p: &[f64; 2]| if keep_above { p[axis] >= at } else { p[axis] <= at };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for k in 0..poly.len() {
        let a = poly[k];
        let b = poly[(k + 1) % poly.len()];
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = (at - a[axis]) / (b[axis] - a[axis]);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

/// `text-anchor` of a style (for overlays that place text the same way).
pub fn anchor_offset(anchor: Anchor, width: f64) -> f64 {
    match anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => -width / 2.0,
        Anchor::End => -width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use coxeter_core::mapdata::Point;
    use coxeter_render::format::n;

    /// A circle, a curve, a label and a turned marker, moved by `(dx, dy)`.
    fn tree(dx: f64, dy: f64, r: &str) -> Element {
        let p = |x: f64, y: f64| format!("{} {}", n(x + dx), n(y + dy));
        Element::paired("svg").with("width", "400").with("height", "300").child(
            Element::paired("g")
                .with("class", "emc-layer")
                .child(
                    Element::new("circle")
                        .with("cx", n(40.0 + dx))
                        .with("cy", n(50.0 + dy))
                        .with("r", r.to_string())
                        .with("fill", "#dc2626")
                        .with("stroke", "#0f172a")
                        .with("stroke-width", "2"),
                )
                .child(
                    Element::new("path")
                        .with("d", format!("M {} C {}, {}, {}", p(60.004, 70.006), p(90.333, 120.667), p(130.111, 20.889), p(170.5, 90.25)))
                        .with("fill", "none")
                        .with("stroke", "#2563eb")
                        .with("stroke-width", "3"),
                )
                .child(Element::paired("text").with("x", n(120.0 + dx)).with("y", n(180.0 + dy)).with("font-size", "12").with("fill", "#0f172a").text("glc__D"))
                .child(
                    Element::paired("g").with("transform", format!("translate({}) rotate(30)", p(250.0, 150.0))).child(
                        Element::new("rect").with("x", "-6").with("y", "-3").with("width", "12").with("height", "6").with("fill", "#16a34a"),
                    ),
                ),
        )
    }

    /// Two scenes draw the same: the same meshes per element, vertex for
    /// vertex up to their order (a fill tessellated elsewhere may list its
    /// vertices differently), on screen at 2 px per unit.
    fn assert_same_drawing(a: &Scene, b: &Scene) {
        let cam = Camera { x: 10.0, y: 5.0, scale: 2.0, fitted: true };
        let view = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let (ma, na) = crate::painter::screen_mesh(a, &cam, egui::Pos2::ZERO, view);
        let (mb, nb) = crate::painter::screen_mesh(b, &cam, egui::Pos2::ZERO, view);
        assert_eq!((na, ma.vertices.len(), ma.indices.len()), (nb, mb.vertices.len(), mb.indices.len()));
        // Printed to hundredths: within half of one.
        let nearest = |v: &egui::epaint::Vertex, m: &egui::epaint::Mesh| {
            m.vertices.iter().filter(|u| u.color == v.color).map(|u| (u.pos - v.pos).length()).fold(f32::INFINITY, f32::min)
        };
        let worst = ma.vertices.iter().map(|v| nearest(v, &mb)).chain(mb.vertices.iter().map(|v| nearest(v, &ma))).fold(0.0f32, f32::max);
        assert!(worst < 0.02, "vertices {worst} px apart");
    }

    #[test]
    fn a_moved_element_is_shifted_not_tessellated_again() {
        let mut scene = Scene::build(tree(0.0, 0.0, "5"), PageTheme::light(), 2.0);
        // A delta the printed hundredths round unevenly.
        let (dx, dy) = (23.337, -11.502);
        scene.update(tree(dx, dy, "5"));
        let s = scene.stats;
        assert_eq!(s.rebuilt, 5, "the four that moved and the rect under one");
        assert_eq!(s.shifted, s.rebuilt, "nothing tessellated again");
        let circle = (0..scene.doc.len()).find(|&i| scene.doc.element(i).name == "circle").unwrap();
        let [sx, sy] = scene.elems[circle].shift;
        assert!((sx - dx).abs() < 0.006 && (sy - dy).abs() < 0.006, "{sx} {sy}");
        // What a scene made from the moved tree draws, and where it is hit.
        let fresh = Scene::build(tree(dx, dy, "5"), PageTheme::light(), 2.0);
        assert_same_drawing(&scene, &fresh);
        let filter = crate::hit::HitFilter::default();
        for s in [&scene, &fresh] {
            assert_eq!(crate::hit::hit_test(s, Point::new(40.0 + dx, 50.0 + dy), filter), Some(circle));
            assert_ne!(crate::hit::hit_test(s, Point::new(40.0, 50.0), filter), Some(circle));
        }
        // Moved again: measured from where the meshes were made.
        let (dx2, dy2) = (-7.019, 30.333);
        scene.update(tree(dx2, dy2, "5"));
        assert_eq!(scene.stats.shifted, 5);
        assert_same_drawing(&scene, &Scene::build(tree(dx2, dy2, "5"), PageTheme::light(), 2.0));
        // A circle that grows did not only move: it is tessellated again.
        scene.update(tree(dx2, dy2, "7"));
        assert_eq!((scene.stats.rebuilt, scene.stats.shifted), (1, 0));
        assert_eq!(scene.elems[circle].shift, [0.0, 0.0]);
        assert_same_drawing(&scene, &Scene::build(tree(dx2, dy2, "7"), PageTheme::light(), 2.0));
    }

    #[test]
    fn translations_allow_for_printed_hundredths() {
        let a = placement(&Element::new("path").with("d", "M 10 10 L 20.004 30 H 5")).unwrap();
        let b = placement(&Element::new("path").with("d", "M 13.34 9 L 23.34 29 H 8.34")).unwrap();
        let t = translation(&a, &b).unwrap();
        assert!((t[0] - 3.338).abs() < 0.01 && (t[1] + 1.0).abs() < 1e-9, "{t:?}");
        let bent = placement(&Element::new("path").with("d", "M 13.34 9 L 23.5 29 H 8.34")).unwrap();
        assert_eq!(translation(&a, &bent), None);
        let other = placement(&Element::new("path").with("d", "M 13.34 9 C 1 1, 2 2, 23.34 29")).unwrap();
        assert_eq!(translation(&a, &other), None);
    }
}
