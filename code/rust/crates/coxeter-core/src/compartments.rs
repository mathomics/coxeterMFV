//! Compartment registry.
//!
//! Every compartment the renderer draws gets an explicit visual identity: a
//! fill, a border (the "membrane"), a text colour and a short label. Colours
//! are chosen so that neighbouring compartments never share a hue and so that
//! every fill/text pair clears WCAG AA at 14px.
//!
//! Compartment ids follow the BiGG convention (`c`, `e`, `p`, `m`, ...). Unknown
//! ids fall back to a deterministic slot in [`EXTRA_PALETTE`] so that a
//! user-supplied model still renders with stable, distinct colours.

use std::collections::HashSet;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::js;

/// Compartment names keyed by id, as a model supplies them.
pub type Names = IndexMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompartmentTheme {
    pub fill: &'static str,
    pub fill_dark: &'static str,
    pub stroke: &'static str,
    pub stroke_dark: &'static str,
    pub text: &'static str,
    pub text_dark: &'static str,
}

const fn t(
    fill: &'static str,
    fill_dark: &'static str,
    stroke: &'static str,
    stroke_dark: &'static str,
    text: &'static str,
    text_dark: &'static str,
) -> CompartmentTheme {
    CompartmentTheme { fill, fill_dark, stroke, stroke_dark, text, text_dark }
}

/// How the boundary of a compartment is stroked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Membrane {
    /// Double stroke, for true lipid membranes.
    Bilayer,
    /// Thick single stroke with a hatch, for cell walls.
    Wall,
    /// Dashed stroke, for spaces that are not membrane-bound.
    Open,
}

impl Membrane {
    pub fn as_str(self) -> &'static str {
        match self {
            Membrane::Bilayer => "bilayer",
            Membrane::Wall => "wall",
            Membrane::Open => "open",
        }
    }
    /// The style a saved name (`bilayer`, `wall`, `open`) stands for.
    pub fn parse(s: &str) -> Option<Membrane> {
        match s {
            "bilayer" => Some(Membrane::Bilayer),
            "wall" => Some(Membrane::Wall),
            "open" => Some(Membrane::Open),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompartmentInfo {
    pub id: String,
    pub name: String,
    pub es: String,
    pub membrane: Membrane,
    pub theme: CompartmentTheme,
}

struct Canonical {
    id: &'static str,
    name: &'static str,
    es: &'static str,
    membrane: Membrane,
    theme: CompartmentTheme,
}

use Membrane::{Bilayer, Open, Wall};

/// Canonical compartments, in display order.
const COMPARTMENTS: [Canonical; 19] = [
    Canonical { id: "e", name: "Extracellular", es: "Extracelular", membrane: Open, theme: t("#f1f5f9", "#1e293b", "#94a3b8", "#64748b", "#475569", "#cbd5e1") },
    Canonical { id: "s", name: "Shared medium", es: "Medio compartido", membrane: Open, theme: t("#f8fafc", "#0f172a", "#cbd5e1", "#475569", "#64748b", "#94a3b8") },
    Canonical { id: "w", name: "Cell wall", es: "Pared celular", membrane: Wall, theme: t("#f5f5f4", "#292524", "#a8a29e", "#78716c", "#57534e", "#d6d3d1") },
    Canonical { id: "p", name: "Periplasm", es: "Periplasma", membrane: Bilayer, theme: t("#fef3c7", "#422006", "#d97706", "#b45309", "#92400e", "#fcd34d") },
    Canonical { id: "c", name: "Cytosol", es: "Citosol", membrane: Bilayer, theme: t("#e0f2fe", "#082f49", "#0284c7", "#0369a1", "#075985", "#7dd3fc") },
    Canonical { id: "im", name: "Intermembrane space", es: "Espacio intermembrana", membrane: Bilayer, theme: t("#ffe4e6", "#4c0519", "#fb7185", "#e11d48", "#9f1239", "#fda4af") },
    Canonical { id: "m", name: "Mitochondrial matrix", es: "Matriz mitocondrial", membrane: Bilayer, theme: t("#fee2e2", "#450a0a", "#dc2626", "#b91c1c", "#991b1b", "#fca5a5") },
    Canonical { id: "h", name: "Chloroplast stroma", es: "Estroma del cloroplasto", membrane: Bilayer, theme: t("#dcfce7", "#052e16", "#16a34a", "#15803d", "#166534", "#86efac") },
    Canonical { id: "u", name: "Thylakoid lumen", es: "Lumen tilacoidal", membrane: Bilayer, theme: t("#d1fae5", "#022c22", "#059669", "#047857", "#065f46", "#6ee7b7") },
    Canonical { id: "n", name: "Nucleus", es: "Núcleo", membrane: Bilayer, theme: t("#ede9fe", "#2e1065", "#7c3aed", "#6d28d9", "#5b21b6", "#c4b5fd") },
    Canonical { id: "x", name: "Peroxisome", es: "Peroxisoma", membrane: Bilayer, theme: t("#ffedd5", "#431407", "#ea580c", "#c2410c", "#9a3412", "#fdba74") },
    Canonical { id: "r", name: "Endoplasmic reticulum", es: "Retículo endoplásmico", membrane: Bilayer, theme: t("#fae8ff", "#4a044e", "#c026d3", "#a21caf", "#86198f", "#f0abfc") },
    Canonical { id: "g", name: "Golgi apparatus", es: "Aparato de Golgi", membrane: Bilayer, theme: t("#fce7f3", "#500724", "#db2777", "#be185d", "#9d174d", "#f9a8d4") },
    Canonical { id: "v", name: "Vacuole", es: "Vacuola", membrane: Bilayer, theme: t("#cffafe", "#083344", "#0891b2", "#0e7490", "#155e75", "#67e8f9") },
    Canonical { id: "l", name: "Lysosome", es: "Lisosoma", membrane: Bilayer, theme: t("#ecfccb", "#1a2e05", "#65a30d", "#4d7c0f", "#3f6212", "#bef264") },
    Canonical { id: "cx", name: "Carboxysome", es: "Carboxisoma", membrane: Bilayer, theme: t("#e0e7ff", "#1e1b4b", "#4f46e5", "#4338ca", "#3730a3", "#a5b4fc") },
    Canonical { id: "f", name: "Flagellum", es: "Flagelo", membrane: Open, theme: t("#f0fdfa", "#042f2e", "#14b8a6", "#0d9488", "#0f766e", "#5eead4") },
    Canonical { id: "lp", name: "Lipid particle", es: "Partícula lipídica", membrane: Bilayer, theme: t("#fef9c3", "#422006", "#ca8a04", "#a16207", "#854d0e", "#fde047") },
    // BiGG writes the eyespot as `s`, which between cells is the shared medium.
    // Inside a cell it is drawn under this id (see cell_compartment_visual_id()).
    Canonical { id: "es", name: "Eyespot", es: "Mancha ocular", membrane: Bilayer, theme: t("#ffe4d6", "#431407", "#f97316", "#c2410c", "#9a3412", "#fdba74") },
];

/// Fallback hues for compartment ids that are not in the canonical table.
pub const EXTRA_PALETTE: [CompartmentTheme; 6] = [
    t("#e2e8f0", "#1e293b", "#475569", "#334155", "#334155", "#cbd5e1"),
    t("#fff7ed", "#431407", "#f97316", "#c2410c", "#9a3412", "#fdba74"),
    t("#eff6ff", "#172554", "#3b82f6", "#1d4ed8", "#1e40af", "#93c5fd"),
    t("#f0fdf4", "#052e16", "#22c55e", "#15803d", "#166534", "#86efac"),
    t("#fdf2f8", "#500724", "#ec4899", "#be185d", "#9d174d", "#f9a8d4"),
    t("#fefce8", "#422006", "#eab308", "#a16207", "#854d0e", "#fde047"),
];

/// Default nesting used when a model does not declare `compartment_tree`:
/// compartment id → the compartment that physically contains it. `None` means
/// directly inside the cell boundary.
pub const DEFAULT_PARENT: &[(&str, Option<&str>)] = &[
    ("e", None),
    ("w", Some("e")),
    ("p", Some("w")),
    ("c", Some("p")),
    ("n", Some("c")),
    ("x", Some("c")),
    ("r", Some("c")),
    ("g", Some("c")),
    ("v", Some("c")),
    ("l", Some("c")),
    ("cx", Some("c")),
    // A cell's own `s` is BiGG's eyespot. The shared medium never appears in a
    // species' tree, so there is no second meaning to keep apart here.
    ("s", Some("c")),
    // The space between the two mitochondrial membranes has two BiGG
    // spellings: `im`, and `i` in the algal and human reconstructions. Either
    // way the matrix sits inside it. Chained, so a model that declares only
    // one of them still nests correctly.
    ("i", Some("c")),
    ("im", Some("i")),
    ("m", Some("im")),
    ("h", Some("c")),
    ("u", Some("h")),
    ("f", None),
];

/// Yeast9 uses a different compartment vocabulary from BiGG for several
/// organelles (notably `p` = peroxisome, where BiGG uses `p` for periplasm).
/// Current Yeast9 / yeast-GEM 9.1.x exposes 14 compartment ids.
pub const YEAST9_PARENT: &[(&str, Option<&str>)] = &[
    ("e", None),
    ("ce", Some("e")),
    ("c", Some("ce")),
    ("erm", Some("c")),
    ("er", Some("erm")),
    ("gm", Some("c")),
    ("g", Some("gm")),
    ("lp", Some("c")),
    ("mm", Some("c")),
    ("m", Some("mm")),
    ("n", Some("c")),
    ("p", Some("c")),
    ("vm", Some("c")),
    ("v", Some("vm")),
];

/// Look a compartment up in one of the parent tables: `None` when the table
/// has no entry, `Some(parent)` when it has one.
fn table_parent(table: &[(&str, Option<&'static str>)], id: &str) -> Option<Option<String>> {
    table
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, p)| p.map(str::to_string))
}

static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[._-]+").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

fn normalise_compartment_label(value: &str) -> String {
    let lower = value.trim().to_lowercase();
    let spaced = SEPARATORS.replace_all(&lower, " ");
    SPACES.replace_all(&spaced, " ").into_owned()
}

fn name_of<'a>(names: &'a Names, id: &str) -> &'a str {
    names.get(id).map(String::as_str).unwrap_or("")
}

/// Resolve model-specific compartment aliases to Coxeter's visual vocabulary.
/// Separate from the biological id: Yeast9's `p` is a peroxisome, while
/// bacterial BiGG models commonly use `p` for periplasm.
pub fn compartment_visual_id(id: &str, names: &Names) -> String {
    let label = normalise_compartment_label(name_of(names, id));
    if label.contains("peroxisome") {
        return "x".into();
    }
    if label.contains("cell envelope") || label.contains("cell wall") {
        return "w".into();
    }
    if label.contains("endoplasmic reticulum") {
        return "r".into();
    }
    if label.contains("golgi") {
        return "g".into();
    }
    if label.contains("lipid particle") || label.contains("lipid droplet") {
        return "lp".into();
    }
    if label.contains("mitochond") {
        return "m".into();
    }
    if label.contains("vacuol") {
        return "v".into();
    }
    // Stable Yeast9 aliases still work when an imported model omits names.
    match id {
        "ce" => "w".into(),
        "er" | "erm" => "r".into(),
        "gm" => "g".into(),
        "mm" => "m".into(),
        "vm" => "v".into(),
        // BiGG's other spelling of the mitochondrial intermembrane space.
        "i" => "im".into(),
        _ => id.into(),
    }
}

/// The same resolution for a compartment that belongs to a cell.
///
/// `s` is the one id with two meanings. Between cells it is Coxeter's shared
/// medium; inside a BiGG algal model it is the eyespot. A cell never contains
/// the shared medium, so wherever a compartment is known to be a cell's own,
/// `s` is the organelle.
pub fn cell_compartment_visual_id(id: &str, names: &Names) -> String {
    let visual = compartment_visual_id(id, names);
    if visual == "s" && !names_shared_medium(names) { "es".into() } else { visual }
}

/// Does this set of names call `s` the shared medium? The merged community
/// model is the one model that really contains it, and it says so by name.
fn names_shared_medium(names: &Names) -> bool {
    let label = normalise_compartment_label(name_of(names, "s"));
    label.contains("shared medium") || label.contains("medio compartido")
}

/// True when the compartment ids/names match the yeast-GEM / Yeast9 topology.
pub fn is_yeast9_compartment_set<S: AsRef<str>>(ids: &[S], names: &Names) -> bool {
    let present: Vec<&str> = {
        let mut seen = HashSet::new();
        ids.iter().map(AsRef::as_ref).filter(|id| seen.insert(*id)).collect()
    };
    if !(present.contains(&"c") && present.contains(&"e") && present.contains(&"ce")) {
        return false;
    }
    let labels: Vec<String> = present
        .iter()
        .map(|id| normalise_compartment_label(names.get(*id).map(String::as_str).unwrap_or(id)))
        .collect();
    let semantic = [
        "cell envelope",
        "endoplasmic reticulum",
        "golgi",
        "lipid particle",
        "mitochondrion",
        "nucleus",
        "peroxisome",
        "vacuole",
    ];
    let hits = semantic
        .iter()
        .filter(|needle| labels.iter().any(|l| l == *needle || l.contains(*needle)))
        .count();
    // IDs are stable in yeast-GEM, while names make the detection robust to
    // imported SBML files that keep the official compartment labels.
    let id_signature = ["er", "erm", "g", "gm", "lp", "m", "mm", "n", "p", "v", "vm"]
        .iter()
        .filter(|id| present.contains(id))
        .count();
    hits >= 5 || id_signature >= 8
}

/// Named profile used by the layout engine for model-specific geometry.
pub fn compartment_layout_profile<S: AsRef<str>>(ids: &[S], names: &Names) -> &'static str {
    if is_yeast9_compartment_set(ids, names) { "yeast9" } else { "default" }
}

/// Deterministically resolve the visual definition of a compartment id.
pub fn compartment_info(id: &str) -> CompartmentInfo {
    if let Some(c) = COMPARTMENTS.iter().find(|c| c.id == id) {
        return CompartmentInfo {
            id: c.id.into(),
            name: c.name.into(),
            es: c.es.into(),
            membrane: c.membrane,
            theme: c.theme,
        };
    }
    // Hash the id so the same unknown compartment always lands on the same hue.
    let mut h: u32 = 0;
    for unit in id.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as u32);
    }
    CompartmentInfo {
        id: id.into(),
        name: id.into(),
        es: id.into(),
        membrane: Membrane::Bilayer,
        theme: EXTRA_PALETTE[(h as usize) % EXTRA_PALETTE.len()],
    }
}

/// A name the model actually supplies. An exporter with nothing to write
/// repeats the id (cobrapy turns an SBML compartment without a name into
/// `"c": "c"`), and that is an absent name, not a compartment called "c".
fn supplied_name<'a>(id: &str, names: &'a Names) -> Option<&'a str> {
    let name = names.get(id)?;
    let trimmed = name.trim();
    (!trimmed.is_empty() && trimmed != id).then_some(name.as_str())
}

fn registry_name(visual_id: &str, locale: &str) -> String {
    let info = compartment_info(visual_id);
    if locale == "es" { info.es } else { info.name }
}

/// Human readable compartment name in the requested locale (`"en"`/`"es"`).
pub fn compartment_name(id: &str, names: &Names, locale: &str) -> String {
    supplied_name(id, names)
        .map(str::to_string)
        .unwrap_or_else(|| registry_name(&compartment_visual_id(id, names), locale))
}

/// Human readable name of a compartment that belongs to a cell.
pub fn cell_compartment_name(id: &str, names: &Names, locale: &str) -> String {
    supplied_name(id, names)
        .map(str::to_string)
        .unwrap_or_else(|| registry_name(&cell_compartment_visual_id(id, names), locale))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompartmentColors {
    pub fill: &'static str,
    pub stroke: &'static str,
    pub text: &'static str,
}

/// The theme colours for the active colour scheme.
pub fn compartment_colors(id: &str, dark: bool, names: &Names) -> CompartmentColors {
    let theme = compartment_info(&compartment_visual_id(id, names)).theme;
    if dark {
        CompartmentColors { fill: theme.fill_dark, stroke: theme.stroke_dark, text: theme.text_dark }
    } else {
        CompartmentColors { fill: theme.fill, stroke: theme.stroke, text: theme.text }
    }
}

/// How the boundary of this compartment should be stroked.
pub fn membrane_style(id: &str, names: &Names) -> Membrane {
    compartment_info(&compartment_visual_id(id, names)).membrane
}

/// One node of a compartment containment tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub id: String,
    pub children: Vec<String>,
    pub parent: Option<String>,
    pub depth: usize,
}

/// A containment tree: `by_id` in the order the ids were given, `roots` and
/// every `children` list sorted with `localeCompare`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompartmentTree {
    pub roots: Vec<String>,
    pub by_id: IndexMap<String, TreeNode>,
}

impl CompartmentTree {
    pub fn get(&self, id: &str) -> Option<&TreeNode> {
        self.by_id.get(id)
    }
}

/// Build a containment tree for a set of compartment ids.
///
/// `explicit` is the model's own `compartment_tree` (child → parent or null).
pub fn build_compartment_tree<S: AsRef<str>>(
    ids: &[S],
    explicit: Option<&IndexMap<String, Option<String>>>,
    names: &Names,
) -> CompartmentTree {
    let mut present: IndexMap<String, ()> = IndexMap::new();
    for id in ids {
        present.insert(id.as_ref().to_string(), ());
    }
    let id_list: Vec<String> = present.keys().cloned().collect();
    let yeast = is_yeast9_compartment_set(&id_list, names);
    let defaults: &[(&str, Option<&'static str>)] = if yeast { YEAST9_PARENT } else { DEFAULT_PARENT };
    // The shared medium, where a model has one, contains the cells rather
    // than sitting in one of them.
    let shared_medium = names_shared_medium(names);

    let mut parent_of: IndexMap<String, Option<String>> = IndexMap::new();
    for id in &id_list {
        let mut parent: Option<String> = match explicit.and_then(|e| e.get(id)) {
            Some(p) => p.clone(),
            None => {
                if id == "s" && shared_medium {
                    None
                } else {
                    table_parent(defaults, id).flatten()
                }
            }
        };
        // Walk up the default chain until a parent exists in this model. A
        // model with c + m but no `im` nests m directly in c.
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(p) = parent.clone() {
            if p.is_empty() || present.contains_key(&p) {
                break;
            }
            if !seen.insert(p.clone()) {
                parent = None;
                break;
            }
            parent = table_parent(defaults, &p).flatten();
        }
        let parent = parent.filter(|p| !p.is_empty() && p != id);
        parent_of.insert(id.clone(), parent);
    }

    // Break accidental cycles so the layout never recurses forever.
    for id in &id_list {
        let mut chain: HashSet<String> = HashSet::from([id.clone()]);
        let mut p = parent_of.get(id).cloned().flatten();
        while let Some(cur) = p {
            if chain.contains(&cur) {
                parent_of.insert(id.clone(), None);
                break;
            }
            chain.insert(cur.clone());
            p = parent_of.get(&cur).cloned().flatten();
        }
    }

    let mut by_id: IndexMap<String, TreeNode> = IndexMap::new();
    for id in &id_list {
        by_id.insert(
            id.clone(),
            TreeNode { id: id.clone(), children: vec![], parent: parent_of.get(id).cloned().flatten(), depth: 0 },
        );
    }
    let mut roots = Vec::new();
    for id in &id_list {
        let parent = by_id[id].parent.clone();
        match parent {
            Some(p) if by_id.contains_key(&p) => by_id[&p].children.push(id.clone()),
            _ => {
                by_id[id].parent = None;
                roots.push(id.clone());
            }
        }
    }

    fn set_depth(by_id: &mut IndexMap<String, TreeNode>, id: &str, d: usize) {
        by_id[id].depth = d;
        let children = by_id[id].children.clone();
        for ch in children {
            set_depth(by_id, &ch, d + 1);
        }
    }
    for r in roots.clone() {
        set_depth(&mut by_id, &r, 0);
    }

    // Stable ordering by localeCompare, recursively.
    roots.sort_by(|a, b| js::locale_compare(a, b));
    for node in by_id.values_mut() {
        node.children.sort_by(|a, b| js::locale_compare(a, b));
    }
    CompartmentTree { roots, by_id }
}

/// All compartment ids known to the registry, in display order.
pub fn known_compartment_ids() -> Vec<&'static str> {
    COMPARTMENTS.iter().map(|c| c.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(pairs: &[(&str, &str)]) -> Names {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn default_tree_skips_missing_levels() {
        let tree = build_compartment_tree(&["c", "e", "m"], None, &Names::new());
        assert_eq!(tree.roots, ["e"]);
        assert_eq!(tree.get("c").unwrap().parent.as_deref(), Some("e"));
        assert_eq!(tree.get("m").unwrap().parent.as_deref(), Some("c"));
        assert_eq!(tree.get("m").unwrap().depth, 2);
    }

    #[test]
    fn yeast9_is_recognised() {
        let ids = ["c", "ce", "e", "er", "erm", "g", "gm", "lp", "m", "mm", "n", "p", "v", "vm"];
        let tree = build_compartment_tree(&ids, None, &Names::new());
        assert_eq!(tree.get("p").unwrap().parent.as_deref(), Some("c"));
        assert_eq!(tree.get("c").unwrap().parent.as_deref(), Some("ce"));
        assert_eq!(compartment_visual_id("p", &names(&[("p", "peroxisome")])), "x");
    }

    #[test]
    fn eyespot_versus_medium() {
        assert_eq!(cell_compartment_visual_id("s", &Names::new()), "es");
        assert_eq!(cell_compartment_visual_id("s", &names(&[("s", "Shared medium")])), "s");
        assert_eq!(compartment_name("c", &names(&[("c", "c")]), "en"), "Cytosol");
        assert_eq!(compartment_name("c", &names(&[("c", "cytoplasm")]), "en"), "cytoplasm");
    }

    #[test]
    fn unknown_ids_get_a_stable_palette_slot() {
        // JS: for "zz": h = 122*31 + 122 = 3904; 3904 % 6 = 4.
        assert_eq!(compartment_info("zz").theme, EXTRA_PALETTE[4]);
    }
}
