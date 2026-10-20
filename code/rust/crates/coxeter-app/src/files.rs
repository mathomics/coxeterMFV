//! Files in and out: the hidden `<input type="file">` elements of
//! `index.html` and the `download(name, text, type)` of `src/ui/app.js`,
//! as native dialogs.
//!
//! [`FileDialogs`] is what the interface asks; [`NativeDialogs`] answers with
//! the system's dialogs (`rfd`), [`ScriptedDialogs`] with answers prepared by
//! a test (and is what `--screenshot` runs with, so it never blocks).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// A file type a dialog offers: a name and its extensions (no dots).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

/// The `accept` attributes of the file inputs of `index.html`, and the
/// types of what the app writes.
pub mod filter {
    use super::Filter;
    pub const JSON: Filter = Filter { name: "JSON", extensions: &["json"] };
    pub const MODEL: Filter = Filter { name: "COBRA JSON, SBML, MATLAB or Excel", extensions: &["json", "xml", "sbml", "mat", "xlsx"] };
    pub const SBML: Filter = Filter { name: "SBML", extensions: &["xml", "sbml"] };
    pub const CSV: Filter = Filter { name: "CSV", extensions: &["csv"] };
    pub const TABLE: Filter = Filter { name: "CSV or TSV", extensions: &["csv", "tsv", "txt"] };
    pub const SVG: Filter = Filter { name: "SVG image", extensions: &["svg"] };
    pub const PNG: Filter = Filter { name: "PNG image", extensions: &["png"] };
    pub const MARKDOWN: Filter = Filter { name: "Markdown", extensions: &["md"] };
}

/// The dialogs the interface opens.
pub trait FileDialogs {
    /// Pick files to read (`input.click()`); empty when cancelled.
    fn open(&mut self, title: &str, filters: &[Filter], multiple: bool) -> Vec<PathBuf>;
    /// Pick where to write `suggested` (`download(name, …)`); `None` when cancelled.
    fn save(&mut self, title: &str, suggested: &str, filters: &[Filter]) -> Option<PathBuf>;
    /// `window.confirm(message)`.
    fn confirm(&mut self, title: &str, message: &str) -> bool;
}

/// The system's dialogs.
#[derive(Debug, Default)]
pub struct NativeDialogs {
    /// The folder the last file was opened from or written to.
    pub last_dir: Option<PathBuf>,
}

impl NativeDialogs {
    fn dialog(&self, title: &str, filters: &[Filter]) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new().set_title(title);
        for f in filters {
            d = d.add_filter(f.name, f.extensions);
        }
        if let Some(dir) = &self.last_dir {
            d = d.set_directory(dir);
        }
        d
    }
    fn remember(&mut self, path: &Path) {
        self.last_dir = path.parent().map(Path::to_path_buf);
    }
}

impl FileDialogs for NativeDialogs {
    fn open(&mut self, title: &str, filters: &[Filter], multiple: bool) -> Vec<PathBuf> {
        let d = self.dialog(title, filters);
        let picked = if multiple { d.pick_files().unwrap_or_default() } else { d.pick_file().into_iter().collect() };
        if let Some(first) = picked.first() {
            self.remember(first);
        }
        picked
    }
    fn save(&mut self, title: &str, suggested: &str, filters: &[Filter]) -> Option<PathBuf> {
        let path = self.dialog(title, filters).set_file_name(suggested).save_file()?;
        self.remember(&path);
        Some(path)
    }
    fn confirm(&mut self, title: &str, message: &str) -> bool {
        rfd::MessageDialog::new()
            .set_title(title)
            .set_description(message)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show()
            == rfd::MessageDialogResult::Ok
    }
}

/// Dialogs answered from a script: what a test (or a screenshot) prepared.
#[derive(Debug, Default)]
pub struct ScriptedDialogs {
    /// The answers to the next `open` calls, in order (none: cancelled).
    pub opens: VecDeque<Vec<PathBuf>>,
    /// Where saves land, under the suggested name; `None`: every save is cancelled.
    pub save_dir: Option<PathBuf>,
    /// The answer to every `confirm`.
    pub confirm: bool,
    /// Every dialog asked for, as `open: <title>`, `save: <suggested>`, `confirm: <message>`.
    pub asked: Vec<String>,
    /// Every path a save returned.
    pub saved: Vec<PathBuf>,
}

impl ScriptedDialogs {
    pub fn new() -> ScriptedDialogs {
        ScriptedDialogs { confirm: true, ..Default::default() }
    }
    /// Saves go to `dir`.
    pub fn saving_to(dir: impl Into<PathBuf>) -> ScriptedDialogs {
        ScriptedDialogs { save_dir: Some(dir.into()), confirm: true, ..Default::default() }
    }
}

impl FileDialogs for ScriptedDialogs {
    fn open(&mut self, title: &str, _filters: &[Filter], _multiple: bool) -> Vec<PathBuf> {
        self.asked.push(format!("open: {title}"));
        self.opens.pop_front().unwrap_or_default()
    }
    fn save(&mut self, _title: &str, suggested: &str, _filters: &[Filter]) -> Option<PathBuf> {
        self.asked.push(format!("save: {suggested}"));
        let path = self.save_dir.as_ref()?.join(suggested);
        self.saved.push(path.clone());
        Some(path)
    }
    fn confirm(&mut self, _title: &str, message: &str) -> bool {
        self.asked.push(format!("confirm: {message}"));
        self.confirm
    }
}

/// A shared handle on scripted dialogs, so a test keeps reading what the app asked.
#[derive(Debug, Clone, Default)]
pub struct SharedDialogs(pub std::sync::Arc<std::sync::Mutex<ScriptedDialogs>>);

impl SharedDialogs {
    pub fn new(inner: ScriptedDialogs) -> SharedDialogs {
        SharedDialogs(std::sync::Arc::new(std::sync::Mutex::new(inner)))
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut ScriptedDialogs) -> R) -> R {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }
}

impl FileDialogs for SharedDialogs {
    fn open(&mut self, title: &str, filters: &[Filter], multiple: bool) -> Vec<PathBuf> {
        self.with(|d| d.open(title, filters, multiple))
    }
    fn save(&mut self, title: &str, suggested: &str, filters: &[Filter]) -> Option<PathBuf> {
        self.with(|d| d.save(title, suggested, filters))
    }
    fn confirm(&mut self, title: &str, message: &str) -> bool {
        self.with(|d| d.confirm(title, message))
    }
}

/// The name a path shows in messages (`file.name`).
pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// `file.text()`: a file read as text (UTF-8, lossy like the browser's decoder).
pub fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("{}: {e}", display_name(path)))
}

/// `file.arrayBuffer()`: a file read as bytes.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", display_name(path)))
}

/// What kind of file a path given on the command line is, from its name and
/// first bytes: what the app should open it as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// COBRA JSON, SBML, or a COBRA Toolbox MATLAB or Excel file.
    Model,
    /// A map document (Map JSON).
    Map,
    /// A saved scenario (Scenario).
    Scenario,
    /// A reproducibility bundle.
    Bundle,
    /// Long-format metatranscriptomics (species, gene, condition, value).
    Metatranscriptomics,
    /// A wide omics table (`id,Condition_A,…`).
    Omics,
}

/// Guess what a file is (`--screenshot a.json b.xml`, files dropped on the window).
pub fn classify(path: &Path, text: &str) -> Option<FileKind> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "xml" | "sbml" | "mat" | "xlsx" | "xlsm" => return Some(FileKind::Model),
        "csv" | "tsv" | "txt" => {
            let header = text.lines().next().unwrap_or("").to_ascii_lowercase();
            let cols: Vec<&str> = header.split([',', '\t', ';']).map(str::trim).collect();
            let has = |names: &[&str]| cols.iter().any(|c| names.contains(&c.trim_matches('"')));
            let long = has(&["species", "mag", "organism", "taxon"]) && has(&["gene", "gene_id", "locus_tag"]) && has(&["condition", "sample"]);
            return Some(if long { FileKind::Metatranscriptomics } else { FileKind::Omics });
        }
        _ => {}
    }
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    if obj.contains_key("integrity") {
        return Some(FileKind::Bundle);
    }
    if obj.get("format").and_then(|f| f.as_str()) == Some(coxeter_core::mapdata::MAP_DOCUMENT_FORMAT)
        || (obj.contains_key("nodes") && obj.contains_key("segments"))
    {
        return Some(FileKind::Map);
    }
    if obj.contains_key("reactions") && obj.contains_key("metabolites") {
        return Some(FileKind::Model);
    }
    if obj.contains_key("species") || obj.contains_key("scenario") || obj.contains_key("medium") {
        return Some(FileKind::Scenario);
    }
    None
}
