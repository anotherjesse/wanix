//! [`CancelToken`]: the kill seam for host-thread parks.
//!
//! A task killed through `#task/<id>/ctl kill` dies promptly when the guest is
//! *executing* (the wasm driver's epoch interrupt), but a tier-2 task parked in
//! a blocking host read (quiet stdin, quiet `#pipe`, quiet `events` stream)
//! never returns to guest code on its own. The token closes that gap without
//! threading `Task` through the WASI crates: a probe closure is injected once
//! into [`crate::WasiConfig`] at task start (see [`crate::task_wasi_config`],
//! whose probe reads `Task::kill_requested`), and every shared park loop
//! ([`crate::wait::wait_read_ready`], the `wanix-wasi-host` `poll_oneoff`
//! backoff) checks it each wake. A cancelled park returns and the blocking
//! call reports [`crate::Errno::Intr`] instead of entering a device read that
//! could block indefinitely — so the kill is observed within one park interval
//! (≤ 5 ms) instead of waiting for readiness that may never come.
//!
//! A context built without a token is never cancelled: non-task embeddings
//! (fixtures, engine tests) keep today's behavior.

use std::fmt;
use std::sync::Arc;

/// A cancellation probe checked by blocking host parks.
///
/// Cheap to clone; the probe must be fast and lock-light, since park loops
/// consult it on every wake (up to every 100 µs).
#[derive(Clone)]
pub struct CancelToken {
    probe: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl CancelToken {
    /// Creates a token from a probe returning whether cancellation is
    /// requested.
    #[must_use]
    pub fn new(probe: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            probe: Arc::new(probe),
        }
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        (self.probe)()
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn token_reports_the_probe_state() {
        let flag = Arc::new(AtomicBool::new(false));
        let probe = Arc::clone(&flag);
        let token = CancelToken::new(move || probe.load(Ordering::Relaxed));
        assert!(!token.is_cancelled());
        flag.store(true, Ordering::Relaxed);
        assert!(token.is_cancelled());
    }
}
