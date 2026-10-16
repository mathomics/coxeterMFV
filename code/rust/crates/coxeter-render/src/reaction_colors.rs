//! Categorical reaction colouring for subsystem/pathway views
//! (`src/render/reaction-colors.js`).
//!
//! Flux remains encoded by stroke width. These helpers only choose the colour,
//! keeping visual grouping completely separate from the biological model and
//! the solver.

use std::collections::HashSet;
use std::sync::LazyLock;

use coxeter_core::js;
use coxeter_core::model::{Model, Reaction};
use indexmap::IndexMap;
use regex::Regex;

use crate::format::js_trim;

const LIGHT: [&str; 16] = [
    "#2563eb", "#dc2626", "#059669", "#7c3aed", "#d97706", "#0891b2", "#db2777", "#65a30d", "#4f46e5", "#c2410c",
    "#0f766e", "#9333ea", "#0284c7", "#be123c", "#15803d", "#a16207",
];

const DARK: [&str; 16] = [
    "#60a5fa", "#f87171", "#34d399", "#c4b5fd", "#fbbf24", "#22d3ee", "#f472b6", "#a3e635", "#818cf8", "#fb923c",
    "#2dd4bf", "#d8b4fe", "#38bdf8", "#fb7185", "#4ade80", "#facc15",
];

/// What colours the reactions: their flux, or the group they belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReactionColorMode {
    #[default]
    Flux,
    Subsystem,
    Pathway,
}

impl ReactionColorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReactionColorMode::Flux => "flux",
            ReactionColorMode::Subsystem => "subsystem",
            ReactionColorMode::Pathway => "pathway",
        }
    }
}

/// `normalizeReactionColorMode(mode)`: anything unknown is `flux`.
pub fn normalize_reaction_color_mode(mode: &str) -> ReactionColorMode {
    match mode {
        "subsystem" => ReactionColorMode::Subsystem,
        "pathway" => ReactionColorMode::Pathway,
        _ => ReactionColorMode::Flux,
    }
}

/// Which grouping the translucent ribbons under the reactions follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PathwayRibbonMode {
    #[default]
    Off,
    Subsystem,
    Pathway,
}

impl PathwayRibbonMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PathwayRibbonMode::Off => "off",
            PathwayRibbonMode::Subsystem => "subsystem",
            PathwayRibbonMode::Pathway => "pathway",
        }
    }
}

/// `normalizePathwayRibbonMode(mode)`: anything unknown is `off`.
pub fn normalize_pathway_ribbon_mode(mode: &str) -> PathwayRibbonMode {
    match mode {
        "subsystem" => PathwayRibbonMode::Subsystem,
        "pathway" => PathwayRibbonMode::Pathway,
        _ => PathwayRibbonMode::Off,
    }
}

/// `reactionCategory(reaction, mode)`: the group a reaction is coloured by,
/// `None` in flux mode.
pub fn reaction_category(reaction: &Reaction, mode: ReactionColorMode) -> Option<String> {
    match mode {
        ReactionColorMode::Flux => None,
        ReactionColorMode::Subsystem => Some(clean_or(&reaction.subsystem, "Sin subsystem")),
        ReactionColorMode::Pathway => Some(clean_or(&reaction.pathway, "Sin pathway")),
    }
}

fn clean_or(value: &str, fallback: &str) -> String {
    let t = js_trim(value);
    if t.is_empty() { fallback.to_string() } else { t.to_string() }
}

static MISSING_GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Sin (subsystem|pathway)$").unwrap());

/// Is this the explicit "no group" label (`Sin subsystem`, `Sin pathway`)?
pub fn is_missing_group(label: &str) -> bool {
    MISSING_GROUP.is_match(label)
}

/// `categoryColor(label, dark)`: a stable colour for a group name.
pub fn category_color(label: &str, dark: bool) -> &'static str {
    let text = js_trim(label);
    if text.is_empty() || is_missing_group(text) {
        return if dark { "#94a3b8" } else { "#64748b" };
    }
    let palette = if dark { &DARK } else { &LIGHT };
    palette[(stable_hash(text) % palette.len() as u32) as usize]
}

/// FNV-1a over UTF-16 code units, as `stableHash` computes it with `Math.imul`.
fn stable_hash(text: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for unit in text.encode_utf16() {
        hash ^= unit as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

/// One entry of the colour legend.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendEntry {
    pub label: String,
    pub count: usize,
    pub color: &'static str,
}

/// Per-reaction colours and groups, and the legend.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReactionColoring {
    pub mode: ReactionColorMode,
    pub colors: IndexMap<String, String>,
    pub groups: IndexMap<String, String>,
    pub legend: Vec<LegendEntry>,
}

/// `buildReactionColoring(model, mode, opts)`. `reaction_ids` limits the
/// legend and its counts, but colours are stable because they depend on the
/// category name, not order.
pub fn build_reaction_coloring(
    model: Option<&Model>,
    mode: ReactionColorMode,
    reaction_ids: Option<&HashSet<String>>,
    dark: bool,
) -> ReactionColoring {
    let mut out = ReactionColoring { mode, ..Default::default() };
    let Some(model) = model else { return out };
    if mode == ReactionColorMode::Flux {
        return out;
    }
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for (rid, reaction) in &model.reactions {
        let group = reaction_category(reaction, mode).expect("not flux mode");
        out.colors.insert(rid.clone(), category_color(&group, dark).to_string());
        if reaction_ids.is_none_or(|ids| ids.contains(rid)) {
            *counts.entry(group.clone()).or_insert(0) += 1;
        }
        out.groups.insert(rid.clone(), group);
    }
    let mut legend: Vec<LegendEntry> = counts
        .into_iter()
        .map(|(label, count)| LegendEntry { color: category_color(&label, dark), label, count })
        .collect();
    legend.sort_by(|a, b| {
        let a_missing = a.label.starts_with("Sin ");
        let b_missing = b.label.starts_with("Sin ");
        if a_missing != b_missing {
            return if a_missing { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
        }
        js::locale_compare(&a.label, &b.label)
    });
    out.legend = legend;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_normalise() {
        assert_eq!(normalize_pathway_ribbon_mode("pathway"), PathwayRibbonMode::Pathway);
        assert_eq!(normalize_pathway_ribbon_mode("subsystem"), PathwayRibbonMode::Subsystem);
        assert_eq!(normalize_pathway_ribbon_mode("anything-else"), PathwayRibbonMode::Off);
        assert_eq!(normalize_reaction_color_mode("x"), ReactionColorMode::Flux);
    }

    #[test]
    fn missing_groups_are_grey() {
        assert_eq!(category_color("Sin pathway", false), "#64748b");
        assert_eq!(category_color("sin SUBSYSTEM", true), "#94a3b8");
        assert_eq!(category_color("  ", false), "#64748b");
    }
}
