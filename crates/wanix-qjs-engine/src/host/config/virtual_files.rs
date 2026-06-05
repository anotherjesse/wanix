use super::QuickJsHostConfig;
use crate::allocation::try_copy_bytes;
use anyhow::{Result, bail};
use std::sync::Arc;

pub(crate) const MAX_VIRTUAL_FILE_PATH_BYTES: usize = 4096;

impl QuickJsHostConfig {
    /// Returns the number of configured engine-only virtual fixture files.
    #[must_use]
    pub fn read_only_virtual_file_count(&self) -> usize {
        self.read_only_virtual_files.len()
    }

    pub(crate) fn has_read_only_virtual_files(&self) -> bool {
        !self.read_only_virtual_files.is_empty()
    }

    pub(crate) fn read_only_virtual_file(&self, absolute_path: &[u8]) -> Option<Arc<[u8]>> {
        self.read_only_virtual_files.get(absolute_path).cloned()
    }

    pub(crate) fn has_read_only_virtual_directory(&self, absolute_path: &[u8]) -> bool {
        if absolute_path == b"/" {
            return self.has_read_only_virtual_files();
        }
        let mut prefix = Vec::with_capacity(absolute_path.len() + 1);
        prefix.extend_from_slice(absolute_path);
        prefix.push(b'/');
        self.read_only_virtual_files
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(path, _bytes)| path.starts_with(&prefix))
    }

    /// Adds an immutable engine-only fixture file at an absolute guest path.
    ///
    /// This narrow surface is useful for standalone engine tests or static
    /// fixture bytes. Runtime hosts that own process, namespace, or fd semantics
    /// should attach a live WASI provider instead.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is not an absolute normalized guest file path,
    /// contains NUL, empty, `.`, `..`, or backslash components, or if `bytes`
    /// cannot be copied into host-owned storage.
    pub fn with_read_only_virtual_file(
        mut self,
        path: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<Self> {
        let path = normalize_absolute_virtual_file_path(path.as_ref())?;
        let bytes = try_copy_bytes(bytes.as_ref(), "read-only virtual file")?;
        self.read_only_virtual_files
            .insert(path, Arc::from(bytes.into_boxed_slice()));
        Ok(self)
    }

    /// Adds immutable engine-only fixture files from absolute guest paths.
    ///
    /// Later entries replace earlier entries with the same normalized path.
    ///
    /// # Errors
    ///
    /// Returns an error if any path is invalid or any file contents cannot be
    /// copied into host-owned storage.
    pub fn with_read_only_virtual_files<I, P, B>(mut self, files: I) -> Result<Self>
    where
        I: IntoIterator<Item = (P, B)>,
        P: AsRef<str>,
        B: AsRef<[u8]>,
    {
        for (path, bytes) in files {
            self = self.with_read_only_virtual_file(path, bytes)?;
        }
        Ok(self)
    }
}

fn normalize_absolute_virtual_file_path(path: &str) -> Result<Vec<u8>> {
    let bytes = path.as_bytes();
    if !bytes.starts_with(b"/") {
        bail!("read-only virtual file path must be absolute");
    }
    if bytes == b"/" {
        bail!("read-only virtual file path must name a file");
    }
    if bytes.len() > MAX_VIRTUAL_FILE_PATH_BYTES {
        bail!("read-only virtual file path must be at most {MAX_VIRTUAL_FILE_PATH_BYTES} bytes");
    }
    validate_virtual_path_components(&bytes[1..], "read-only virtual file path")?;
    Ok(bytes.to_vec())
}

pub(crate) fn validate_virtual_path_components(path: &[u8], label: &str) -> Result<()> {
    if path.is_empty() {
        bail!("{label} must not be empty");
    }
    for component in path.split(|byte| *byte == b'/') {
        if component.is_empty() {
            bail!("{label} must not contain empty components");
        }
        if component == b"." || component == b".." {
            bail!("{label} must not contain . or .. components");
        }
        if component.contains(&0) {
            bail!("{label} must not contain NUL bytes");
        }
        if component.contains(&b'\\') {
            bail!("{label} must not contain backslash components");
        }
    }
    Ok(())
}
