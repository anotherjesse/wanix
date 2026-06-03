//! Wire protocol helpers for Rust Wanix.
//!
//! Protocol modules stay below Wanix filesystem and runtime policy. They parse
//! and encode wire contracts so higher-level crates can decide how those
//! messages map to namespaces, tasks, transports, and host authorization.

pub mod p9;

pub use p9::*;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix wire protocol helpers";
