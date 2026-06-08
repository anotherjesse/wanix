//! A [`SiteIo`] backed by `std::fs`, used for host generation and by the wasm
//! guest (where libstd's `std::fs` lowers to WASI against the task namespace).

use std::path::{Path, PathBuf};

use crate::{SiteEntry, SiteIo};

/// A [`SiteIo`] rooted at a host directory, using `std::fs` for all access.
///
/// The same type compiles for `wasm32-wasip1`, where `std::fs` is backed by
/// WASI calls against the guest task's preopened namespace — so the wasm SSG
/// reads the corpus and writes its output through the Wanix filesystem.
#[derive(Debug, Clone)]
pub struct StdFsIo {
    root: PathBuf,
}

impl StdFsIo {
    /// Creates a [`StdFsIo`] rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolves a `/`-separated relative path against the root.
    fn resolve(&self, rel: &str) -> PathBuf {
        if rel.is_empty() || rel == "." {
            return self.root.clone();
        }
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|s| !s.is_empty()) {
            p.push(part);
        }
        p
    }
}

impl SiteIo for StdFsIo {
    fn read_dir(&self, dir: &str) -> std::io::Result<Vec<SiteEntry>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.resolve(dir))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type()?.is_dir();
            out.push(SiteEntry { name, is_dir });
        }
        Ok(out)
    }

    fn read_to_string(&self, path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.resolve(path))
    }

    fn create_dir_all(&self, path: &str) -> std::io::Result<()> {
        let p: &Path = &self.resolve(path);
        std::fs::create_dir_all(p)
    }

    fn write(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::write(self.resolve(path), bytes)
    }
}
