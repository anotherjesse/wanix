//! QuickJS/WASI task driver for Rust Wanix.
//!
//! This crate will adapt the `rust-wasi-quickjs` prototype into a Wanix task
//! driver. QuickJS runs inside a WASI reactor hosted by Wasmtime; Wanix
//! supplies the namespace, fds, host callbacks, module loading policy, and
//! snapshot reattachment policy.

use wanix_wasi::WasiConfig;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "quickjs wasi task driver";

/// Host configuration for a future QuickJS/WASI runtime.
#[derive(Debug, Clone)]
pub struct HostConfig {
    wasi: WasiConfig,
}

impl HostConfig {
    /// Creates a host config from Wanix-backed WASI settings.
    #[must_use]
    pub fn new(wasi: WasiConfig) -> Self {
        Self { wasi }
    }

    /// Returns the Wanix-backed WASI settings.
    #[must_use]
    pub fn wasi(&self) -> &WasiConfig {
        &self.wasi
    }
}

/// First externally visible demo target for this crate.
pub const FIRST_DEMO_TARGET: &str =
    "run JavaScript outside Chrome with access to a Wanix namespace";

/// Returns the WASI crate purpose, proving the intended dependency edge.
#[must_use]
pub fn wasi_contract_purpose() -> &'static str {
    wanix_wasi::CRATE_PURPOSE
}

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, FIRST_DEMO_TARGET, HostConfig, wasi_contract_purpose};
    use wanix_wasi::WasiConfig;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn host_config_wraps_wanix_wasi_settings() {
        let config = HostConfig::new(WasiConfig::default());

        assert_eq!(config.wasi().preopens()[0].guest_path().as_str(), ".");
        assert_eq!(wasi_contract_purpose(), "wanix-backed wasi imports");
        assert!(FIRST_DEMO_TARGET.contains("outside Chrome"));
    }
}
