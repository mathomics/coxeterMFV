//! Cross-feeding line colours (`src/render/crossfeed-colors.js`): each
//! exchanged metabolite has one colour, the same on every map, so a colour
//! names a metabolite wherever its lines go.
//!
//! The exchanges that matter most between species (the short-chain fatty acids,
//! lactate, the other fermentation products, glycerol and glucose) each have a
//! colour no other metabolite gets: acetate is always blue, glycerol always red.
//! Sugars, amino acids and the other common exchanges share a second set of
//! colours, and any other metabolite is placed in that set by a hash of its id.
//! On one map two metabolites that would share a colour are told apart: the one
//! later in the order below takes the next free colour of the shared set.
//!
//! Every colour has a lighter variant for the dark theme, and both keep at least
//! 3:1 contrast with the shared medium they are drawn over.

use std::cmp::Ordering;
use std::collections::HashSet;

use coxeter_core::js;
use indexmap::IndexMap;

/// (name, light, dark) per colour.
const PALETTE: [(&str, &str, &str); 20] = [
    ("blue", "#2563eb", "#60a5fa"),
    ("red", "#dc2626", "#f87171"),
    ("green", "#16a34a", "#4ade80"),
    ("lime", "#4d7c0f", "#bef264"),
    ("orange", "#ea580c", "#fb923c"),
    ("violet", "#7c3aed", "#a78bfa"),
    ("fuchsia", "#c026d3", "#e879f9"),
    ("teal", "#0d9488", "#2dd4bf"),
    ("gold", "#a16207", "#facc15"),
    ("sky", "#0284c7", "#7dd3fc"),
    ("slate", "#475569", "#cbd5e1"),
    ("brown", "#92400e", "#d6a77a"),
    ("indigo", "#3730a3", "#a5b4fc"),
    // the shared set
    ("rose", "#be123c", "#fda4af"),
    ("pink", "#db2777", "#f472b6"),
    ("cyan", "#155e75", "#67e8f9"),
    ("emerald", "#065f46", "#bbf7d0"),
    ("maroon", "#7f1d1d", "#fed7aa"),
    ("plum", "#701a75", "#f5d0fe"),
    ("khaki", "#7c6f2a", "#fef08a"),
];

const SHARED: [&str; 7] = ["rose", "pink", "cyan", "emerald", "maroon", "plum", "khaki"];

/// Metabolites with a colour of their own, by key (see `metabolite_key`), with
/// the BiGG and ModelSEED spellings of each.
const OWN: &[(&str, &str)] = &[
    ("ac", "blue"), ("cpd00029", "blue"),
    ("glyc", "red"), ("cpd00100", "red"),
    ("lac", "green"), ("lacl", "green"), ("cpd00159", "green"),
    ("lacd", "lime"), ("cpd00221", "lime"),
    ("but", "orange"), ("cpd00211", "orange"),
    ("ppa", "violet"), ("cpd00141", "violet"),
    ("succ", "fuchsia"), ("cpd00036", "fuchsia"),
    ("for", "teal"), ("cpd00047", "teal"),
    ("etoh", "gold"), ("cpd00363", "gold"),
    ("h2", "sky"), ("cpd11640", "sky"),
    ("co2", "slate"), ("cpd00011", "slate"),
    ("pyr", "brown"), ("cpd00020", "brown"),
    ("glcd", "indigo"), ("cpd00027", "indigo"),
];

/// Common exchanges given a colour of the shared set. Within each group the
/// ones most often exchanged together come first and get different colours.
const SHARED_TABLE: &[(&str, &str)] = &[
    // sugars and other carbon sources
    ("fru", "rose"), ("gal", "pink"), ("man", "cyan"), ("xyld", "emerald"), ("arabl", "maroon"), ("ribd", "plum"), ("cellb", "khaki"),
    ("malt", "rose"), ("sucr", "pink"), ("lcts", "cyan"), ("rmn", "emerald"), ("fucl", "maroon"), ("acgam", "plum"), ("glcur", "khaki"),
    ("galur", "rose"),
    // amino acids
    ("glul", "pink"), ("glnl", "cyan"), ("aspl", "emerald"), ("asnl", "maroon"), ("alal", "plum"), ("gly", "khaki"), ("serl", "rose"),
    ("thrl", "pink"), ("prol", "cyan"), ("vall", "emerald"), ("leul", "maroon"), ("ilel", "plum"), ("metl", "khaki"), ("cysl", "rose"),
    ("lysl", "pink"), ("argl", "cyan"), ("hisl", "emerald"), ("phel", "maroon"), ("tyrl", "plum"), ("trpl", "khaki"),
    // other fermentation products and common exchanges
    ("nh4", "cyan"), ("fum", "emerald"), ("mall", "maroon"), ("cit", "plum"), ("acald", "khaki"), ("meoh", "rose"),
    ("12ppds", "pink"), ("isobut", "cyan"), ("isoval", "emerald"), ("2mba", "maroon"), ("ch4", "plum"), ("h2s", "khaki"),
];

/// The id a table entry matches: lower case, letters and digits only, so
/// lac__L, lac_L and lac-L are one.
fn metabolite_key(base: &str) -> String {
    base.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

// The same string hash as for unknown compartments, so it is stable everywhere.
fn stable_hash(text: &str) -> u32 {
    text.encode_utf16().fold(0u32, |h, unit| h.wrapping_mul(31).wrapping_add(unit as u32))
}

/// The colour a metabolite asks for, and its tier: 0 its own, 1 shared from
/// the table, 2 hashed.
fn color_name(base: &str) -> (u8, &'static str) {
    let key = metabolite_key(base);
    let find = |table: &[(&str, &'static str)]| table.iter().find(|(k, _)| *k == key).map(|&(_, name)| name);
    if let Some(name) = find(OWN) {
        return (0, name);
    }
    if let Some(name) = find(SHARED_TABLE) {
        return (1, name);
    }
    (2, SHARED[(stable_hash(&key) % SHARED.len() as u32) as usize])
}

fn hex(name: &str, dark: bool) -> &'static str {
    let &(_, light, dark_hex) = PALETTE.iter().find(|(n, _, _)| *n == name).expect("every colour name is in the palette");
    if dark { dark_hex } else { light }
}

/// `crossFeedingColor(base, dark)`: the colour of a metabolite's cross-feeding
/// lines taken on its own.
pub fn cross_feeding_color(base: &str, dark: bool) -> &'static str {
    hex(color_name(base).1, dark)
}

/// `crossFeedingColors(bases, dark)`: the colours of the metabolites drawn
/// together on one map, from base to colour: each its own colour
/// ([`cross_feeding_color`]), except that a colour already taken sends the
/// later metabolite to the next free colour of the shared set. Metabolites with
/// a colour of their own come first, then the table's, then the rest, each in
/// id order; flux plays no part, so a new solution does not repaint the lines.
/// Once every shared colour is taken, colours repeat.
pub fn cross_feeding_colors<I, S>(bases: I, dark: bool) -> IndexMap<String, &'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = HashSet::new();
    let mut entries: Vec<(String, u8, &'static str)> = Vec::new();
    for base in bases {
        let base = base.as_ref();
        if seen.insert(base.to_string()) {
            let (tier, name) = color_name(base);
            entries.push((base.to_string(), tier, name));
        }
    }
    entries.sort_by(|a, b| match a.1.cmp(&b.1) {
        Ordering::Equal => js::code_unit_compare(&a.0, &b.0),
        other => other,
    });
    let mut used: HashSet<&'static str> = HashSet::new();
    let mut out = IndexMap::new();
    for (base, tier, wanted) in entries {
        let mut name = wanted;
        if used.contains(name) {
            let from = if tier == 0 { 0 } else { SHARED.iter().position(|s| *s == name).map_or(0, |i| i + 1) };
            if let Some(next) = (0..SHARED.len()).map(|k| SHARED[(from + k) % SHARED.len()]).find(|c| !used.contains(c)) {
                name = next;
            }
        }
        used.insert(name);
        out.insert(base, hex(name, dark));
    }
    out
}
