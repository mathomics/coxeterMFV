//! User-facing automatic-layout presets (port of
//! `src/layout/layout-settings.js`).
//!
//! These values are deliberately expressed in map pixels / dimensionless
//! multipliers so they can be shown directly in the UI and passed unchanged
//! to the layout engine.

use coxeter_core::js;

use crate::layout::LayoutOptions;

/// One set of layout settings, as the UI shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutSettings {
    /// `compact`, `normal`, `spacious` or `custom`.
    pub preset: String,
    pub cell_spacing: f64,
    pub minimum_compartment_size: f64,
    pub node_spacing: f64,
    pub cell_area_multiplier: f64,
}

/// What a caller may hand to [`normalize_layout_settings`]: any field may be
/// missing or not a number, as in the JavaScript.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawLayoutSettings {
    pub preset: Option<String>,
    pub cell_spacing: Option<f64>,
    pub minimum_compartment_size: Option<f64>,
    pub node_spacing: Option<f64>,
    pub cell_area_multiplier: Option<f64>,
}

impl From<&LayoutSettings> for RawLayoutSettings {
    fn from(s: &LayoutSettings) -> Self {
        RawLayoutSettings {
            preset: Some(s.preset.clone()),
            cell_spacing: Some(s.cell_spacing),
            minimum_compartment_size: Some(s.minimum_compartment_size),
            node_spacing: Some(s.node_spacing),
            cell_area_multiplier: Some(s.cell_area_multiplier),
        }
    }
}

const fn preset(name: &'static str, cell_spacing: f64, min_size: f64, node_spacing: f64, area: f64) -> (&'static str, [f64; 4]) {
    (name, [cell_spacing, min_size, node_spacing, area])
}

/// `LAYOUT_PRESETS`, in their declared order.
pub const LAYOUT_PRESETS: [(&str, [f64; 4]); 3] = [
    preset("compact", 120.0, 38.0, 0.82, 1.0),
    preset("normal", 180.0, 48.0, 1.08, 3.0),
    preset("spacious", 240.0, 60.0, 1.42, 6.0),
];

/// `DEFAULT_LAYOUT_PRESET`: the preset used when none is named.
pub const DEFAULT_LAYOUT_PRESET: &str = "spacious";

fn settings_of(name: &str, v: [f64; 4]) -> LayoutSettings {
    LayoutSettings {
        preset: name.to_string(),
        cell_spacing: v[0],
        minimum_compartment_size: v[1],
        node_spacing: v[2],
        cell_area_multiplier: v[3],
    }
}

/// `layoutPreset(name)`: a copy of the named preset, or the default one.
pub fn layout_preset(name: Option<&str>) -> LayoutSettings {
    let name = name.unwrap_or(DEFAULT_LAYOUT_PRESET);
    let (n, v) = LAYOUT_PRESETS
        .iter()
        .find(|(n, _)| *n == name)
        .or_else(|| LAYOUT_PRESETS.iter().find(|(n, _)| *n == DEFAULT_LAYOUT_PRESET))
        .expect("the default preset exists");
    settings_of(n, *v)
}

fn clamp(value: Option<f64>, min: f64, max: f64, fallback: f64) -> f64 {
    match value {
        Some(n) if n.is_finite() => js::min(max, js::max(min, n)),
        _ => fallback,
    }
}

/// `normalizeLayoutSettings(raw)`: clamp every value into its range, and name
/// the preset the values match (`custom` when none does).
pub fn normalize_layout_settings(raw: &RawLayoutSettings) -> LayoutSettings {
    let base = layout_preset(raw.preset.as_deref());
    let mut next = LayoutSettings {
        preset: base.preset.clone(),
        cell_spacing: clamp(raw.cell_spacing, 80.0, 420.0, base.cell_spacing),
        minimum_compartment_size: clamp(raw.minimum_compartment_size, 28.0, 110.0, base.minimum_compartment_size),
        node_spacing: clamp(raw.node_spacing, 0.65, 1.9, base.node_spacing),
        cell_area_multiplier: clamp(raw.cell_area_multiplier, 1.0, 8.0, base.cell_area_multiplier),
    };
    let matched = LAYOUT_PRESETS.iter().find(|(_, p)| {
        (next.cell_spacing - p[0]).abs() < 1e-9
            && (next.minimum_compartment_size - p[1]).abs() < 1e-9
            && (next.node_spacing - p[2]).abs() < 1e-9
            && (next.cell_area_multiplier - p[3]).abs() < 1e-9
    });
    next.preset = matched.map(|(n, _)| n.to_string()).unwrap_or_else(|| "custom".into());
    next
}

/// `layoutEngineOptions(settings)`: the options `layout_community` takes.
pub fn layout_engine_options(settings: &RawLayoutSettings) -> LayoutOptions {
    let n = normalize_layout_settings(settings);
    LayoutOptions {
        cell_gap: Some(n.cell_spacing),
        min_shell: Some(n.minimum_compartment_size),
        node_spacing: Some(n.node_spacing),
        cell_area_multiplier: Some(n.cell_area_multiplier),
        ..LayoutOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_default() {
        assert_eq!(DEFAULT_LAYOUT_PRESET, "spacious");
        let names: Vec<&str> = LAYOUT_PRESETS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["compact", "normal", "spacious"]);
        let compact = layout_preset(Some("compact"));
        let spacious = layout_preset(Some("spacious"));
        assert!(compact.cell_spacing < spacious.cell_spacing);
        assert!(compact.minimum_compartment_size < spacious.minimum_compartment_size);
        assert!(compact.node_spacing < spacious.node_spacing);
        assert_eq!(spacious.cell_area_multiplier, 6.0);
        assert_eq!(layout_preset(Some("nope")).preset, "spacious");
    }

    #[test]
    fn custom_values_are_clamped_and_marked_custom() {
        let s = normalize_layout_settings(&RawLayoutSettings {
            preset: Some("normal".into()),
            cell_spacing: Some(9999.0),
            minimum_compartment_size: Some(5.0),
            node_spacing: Some(1.31),
            cell_area_multiplier: Some(99.0),
        });
        assert_eq!(s.preset, "custom");
        assert_eq!(s.cell_spacing, 420.0);
        assert_eq!(s.minimum_compartment_size, 28.0);
        assert_eq!(s.node_spacing, 1.31);
        assert_eq!(s.cell_area_multiplier, 8.0);
    }

    #[test]
    fn a_preset_is_recognised_by_its_values() {
        let s = normalize_layout_settings(&RawLayoutSettings::from(&layout_preset(Some("normal"))));
        assert_eq!(s.preset, "normal");
        let s = normalize_layout_settings(&RawLayoutSettings { node_spacing: Some(f64::NAN), ..Default::default() });
        assert_eq!(s, layout_preset(None));
    }
}
