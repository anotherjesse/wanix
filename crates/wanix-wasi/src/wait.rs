//! Readiness parking for blocking stdio reads.
//!
//! A command task gets blocking POSIX-style reads (ADR 0010 tier 2): when a
//! queue-backed device file (`#term`, `#pipe`, …) attached to a stdio fd has
//! nothing to read yet, the host thread running the guest parks here until
//! the device reports readiness. Regular byte files report ready
//! unconditionally (including at EOF, where a read honestly returns 0), so
//! they never wait.
//!
//! Wanix files carry no wake primitive across the `File` trait, so parking is
//! readiness polling with exponential backoff capped at a small bound — never
//! a busy spin, never a missed wakeup (the queue is re-checked each interval).
//!
//! This is the shared home of the wait machinery: [`crate::WasiCtx::fd_read`]
//! parks here for stdio fds, and the `wanix-wasi-host` Preview 1 linker
//! re-exports this module for its all-fd `fd_read` wait and its `poll_oneoff`
//! backoff.

use std::time::Duration;

use crate::{WasiCtx, WasiFd};

const INITIAL_PARK: Duration = Duration::from_micros(100);
const MAX_PARK: Duration = Duration::from_millis(5);

/// Exponential sleep backoff between readiness re-checks.
#[derive(Debug)]
pub struct Backoff {
    delay: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    /// Creates a backoff starting at the initial park interval.
    #[must_use]
    pub fn new() -> Self {
        Self {
            delay: INITIAL_PARK,
        }
    }

    /// Sleeps the current interval, then doubles it up to the cap.
    pub fn park(&mut self) {
        std::thread::sleep(self.delay);
        self.delay = (self.delay * 2).min(MAX_PARK);
    }
}

/// Blocks until a read on `fd` would produce data (or fail) without waiting,
/// or the task is cancelled (killed).
///
/// Returns as soon as the fd reports ready. A readiness *error* (bad fd,
/// closed device, …) also returns immediately: the follow-up read is the one
/// that reports the errno to the guest, keeping a single error path.
///
/// Cancellation ([`WasiCtx::is_cancelled`], the task kill seam) is checked on
/// every wake, so a killed task parked here returns within one park interval
/// instead of waiting for readiness that may never come; the caller must then
/// consult `ctx.is_cancelled()` and report [`crate::Errno::Intr`] rather than
/// entering a device read that could block indefinitely.
pub fn wait_read_ready(ctx: &WasiCtx, fd: WasiFd) {
    let mut backoff = Backoff::new();
    while matches!(ctx.fd_read_ready(fd), Ok(false)) {
        if ctx.is_cancelled() {
            return;
        }
        backoff.park();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use wanix_fs::{File, FileType, FsResult, Metadata};

    use crate::{WasiConfig, WasiCtx, WasiFd};

    use super::wait_read_ready;

    /// A scripted stdin-class device file: reads drain a shared queue without
    /// blocking (empty reads return 0) and readiness reports whether the
    /// queue holds bytes — the `#term` program-side shape.
    #[derive(Debug, Clone, Default)]
    struct ScriptedStdin {
        queue: Arc<Mutex<VecDeque<u8>>>,
    }

    impl ScriptedStdin {
        fn feed(&self, bytes: &[u8]) {
            self.queue.lock().unwrap().extend(bytes.iter().copied());
        }
    }

    impl File for ScriptedStdin {
        fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
            let mut queue = self.queue.lock().unwrap();
            let len = queue.len().min(buf.len());
            for slot in buf.iter_mut().take(len) {
                *slot = queue.pop_front().expect("queued byte");
            }
            Ok(len)
        }

        fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
            Ok(buf.len())
        }

        fn read_ready(&self) -> FsResult<bool> {
            Ok(!self.queue.lock().unwrap().is_empty())
        }

        fn metadata(&self) -> FsResult<Metadata> {
            Ok(Metadata::new(FileType::File, 0, 0o666))
        }
    }

    fn ctx_with_scripted_stdin() -> (WasiCtx, ScriptedStdin) {
        let stdin = ScriptedStdin::default();
        let config =
            WasiConfig::new(Default::default()).with_stdin(Box::new(stdin.clone()), "stdin");
        (WasiCtx::new(config), stdin)
    }

    #[test]
    fn returns_immediately_when_ready() {
        let (ctx, stdin) = ctx_with_scripted_stdin();
        stdin.feed(b"x");
        wait_read_ready(&ctx, WasiFd::STDIN);
        let mut buf = [0u8; 1];
        assert_eq!(ctx_read(ctx, &mut buf), 1);
        assert_eq!(&buf, b"x");
    }

    #[test]
    fn readiness_error_returns_without_blocking() {
        let (ctx, _stdin) = ctx_with_scripted_stdin();
        // A bad fd errors on the readiness probe; the wait must fall through so
        // the read path reports the errno.
        wait_read_ready(&ctx, WasiFd::new(99));
    }

    #[test]
    fn a_killed_task_parked_on_a_quiet_stdin_gets_eintr_within_the_park_interval() {
        // The kill seam (deadline-bounded): the scripted stdin never becomes
        // ready, so only the cancel probe can end the park inside `fd_read`.
        // After the probe flips, the blocked read must return `Errno::Intr`
        // (never enter a device read that could block indefinitely), well
        // within the test deadline.
        use std::sync::atomic::{AtomicBool, Ordering};

        use crate::{CancelToken, Errno};

        let killed = Arc::new(AtomicBool::new(false));
        let probe = Arc::clone(&killed);
        let stdin = ScriptedStdin::default(); // quiet: readiness stays false
        let mut ctx = WasiCtx::new(
            WasiConfig::new(Default::default())
                .with_stdin(Box::new(stdin), "stdin")
                .with_cancel_token(CancelToken::new(move || probe.load(Ordering::Relaxed))),
        );
        let killer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            killed.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let mut buf = [0u8; 8];
        let result = ctx.fd_read(WasiFd::STDIN, &mut buf);
        assert_eq!(result, Err(Errno::Intr), "a cancelled park reports EINTR");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the kill must land within the park's poll interval"
        );
        killer.join().expect("killer thread");
    }

    #[test]
    fn fd_read_parks_a_stdio_device_until_another_thread_feeds_it() {
        // The blocking stdio-read contract itself: the scripted device returns
        // 0 from an empty read, so only the readiness park inside
        // `WasiCtx::fd_read` can make this read return the late-fed bytes.
        let (mut ctx, stdin) = ctx_with_scripted_stdin();
        let feeder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            stdin.feed(b"late");
        });
        let mut buf = [0u8; 8];
        let count = ctx.fd_read(WasiFd::STDIN, &mut buf).expect("blocking read");
        assert_eq!(count, 4);
        assert_eq!(&buf[..4], b"late");
        feeder.join().expect("feeder thread");
    }

    fn ctx_read(mut ctx: WasiCtx, buf: &mut [u8]) -> usize {
        ctx.fd_read(WasiFd::STDIN, buf).expect("read ready fd")
    }
}
