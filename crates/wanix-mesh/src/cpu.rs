//! The iroh edge for Plan 9 cpu: run a remote task against a reverse-exported
//! namespace over QUIC.
//!
//! This is the network bridge for [`wanix_cpu`]. The synchronous, transport-
//! agnostic cpu core (the acceptor's `run_job`, the caller's `serve_export` /
//! `drive_control`, the role discriminator, and the `CpuEvent` / spec framing)
//! is reused **unchanged**; this module only opens the two QUIC bidi streams, the
//! caller writes the role bytes, and the async stream halves are bridged to the
//! sync core through the same [`BlockingDuplex`](crate::BlockingDuplex) seam the
//! 9P control plane uses.
//!
//! # The two streams on one connection
//!
//! A cpu job rides one QUIC connection carrying two bidi streams: a **control**
//! stream (the [`wanix_cpu::CpuJobSpec`] request, then the [`wanix_cpu::CpuEvent`]
//! result batch) and an **export** stream (the caller's reverse 9P server over a
//! scoped, read-only-by-default sub-namespace). The caller writes a 1-byte role
//! discriminator first on each stream so the acceptor can pair them regardless of
//! arrival order — see [`wanix_cpu::StreamRole`].
//!
//! # Trust
//!
//! Remote code execution is the sharpest capability in the mesh. The acceptor is
//! grant-allowlisted: a [`CpuAcceptor`] only runs jobs for peers an explicit
//! allowlist admits, and the CLI keeps the acceptor off the public endpoint, per
//! the blueprint's "exec-device export stays local-trust until public auth
//! lands". The caller's reverse export is itself scoped and read-only by default
//! ([`wanix_cpu::ExportScope`]), so neither side hands the other its whole root.

mod handler;

pub use handler::{CpuAcceptor, CpuDialer, CpuJobReport, TaskTableFactory};

/// The ALPN protocol identifier for the Wanix cpu plane over QUIC.
///
/// Distinct from [`crate::WANIX_9P_ALPN`]: a cpu connection multiplexes a control
/// and an export bidi stream and is gated by the exec allowlist, so it is its own
/// ALPN rather than a role on the 9P plane.
pub const WANIX_CPU_ALPN: &[u8] = b"wanix/cpu/1";

#[cfg(test)]
mod tests {
    use super::WANIX_CPU_ALPN;

    #[test]
    fn alpn_is_the_wanix_cpu_contract() {
        assert_eq!(WANIX_CPU_ALPN, b"wanix/cpu/1");
    }
}
