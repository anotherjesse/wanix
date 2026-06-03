//! WASI Preview 1 imports backed by Wanix semantics.
//!
//! This crate will own the syscall boundary for Wasmtime-hosted Wanix tasks.
//! Filesystem calls should resolve through Wanix namespaces and task file
//! descriptors instead of delegating namespace behavior to the host operating
//! system or a generic WASI filesystem adapter.

use wanix_fs::NormalizedPath;
use wanix_vfs::Namespace;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix-backed wasi imports";

/// WASI errno values used by the initial adapter contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errno {
    /// Operation succeeded.
    Success,
    /// Bad file descriptor.
    Badf,
    /// Invalid input.
    Inval,
    /// File or directory missing.
    Noent,
    /// Operation not supported.
    Nosys,
    /// Capability rights are insufficient.
    Notcapable,
}

/// Configured preopen directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preopen {
    guest_path: NormalizedPath,
}

impl Preopen {
    /// Creates a preopen rooted at a Wanix namespace path.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `guest_path` is invalid.
    pub fn new(guest_path: impl AsRef<str>) -> wanix_fs::FsResult<Self> {
        Ok(Self {
            guest_path: NormalizedPath::new(guest_path)?,
        })
    }

    /// Returns the guest path exposed as a preopen.
    #[must_use]
    pub fn guest_path(&self) -> &NormalizedPath {
        &self.guest_path
    }
}

/// WASI host configuration backed by a Wanix namespace.
#[derive(Debug, Clone)]
pub struct WasiConfig {
    namespace: Namespace,
    preopens: Vec<Preopen>,
}

impl WasiConfig {
    /// Creates a config for a namespace.
    #[must_use]
    pub fn new(namespace: Namespace) -> Self {
        Self {
            namespace,
            preopens: vec![Preopen {
                guest_path: NormalizedPath::new(".").expect("root path is valid"),
            }],
        }
    }

    /// Returns the Wanix namespace backing the adapter.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Returns configured preopens.
    #[must_use]
    pub fn preopens(&self) -> &[Preopen] {
        &self.preopens
    }
}

impl Default for WasiConfig {
    fn default() -> Self {
        Self::new(Namespace::new())
    }
}

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, Errno, Preopen, WasiConfig};
    use wanix_vfs::Namespace;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn config_exposes_namespace_and_root_preopen() {
        let config = WasiConfig::new(Namespace::new());

        assert_eq!(config.preopens()[0].guest_path().as_str(), ".");
        assert!(config.namespace().bindings().is_empty());
        assert_eq!(Errno::Success, Errno::Success);
    }

    #[test]
    fn preopen_validates_guest_paths() {
        assert!(Preopen::new("root").is_ok());
        assert!(Preopen::new("/host").is_err());
    }
}
