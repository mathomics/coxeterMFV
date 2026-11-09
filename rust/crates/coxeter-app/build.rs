//! The bundled files (`models/**` of the repository), compiled into the
//! binary when it is meant to stand on its own: in a release build, or with
//! the `embed-models` feature. A development build reads them from the
//! repository instead, which keeps the test binaries small and lets a changed
//! model be picked up without a rebuild.
//!
//! Writes `$OUT_DIR/embedded_models.rs`: `pub static FILES: &[(&str, &[u8])]`,
//! one `include_bytes!` per file, keyed by its path relative to the
//! repository root (`models/index.json`), which is what the Store asks for.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let repo = manifest.join("../../..");
    let models = repo.join("models");
    let release = std::env::var("PROFILE").is_ok_and(|p| p == "release");
    let embed = release || std::env::var_os("CARGO_FEATURE_EMBED_MODELS").is_some();
    println!("cargo:rerun-if-env-changed=PROFILE");

    let mut code = String::from("/// Every bundled file, by its path from the repository root.\npub static FILES: &[(&str, &[u8])] = &[\n");
    if embed && models.is_dir() {
        println!("cargo:rerun-if-changed={}", models.display());
        let mut files = Vec::new();
        walk(&models, &mut files);
        for file in files {
            let Ok(rel) = file.strip_prefix(&repo) else { continue };
            let key = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            let abs = std::fs::canonicalize(&file).unwrap_or(file.clone());
            // `include_bytes!` wants a plain path; drop Windows' `\\?\` prefix.
            let abs = abs.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
            code.push_str(&format!("    ({key:?}, include_bytes!({abs:?})),\n"));
        }
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("embedded_models.rs");
    std::fs::write(out, code).expect("write embedded_models.rs");
}
