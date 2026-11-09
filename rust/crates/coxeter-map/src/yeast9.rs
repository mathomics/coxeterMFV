//! First-class Yeast9 / yeast-GEM integration (port of `src/core/yeast9.js`,
//! with the preview placement of `src/ui/store.js`).
//!
//! The source is pinned so the scenario and its representative map remain
//! reproducible. The preview intentionally contains a curated subset of the
//! genome-scale network: enough biology to make every compartment
//! informative without turning the canvas into a 4,000-reaction hairball.

use coxeter_core::community::{CommunityModel, namespaced};
use coxeter_core::js;
use coxeter_core::mapdata::{Point, Rect};
use coxeter_core::model::Model;
use indexmap::{IndexMap, IndexSet};

use crate::editable_map::EditableMap;
use crate::place_reaction::PlaceOptions;

pub const YEAST9_SCENARIO_ID: &str = "yeast9-9-1-1";
pub const YEAST9_MODEL_KEY: &str = "yeast9-9-1-1";

/// Where the pinned model comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Yeast9Source {
    pub version: &'static str,
    pub commit: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
}

pub const YEAST9_SOURCE: Yeast9Source = Yeast9Source {
    version: "9.1.1",
    commit: "2d594ae1c4a2d550ccef120d96a58c7bbf586255",
    filename: "yeast-GEM-9.1.1.xml",
    url: "https://raw.githubusercontent.com/SysBioChalmers/yeast-GEM/2d594ae1c4a2d550ccef120d96a58c7bbf586255/model/yeast-GEM.xml",
};

/// One group of representative reactions drawn in one compartment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewGroup {
    pub compartment: &'static str,
    pub label: &'static str,
    pub reactions: &'static [&'static str],
}

pub const YEAST9_PREVIEW_GROUPS: [PreviewGroup; 15] = [
    PreviewGroup { compartment: "e", label: "Extracellular hydrolysis", reactions: &["r_1024", "r_0370"] },
    PreviewGroup { compartment: "ce", label: "Cell wall synthesis", reactions: &["r_0005", "r_0006"] },
    PreviewGroup { compartment: "ce", label: "Plasma-membrane nutrient transport", reactions: &["r_1166", "r_1979", "r_1244"] },
    PreviewGroup {
        compartment: "c",
        label: "Cytosolic glycolysis and fermentation",
        reactions: &["r_0534", "r_0467", "r_0886", "r_0486", "r_0962", "r_0959", "r_2115"],
    },
    PreviewGroup { compartment: "er", label: "ER sphingolipid and sterol metabolism", reactions: &["r_0041", "r_0244", "r_0263"] },
    PreviewGroup { compartment: "erm", label: "ER-membrane steryl ester metabolism", reactions: &["r_0127", "r_0132"] },
    PreviewGroup { compartment: "g", label: "Golgi mannosylation", reactions: &["r_0192", "r_0747"] },
    PreviewGroup { compartment: "gm", label: "Golgi-membrane phospholipid metabolism", reactions: &["r_2360", "r_2363"] },
    PreviewGroup { compartment: "lp", label: "Lipid-particle storage metabolism", reactions: &["r_2400", "r_0281"] },
    PreviewGroup { compartment: "m", label: "Mitochondrial TCA and respiration", reactions: &["r_0300", "r_4262", "r_0713", "r_1021"] },
    PreviewGroup {
        compartment: "mm",
        label: "Mitochondrial membrane and oxidative phosphorylation",
        reactions: &["r_0226", "r_0438", "r_2464"],
    },
    PreviewGroup { compartment: "n", label: "Nuclear inositol and NAD metabolism", reactions: &["r_0571", "r_0769"] },
    PreviewGroup { compartment: "p", label: "Peroxisomal beta-oxidation", reactions: &["r_0120", "r_0057", "r_0100"] },
    PreviewGroup { compartment: "v", label: "Vacuolar carbohydrate turnover", reactions: &["r_0193", "r_0464"] },
    PreviewGroup { compartment: "vm", label: "Vacuolar membrane transport", reactions: &["r_1086", "r_4237"] },
];

pub const YEAST9_PREVIEW_COMPARTMENTS: [&str; 14] =
    ["e", "ce", "c", "er", "erm", "g", "gm", "lp", "m", "mm", "n", "p", "v", "vm"];

/// One reaction of the preview, with its place in its group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewSpec {
    pub reaction_id: String,
    pub compartment: &'static str,
    pub group: &'static str,
    pub index: usize,
    pub count: usize,
}

/// `yeast9PreviewSpecs(model)`: the preview reactions this model has.
pub fn yeast9_preview_specs(model: &Model) -> Vec<PreviewSpec> {
    let mut out = Vec::new();
    for group in &YEAST9_PREVIEW_GROUPS {
        let present: Vec<&str> = group.reactions.iter().copied().filter(|id| model.reactions.contains_key(*id)).collect();
        for (index, reaction_id) in present.iter().enumerate() {
            out.push(PreviewSpec {
                reaction_id: reaction_id.to_string(),
                compartment: group.compartment,
                group: group.label,
                index,
                count: present.len(),
            });
        }
    }
    out
}

/// What `populateYeast9Preview` drew.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Yeast9Preview {
    pub placed: usize,
    pub covered_compartments: usize,
    pub missing: Vec<String>,
}

/// The canvas the store gives the Yeast9 scenario: the generic automatic
/// canvas is sized for all 4,105 reactions, while the curated scenario draws
/// only a representative subset, so a compact publication-like canvas keeps
/// the 14 compartments legible.
pub const YEAST9_CANVAS: (f64, f64) = (2100.0, 1550.0);

/// `populateYeast9Preview(model, speciesId)` of the store: draw a curated,
/// readable Yeast9 map rather than a genome-scale hairball. Reactions are
/// spread through the free area of their biological compartment, avoiding
/// child organelles whenever possible.
pub fn populate_yeast9_preview(map: &mut EditableMap, community: &CommunityModel, model: &Model, species_id: &str) -> Yeast9Preview {
    let specs = yeast9_preview_specs(model);
    let mut by_compartment: IndexMap<&str, Vec<&PreviewSpec>> = IndexMap::new();
    for spec in &specs {
        by_compartment.entry(spec.compartment).or_default().push(spec);
    }
    let mut out = Yeast9Preview::default();
    let mut covered: IndexSet<&str> = IndexSet::new();
    for (compartment_id, group) in by_compartment {
        let anchors = yeast_preview_anchors(map, species_id, compartment_id, group.len());
        for (index, spec) in group.iter().enumerate() {
            let merged = namespaced(species_id, &spec.reaction_id);
            let opts = PlaceOptions { anchor: anchors.get(index).copied(), ..Default::default() };
            if map.add_reaction(community, &merged, &opts).is_some() {
                out.placed += 1;
                covered.insert(compartment_id);
            } else {
                out.missing.push(spec.reaction_id.clone());
            }
        }
    }
    out.covered_compartments = covered.len();
    out
}

fn point_inside_rect(p: Point, r: &Rect, padding: f64) -> bool {
    p.x > r.x - padding && p.x < r.x + r.w + padding && p.y > r.y - padding && p.y < r.y + r.h + padding
}

/// `yeastPreviewAnchors(map, speciesId, compartmentId, count)` of the store:
/// `count` points spread over a compartment, clear of the compartments
/// nested in it when there is room.
pub fn yeast_preview_anchors(map: &EditableMap, species_id: &str, compartment_id: &str, count: usize) -> Vec<Point> {
    let Some(cell) = map.cell(species_id) else { return vec![] };
    let Some(compartment) = cell.compartments.iter().find(|c| c.id == compartment_id) else { return vec![] };
    if count == 0 {
        return vec![];
    }
    let rect = compartment.rect;
    let children: Vec<Rect> = cell.compartments.iter().filter(|c| c.parent_id.as_deref() == Some(compartment_id)).map(|c| c.rect).collect();

    let mut candidates = Vec::new();
    let (cols, rows) = (8.0, 6.0);
    for row in 0..6 {
        for col in 0..8 {
            let p = Point::new(rect.x + rect.w * ((col as f64 + 1.0) / (cols + 1.0)), rect.y + rect.h * ((row as f64 + 1.0) / (rows + 1.0)));
            let child_padding = js::min(rect.w, rect.h) * 0.018;
            if !children.iter().any(|child| point_inside_rect(p, child, child_padding)) {
                candidates.push(p);
            }
        }
    }
    let n = count as f64;
    let pool: Vec<Point> = if candidates.len() >= count {
        candidates
    } else {
        (0..count)
            .map(|i| {
                let i = i as f64;
                Point::new(rect.x + rect.w * (((i % 4.0) + 1.0) / 5.0), rect.y + rect.h * (((i / 4.0).floor() + 1.0) / ((n / 4.0).ceil() + 1.0)))
            })
            .collect()
    };
    if count == 1 {
        return vec![pool[pool.len() / 2]];
    }
    (0..count)
        .map(|i| {
            let slot = js::round(i as f64 * (pool.len() as f64 - 1.0) / (n - 1.0)) as usize;
            pool[slot]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_curated_preview_covers_all_14_model_compartments() {
        let mut covered: Vec<&str> = YEAST9_PREVIEW_GROUPS.iter().map(|g| g.compartment).collect::<HashSet<_>>().into_iter().collect();
        covered.sort();
        let mut all = YEAST9_PREVIEW_COMPARTMENTS.to_vec();
        all.sort();
        assert_eq!(covered, all);
        assert_eq!(YEAST9_PREVIEW_COMPARTMENTS.len(), 14);
        let ids: Vec<&str> = YEAST9_PREVIEW_GROUPS.iter().flat_map(|g| g.reactions.iter().copied()).collect();
        assert!(ids.len() >= 30);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len(), "representative reaction ids should not be duplicated");
    }
}
