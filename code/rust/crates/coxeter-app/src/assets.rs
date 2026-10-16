//! Where the bundled files (`models/**`) come from, and where downloads are
//! kept.
//!
//! In order: a folder named on the command line or in `COXETER_ROOT`; the
//! files compiled into the binary (a release build, see `build.rs`); a
//! `models/` folder beside the executable or above it; the repository this
//! crate was built from (development).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use coxeter_state::{Assets, DirAssets, EmbeddedAssets};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_models.rs"));
}

/// The files compiled into this binary (empty in a development build).
pub fn embedded_files() -> &'static [(&'static str, &'static [u8])] {
    embedded::FILES
}

/// Whether `dir` holds the bundled files (`models/index.json`).
pub fn is_root(dir: &Path) -> bool {
    dir.join("models").join("index.json").is_file()
}

/// The repository root this crate was built in.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
}

/// The assets to run with, and a description of where they are.
pub fn choose(explicit: Option<&Path>) -> Result<(Arc<dyn Assets>, String), String> {
    if let Some(dir) = explicit.map(Path::to_path_buf).or_else(|| std::env::var_os("COXETER_ROOT").map(PathBuf::from)) {
        if is_root(&dir) {
            return Ok((Arc::new(DirAssets::new(&dir)), dir.display().to_string()));
        }
        return Err(format!("{} has no models/index.json", dir.display()));
    }
    if !embedded_files().is_empty() {
        return Ok((Arc::new(EmbeddedAssets::new(embedded_files())), "embedded".into()));
    }
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(6) {
            if is_root(dir) {
                return Ok((Arc::new(DirAssets::new(dir)), dir.display().to_string()));
            }
        }
    }
    let repo = repo_root();
    if is_root(&repo) {
        return Ok((Arc::new(DirAssets::new(&repo)), repo.display().to_string()));
    }
    Err("The bundled models were not found: pass --models <folder holding models/>.".into())
}

/// The folder downloads are kept in between runs (Yeast9, the Zuñiga sources).
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") { home.join("Library").join("Caches") } else { home.join(".cache") }
            })
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("coxeter-mfv")
}

/// The folder the interface keeps its settings in (the theme).
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") { home.join("Library").join("Application Support") } else { home.join(".config") }
            })
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("coxeter-mfv")
}
