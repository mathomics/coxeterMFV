//! Port of `test/layout-settings.test.mjs`: the automatic-layout presets and
//! what they do to a layout.
//!
//! Not ported: "layout settings panel renders the same controls shown in the canvas toolbar"
//!   — it asserts on the HTML of `renderLayoutSettings` (`src/ui/panels.js`),
//!   the application's UI, not this crate.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{NodeKind, Rect};
use coxeter_map::layout::{Layout, LayoutOptions, layout_community};
use coxeter_map::layout_settings::{
    DEFAULT_LAYOUT_PRESET, LAYOUT_PRESETS, RawLayoutSettings, layout_engine_options, layout_preset,
    normalize_layout_settings,
};
use indexmap::IndexMap;

use common::load_model;

/// The JS `fourCellCommunity()`: four copies of the E. coli core model.
fn four_cell_community() -> CommunityModel {
    let files = [("a", "ecoli_core_mc"), ("b", "ecoli_core_mc"), ("c", "ecoli_core_mc"), ("d", "ecoli_core_mc")];
    let species = files
        .iter()
        .map(|(id, file)| SpeciesSpec {
            id: Some(id.to_string()),
            model: Arc::new(load_model(file)),
            abundance: Some(0.25),
            enabled: Some(true),
            color: None,
        })
        .collect();
    CommunityModel::new(CommunitySpec { species, medium: IndexMap::new(), ..Default::default() }).unwrap()
}

/// `LAYOUT_PRESETS.<name>`: the Rust table holds, per preset,
/// `[cellSpacing, minimumCompartmentSize, nodeSpacing, cellAreaMultiplier]`.
struct Preset {
    cell_spacing: f64,
    minimum_compartment_size: f64,
    node_spacing: f64,
    cell_area_multiplier: f64,
}

fn preset(name: &str) -> Preset {
    let (_, v) = LAYOUT_PRESETS.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no preset {name}"));
    Preset { cell_spacing: v[0], minimum_compartment_size: v[1], node_spacing: v[2], cell_area_multiplier: v[3] }
}

/// JS: `layout presets expose compact, normal and spacious with spacious as the default`
#[test]
fn layout_presets_expose_compact_normal_and_spacious_with_spacious_as_the_default() {
    assert_eq!(DEFAULT_LAYOUT_PRESET, "spacious");
    let keys: Vec<&str> = LAYOUT_PRESETS.iter().map(|(name, _)| *name).collect();
    assert_eq!(keys, ["compact", "normal", "spacious"]);
    assert!(preset("compact").cell_spacing < preset("spacious").cell_spacing);
    assert!(preset("compact").minimum_compartment_size < preset("spacious").minimum_compartment_size);
    assert!(preset("compact").node_spacing < preset("spacious").node_spacing);
    assert_eq!(preset("spacious").cell_area_multiplier, 6.0);
    // The table and `layoutPreset(name)` are the same values under the same names.
    for name in keys {
        let p = preset(name);
        let s = layout_preset(Some(name));
        assert_eq!(s.preset, name);
        assert_eq!(
            (s.cell_spacing, s.minimum_compartment_size, s.node_spacing, s.cell_area_multiplier),
            (p.cell_spacing, p.minimum_compartment_size, p.node_spacing, p.cell_area_multiplier)
        );
    }
}

/// JS: `custom layout values are clamped and marked custom`
#[test]
fn custom_layout_values_are_clamped_and_marked_custom() {
    let settings = normalize_layout_settings(&RawLayoutSettings {
        preset: Some("normal".into()),
        cell_spacing: Some(9999.0),
        minimum_compartment_size: Some(5.0),
        node_spacing: Some(1.31),
        cell_area_multiplier: Some(99.0),
    });
    assert_eq!(settings.preset, "custom");
    assert_eq!(settings.cell_spacing, 420.0);
    assert_eq!(settings.minimum_compartment_size, 28.0);
    assert_eq!(settings.node_spacing, 1.31);
    assert_eq!(settings.cell_area_multiplier, 8.0);
}

fn area(r: &Rect) -> f64 {
    r.w * r.h
}

/// `layout.cells.reduce((sum, cell) => sum + cell.rect.w * cell.rect.h, 0)`.
fn total_cell_area(layout: &Layout) -> f64 {
    layout.cells.iter().fold(0.0, |sum, cell| sum + area(&cell.rect))
}

/// JS: `sixfold cell-area multiplier produces roughly six times the automatic cell area`
#[test]
fn sixfold_cell_area_multiplier_produces_roughly_six_times_the_automatic_cell_area() {
    let community = four_cell_community();
    let common = LayoutOptions {
        cell_gap: Some(180.0),
        min_shell: Some(48.0),
        node_spacing: Some(1.08),
        ..Default::default()
    };
    let base = layout_community(&community, &LayoutOptions { cell_area_multiplier: Some(1.0), ..common.clone() });
    let expanded = layout_community(&community, &LayoutOptions { cell_area_multiplier: Some(6.0), ..common.clone() });

    let ratio = total_cell_area(&expanded) / total_cell_area(&base);

    // Shared-medium margins and inter-cell gaps mean the realised rectangle area
    // is not mathematically exact, but should remain close to the requested 6×.
    assert!(ratio > 5.0 && ratio < 7.0, "expected ~6× cell area, got {}×", js::to_fixed(ratio, 2));
}

/// The JS `nearestDistance(layout)`: the median distance from a metabolite of
/// cell `a` to its nearest neighbour in that cell.
fn nearest_distance(layout: &Layout) -> f64 {
    let values: Vec<_> =
        layout.nodes.values().filter(|node| node.kind == NodeKind::Metabolite && node.cell_id.as_deref() == Some("a")).collect();
    let mut nearest: Vec<f64> = values
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let mut best = f64::INFINITY;
            for (i, other) in values.iter().enumerate() {
                if i == index {
                    continue;
                }
                best = js::min(best, js::hypot(node.x - other.x, node.y - other.y));
            }
            best
        })
        .filter(|d| d.is_finite())
        .collect();
    nearest.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    nearest.get(nearest.len() / 2).copied().unwrap_or(0.0)
}

/// JS: `spacious automatic layout gives cells and nodes more map area than compact`
#[test]
fn spacious_automatic_layout_gives_cells_and_nodes_more_map_area_than_compact() {
    let community = four_cell_community();
    let compact =
        layout_community(&community, &layout_engine_options(&RawLayoutSettings::from(&layout_preset(Some("compact")))));
    let spacious =
        layout_community(&community, &layout_engine_options(&RawLayoutSettings::from(&layout_preset(Some("spacious")))));

    let compact_cell_area = total_cell_area(&compact);
    let spacious_cell_area = total_cell_area(&spacious);

    assert!(spacious.width * spacious.height > compact.width * compact.height);
    assert!(spacious_cell_area > compact_cell_area);

    assert!(nearest_distance(&spacious) > nearest_distance(&compact));
}
