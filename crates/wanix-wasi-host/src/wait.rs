//! Readiness parking for blocking tier-2 syscalls.
//!
//! A command task gets blocking POSIX-style reads (ADR 0010 tier 2): when a
//! queue-backed device fd (`#term`, `#pipe`, …) has nothing to read yet, the
//! host thread running the guest parks here until the device reports
//! readiness. Regular byte files report ready unconditionally (including at
//! EOF, where a read honestly returns 0), so they never wait.
//!
//! Wanix files carry no wake primitive across the `File` trait, so parking is
//! readiness polling with exponential backoff capped at a small bound — never
//! a busy spin, never a missed wakeup (the queue is re-checked each interval).

use std::time::Duration;

use wanix_wasi::{WasiCtx, WasiFd};

const INITIAL_PARK: Duration = Duration::from_micros(100);
const MAX_PARK: Duration = Duration::from_millis(5);

/// Exponential sleep backoff between readiness re-checks.
pub(crate) struct Backoff {
    delay: Duration,
}

impl Backoff {
    pub(crate) fn new() -> Self {
        Self {
            delay: INITIAL_PARK,
        }
    }

    /// Sleeps the current interval, then doubles it up to [`MAX_PARK`].
    pub(crate) fn park(&mut self) {
        std::thread::sleep(self.delay);
        self.delay = (self.delay * 2).min(MAX_PARK);
    }
}

/// Blocks until a read on `fd` would produce data (or fail) without waiting.
///
/// Returns as soon as the fd reports ready. A readiness *error* (bad fd,
/// closed device, …) also returns immediately: the follow-up read is the one
/// that reports the errno to the guest, keeping a single error path.
pub(crate) fn wait_read_ready(ctx: &WasiCtx, fd: WasiFd) {
    let mut backoff = Backoff::new();
    while matches!(ctx.fd_read_ready(fd), Ok(false)) {
        backoff.park();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use wanix_fs::{File, FileType, FsResult, Metadata};
    use wanix_wasi::{WasiConfig, WasiCtx, WasiFd};

    use super::wait_read_ready;

    /// A scripted stdin-class device file: reads drain a shared queue and
    /// readiness reports whether the queue holds bytes — the `#term` shape.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct ScriptedStdin {
        queue: Arc<Mutex<VecDeque<u8>>>,
    }

    impl ScriptedStdin {
        pub(crate) fn feed(&self, bytes: &[u8]) {
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

    pub(crate) fn ctx_with_scripted_stdin() -> (WasiCtx, ScriptedStdin) {
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
    fn parks_until_another_thread_feeds_the_queue() {
        let (ctx, stdin) = ctx_with_scripted_stdin();
        let feeder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            stdin.feed(b"late");
        });
        // Blocks across the feeder's delay; returning at all proves the wait
        // observed the cross-thread readiness flip.
        wait_read_ready(&ctx, WasiFd::STDIN);
        let mut buf = [0u8; 8];
        assert_eq!(ctx_read(ctx, &mut buf), 4);
        assert_eq!(&buf[..4], b"late");
        feeder.join().expect("feeder thread");
    }

    #[test]
    fn readiness_error_returns_without_blocking() {
        let (ctx, _stdin) = ctx_with_scripted_stdin();
        // A bad fd errors on the readiness probe; the wait must fall through so
        // the read path reports the errno.
        wait_read_ready(&ctx, WasiFd::new(99));
    }

    fn ctx_read(mut ctx: WasiCtx, buf: &mut [u8]) -> usize {
        ctx.fd_read(WasiFd::STDIN, buf).expect("read ready fd")
    }
}
