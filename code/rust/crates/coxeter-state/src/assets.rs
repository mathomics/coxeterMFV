//! Files instead of `fetch`.
//!
//! The JavaScript store reads everything it ships with (`models/index.json`,
//! `models/scenarios.json`, the bundled models and the published communities
//! under `models/published/`) with `fetch(`${base}/models/…`)`, and the
//! single-file build answers the same requests from an embedded table
//! (`globalThis.__ESCHER_MCMC_DATA__`). Here an [`Assets`] answers them by
//! path relative to the application root (`models/index.json`):
//!
//! - [`DirAssets`] reads a directory (the repository root in development),
//! - [`EmbeddedAssets`] serves files compiled into the binary, and behaves
//!   like the single-file build (only what is embedded exists),
//! - [`MemoryAssets`] holds files in memory, for tests and for files a GUI has
//!   already read.
//!
//! Downloads from the internet (the official Yeast9 SBML, the Zuñiga sources)
//! go through a [`Fetcher`], which the application injects; this crate has no
//! HTTP client of its own (the `download` feature lends it the one of
//! `coxeter-analysis`). [`CachedFetcher`] keeps what was downloaded on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::Value;

use crate::error::{Result, StoreError};

/// The files the application ships with, by path relative to its root.
pub trait Assets: Send + Sync {
    /// The bytes of `path` (`models/ecoli_core_mc.json`), or the error the JS
    /// `fetchJSON` throws: `Could not load <path> (<status>)`.
    fn read(&self, path: &str) -> Result<Vec<u8>>;

    /// Whether `path` exists. The single-file build tests membership of its
    /// embedded table (`key in embedded`) to choose between full models and
    /// map previews.
    fn contains(&self, path: &str) -> bool;

    /// The single-file build: only embedded files exist, and genome-scale
    /// members may be missing (`globalThis.__ESCHER_MCMC_DATA__` is set).
    fn is_embedded(&self) -> bool {
        false
    }

    /// `fetchText`.
    fn read_text(&self, path: &str) -> Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|_| StoreError::msg(format!("Could not load {path} (not UTF-8 text)")))
    }

    /// `fetchJSON`.
    fn read_json(&self, path: &str) -> Result<Value> {
        let text = self.read_text(path)?;
        serde_json::from_str(&text).map_err(|e| StoreError::msg(format!("Could not load {path} ({e})")))
    }
}

/// Assets read from a directory: the repository root in development, or the
/// folder an installed application keeps its `models/` in.
#[derive(Debug, Clone)]
pub struct DirAssets {
    root: PathBuf,
}

impl DirAssets {
    pub fn new(root: impl Into<PathBuf>) -> DirAssets {
        DirAssets { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, path: &str) -> PathBuf {
        let mut out = self.root.clone();
        for part in path.split('/').filter(|p| !p.is_empty() && *p != ".") {
            out.push(part);
        }
        out
    }
}

impl Assets for DirAssets {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        if path.split('/').any(|p| p == "..") {
            return Err(StoreError::msg(format!("Could not load {path} (403)")));
        }
        std::fs::read(self.path(path)).map_err(|e| {
            let status = if e.kind() == std::io::ErrorKind::NotFound { "404".to_string() } else { e.to_string() };
            StoreError::msg(format!("Could not load {path} ({status})"))
        })
    }

    fn contains(&self, path: &str) -> bool {
        !path.split('/').any(|p| p == "..") && self.path(path).is_file()
    }
}

/// Files compiled into the binary (`include_bytes!`), as the single-file HTML
/// embeds them.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedAssets {
    files: &'static [(&'static str, &'static [u8])],
}

impl EmbeddedAssets {
    pub const fn new(files: &'static [(&'static str, &'static [u8])]) -> EmbeddedAssets {
        EmbeddedAssets { files }
    }
}

impl Assets for EmbeddedAssets {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| bytes.to_vec())
            .ok_or_else(|| StoreError::msg(format!("Could not load {path} (404)")))
    }

    fn contains(&self, path: &str) -> bool {
        self.files.iter().any(|(p, _)| *p == path)
    }

    fn is_embedded(&self) -> bool {
        true
    }
}

/// Files held in memory: what tests stub `fetch` with, and files a GUI has
/// already read. With `embedded` set it stands for the single-file build.
#[derive(Debug, Default)]
pub struct MemoryAssets {
    files: RwLock<HashMap<String, Vec<u8>>>,
    embedded: bool,
    /// Another source for whatever is not held here.
    fallback: Option<DirAssets>,
}

impl MemoryAssets {
    pub fn new() -> MemoryAssets {
        MemoryAssets::default()
    }

    /// In-memory files standing for the single-file build's embedded table.
    pub fn embedded() -> MemoryAssets {
        MemoryAssets { embedded: true, ..MemoryAssets::default() }
    }

    /// Files held here first, then the directory.
    pub fn over(dir: DirAssets) -> MemoryAssets {
        MemoryAssets { fallback: Some(dir), ..MemoryAssets::default() }
    }

    pub fn insert(&self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        if let Ok(mut files) = self.files.write() {
            files.insert(path.into(), bytes.into());
        }
    }

    pub fn insert_json(&self, path: impl Into<String>, value: &Value) {
        self.insert(path, value.to_string().into_bytes());
    }
}

impl Assets for MemoryAssets {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        if let Some(bytes) = self.files.read().ok().and_then(|f| f.get(path).cloned()) {
            return Ok(bytes);
        }
        match &self.fallback {
            Some(dir) => dir.read(path),
            None => Err(StoreError::msg(format!("Could not load {path} (404)"))),
        }
    }

    fn contains(&self, path: &str) -> bool {
        self.files.read().is_ok_and(|f| f.contains_key(path)) || self.fallback.as_ref().is_some_and(|d| d.contains(path))
    }

    fn is_embedded(&self) -> bool {
        self.embedded
    }
}

// Downloads

/// What a download returned: `response.status` and its body.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// `response.ok`.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Downloads from the internet. The application provides one; tests and
/// offline builds use [`NoFetcher`].
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<FetchResponse>;
}

/// No network: every download fails with a message that says so.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFetcher;

impl Fetcher for NoFetcher {
    fn fetch(&self, url: &str) -> Result<FetchResponse> {
        Err(StoreError::msg(format!("Downloading {url} is not available in this build.")))
    }
}

/// A fetcher that keeps every successful download in a directory and answers
/// later requests for the same URL from there.
pub struct CachedFetcher<F: Fetcher> {
    inner: F,
    dir: PathBuf,
}

impl<F: Fetcher> CachedFetcher<F> {
    /// Wrap `inner`, keeping its downloads in `dir`.
    pub fn new(inner: F, dir: impl Into<PathBuf>) -> CachedFetcher<F> {
        CachedFetcher { inner, dir: dir.into() }
    }

    /// The cache file of a URL: its readable tail plus a hash of the whole.
    pub fn cache_path(&self, url: &str) -> PathBuf {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in url.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let tail: String = url
            .rsplit('/')
            .next()
            .unwrap_or("download")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
            .take(80)
            .collect();
        self.dir.join(format!("{hash:016x}-{tail}"))
    }
}

impl<F: Fetcher> Fetcher for CachedFetcher<F> {
    fn fetch(&self, url: &str) -> Result<FetchResponse> {
        let path = self.cache_path(url);
        if let Ok(body) = std::fs::read(&path) {
            return Ok(FetchResponse { status: 200, body });
        }
        let response = self.inner.fetch(url)?;
        if response.ok() {
            // A cache that cannot be written is only slower next time.
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(&path, &response.body);
        }
        Ok(response)
    }
}

/// The HTTPS downloader of `coxeter-analysis` (feature `download`), for the
/// application: wrap it in a [`CachedFetcher`] to keep what it fetched.
#[cfg(feature = "download")]
impl Fetcher for coxeter_analysis::HttpsFetcher {
    fn fetch(&self, url: &str) -> Result<FetchResponse> {
        let response = coxeter_analysis::Fetcher::fetch(self, url)?;
        Ok(FetchResponse { status: response.status, body: response.body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counting(std::sync::atomic::AtomicUsize);
    impl Fetcher for Counting {
        fn fetch(&self, _url: &str) -> Result<FetchResponse> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(FetchResponse { status: 200, body: b"hello".to_vec() })
        }
    }

    #[test]
    fn memory_assets_and_errors() {
        let a = MemoryAssets::new();
        a.insert("models/x.json", "{\"a\":1}");
        assert_eq!(a.read_json("models/x.json").unwrap()["a"], 1);
        assert!(a.contains("models/x.json"));
        let err = a.read("models/none.json").unwrap_err().to_string();
        assert_eq!(err, "Could not load models/none.json (404)");
        assert!(!a.is_embedded());
        assert!(MemoryAssets::embedded().is_embedded());
    }

    #[test]
    fn dir_assets_refuse_parent_paths() {
        let d = DirAssets::new(coxeter_core::testkit::repo_root());
        assert!(d.contains("models/index.json"));
        assert!(d.read("../x").is_err());
    }

    #[test]
    fn cached_fetcher_downloads_once() {
        let dir = std::env::temp_dir().join(format!("coxeter-state-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = CachedFetcher::new(Counting(Default::default()), &dir);
        assert_eq!(f.fetch("https://example.org/a/b.xml").unwrap().body, b"hello");
        assert_eq!(f.fetch("https://example.org/a/b.xml").unwrap().body, b"hello");
        assert_eq!(f.inner.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
