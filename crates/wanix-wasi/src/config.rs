use wanix_fs::{FsResult, NormalizedPath};
use wanix_vfs::Namespace;

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
    pub fn new(guest_path: impl AsRef<str>) -> FsResult<Self> {
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

    /// Adds a preopen and returns the updated config.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `guest_path` is invalid.
    pub fn with_preopen(mut self, guest_path: impl AsRef<str>) -> FsResult<Self> {
        self.preopens.push(Preopen::new(guest_path)?);
        Ok(self)
    }
}

impl Default for WasiConfig {
    fn default() -> Self {
        Self::new(Namespace::new())
    }
}
