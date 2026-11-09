//! Port of `src/ui/file-names.js`.
//!
//! Names for exported files: what they are of, then a short suffix for what
//! they are (`yeastGEM_v9.1.1_map.json`, `yeastGEM_v9.1.1_model.json`,
//! `consorcio-anaerobio_scenario.json`), so the files saved from one model sort
//! together and say at a glance which is the map and which the model.

use std::sync::LazyLock;

use chrono::{Datelike, Timelike};
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::jscompat::DOT;

/// The suffixes of `EXPORT_SUFFIX`.
pub mod export_suffix {
    /// Map JSON, SVG and PNG.
    pub const MAP: &str = "map";
    /// Model JSON and SBML.
    pub const MODEL: &str = "model";
    pub const SCENARIO: &str = "scenario";
    pub const FLUXES: &str = "fluxes";
    /// Reaction bounds (CSV).
    pub const BOUNDS: &str = "bounds";
    /// Reproducibility bundle.
    pub const REPRO: &str = "repro";
    /// Methods / provenance report.
    pub const METHODS: &str = "methods";
    pub const FINDINGS: &str = "findings";
}

fn is_stem_punct(c: char) -> bool {
    matches!(c, '.' | '_' | '+' | '-')
}

/// A model or community name made safe for a file name: accents dropped, any
/// run of characters other than letters, digits, `.`, `-`, `_` and `+` turned
/// into a single `_`, and kept to a readable length. Case is kept (`iML1515`),
/// and so is the `+` that joins several uploaded models.
pub fn file_stem(text: &str, fallback: &str) -> String {
    let decomposed: String = text.nfkd().filter(|c| !('\u{0300}'..='\u{036F}').contains(c)).collect();
    // Every run of other characters becomes one `_`. After this the stem is ASCII.
    let mut replaced = String::with_capacity(decomposed.len());
    let mut in_run = false;
    for c in decomposed.chars() {
        if c.is_ascii_alphanumeric() || is_stem_punct(c) {
            replaced.push(c);
            in_run = false;
        } else if !in_run {
            replaced.push('_');
            in_run = true;
        }
    }
    let mut collapsed = String::with_capacity(replaced.len());
    for c in replaced.chars() {
        if c == '_' && collapsed.ends_with('_') {
            continue;
        }
        collapsed.push(c);
    }
    let trimmed = collapsed.trim_start_matches(is_stem_punct).trim_end_matches(is_stem_punct);
    let cut = &trimmed[..trimmed.len().min(80)];
    let stem = cut.trim_end_matches(is_stem_punct);
    if stem.is_empty() { fallback.to_string() } else { stem.to_string() }
}

/// `fileStem(text)` with the default fallback, `coxeter`.
pub fn file_stem_default(text: &str) -> String {
    file_stem(text, "coxeter")
}

/// `<name>_<suffix>.<extension>`.
pub fn export_file_name(name: &str, suffix: &str, extension: &str) -> String {
    format!("{}_{}.{extension}", file_stem(name, "coxeter"), file_stem(suffix, "export"))
}

/// For exports kept as a record of a moment (the reproducibility bundle, the
/// methods report): `<name>_<suffix>_<yyyymmdd-hhmm>.<extension>`, local time,
/// so a bundle taken before a change and one taken after do not share a name.
pub fn timestamped_file_name<T: Datelike + Timelike>(name: &str, suffix: &str, extension: &str, date: &T) -> String {
    let stamp = format!(
        "{}{:02}{:02}-{:02}{:02}",
        date.year(),
        date.month(),
        date.day(),
        date.hour(),
        date.minute()
    );
    export_file_name(name, &format!("{suffix}_{stamp}"), extension)
}

/// [`timestamped_file_name`] at the current local time (`new Date()`).
pub fn timestamped_file_name_now(name: &str, suffix: &str, extension: &str) -> String {
    timestamped_file_name(name, suffix, extension, &chrono::Local::now())
}

/// A member model of a published community: `<community>_<member>_model.<ext>`.
/// Member ids are short codes (Bt, Ec) that recur across communities, so the
/// community stays in the name.
pub fn member_model_file_name(community_id: &str, member_id: &str, extension: &str) -> String {
    export_file_name(
        &format!("{}_{}", file_stem(community_id, "coxeter"), file_stem(member_id, "member")),
        export_suffix::MODEL,
        extension,
    )
}

static DATA_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^({DOT}*?)(?:\.([A-Za-z0-9]+))?$")).expect("pattern"));

/// A data file of a published community: `media.csv` → `<community>_media.csv`.
pub fn community_data_file_name(community_id: &str, file: &str) -> String {
    let caps = DATA_FILE.captures(file);
    let stem = caps.as_ref().and_then(|c| c.get(1)).map(|m| m.as_str()).filter(|s| !s.is_empty()).unwrap_or("data");
    let ext = caps.as_ref().and_then(|c| c.get(2)).map(|m| m.as_str()).filter(|s| !s.is_empty()).unwrap_or("csv");
    export_file_name(community_id, stem, ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    // From test/ui-audit-fixes.test.mjs.
    #[test]
    fn exported_files_are_named_after_the_model_with_a_short_suffix() {
        assert_eq!(export_file_name("yeastGEM_v9.1.1", export_suffix::MAP, "json"), "yeastGEM_v9.1.1_map.json");
        assert_eq!(export_file_name("yeastGEM_v9.1.1", export_suffix::MODEL, "xml"), "yeastGEM_v9.1.1_model.xml");
        assert_eq!(file_stem_default("Zuñiga (2020): S. elongatus + E. coli"), "Zuniga_2020_S._elongatus_+_E._coli");
        assert_eq!(file_stem_default("  ../  "), "coxeter", "nothing usable falls back to a name");
        assert!(file_stem_default(&"x".repeat(200)).len() <= 80);
    }

    #[test]
    fn member_community_data_and_dated_exports_keep_their_context_in_the_name() {
        assert_eq!(member_model_file_name("steadycom-gut-nine", "Ec", "json"), "steadycom-gut-nine_Ec_model.json");
        assert_ne!(
            member_model_file_name("steadycom-gut-nine", "Ec", "json"),
            member_model_file_name("zuniga-2020-phea-rescue", "Ec", "json")
        );
        assert_eq!(member_model_file_name("stolyar-2007-syntrophy", "Dv", "xml"), "stolyar-2007-syntrophy_Dv_model.xml");
        assert_eq!(community_data_file_name("steadycom-gut-nine", "media.csv"), "steadycom-gut-nine_media.csv");
        assert_eq!(community_data_file_name("x", "bounds"), "x_bounds.csv", "a file without extension stays a CSV");
        let when = chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap().and_hms_opt(15, 7, 0).unwrap();
        assert_eq!(timestamped_file_name("consorcio-anaerobio", "repro", "json", &when), "consorcio-anaerobio_repro_20261010-1507.json");
        assert_eq!(timestamped_file_name("yeastGEM_v9.1.1", "methods", "md", &when), "yeastGEM_v9.1.1_methods_20261010-1507.md");
        assert_eq!(export_file_name("iML1515+yeastGEM", "map", "json"), "iML1515+yeastGEM_map.json", "the + joining uploads survives");
    }
}
