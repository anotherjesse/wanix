//! Readiness parking for blocking tier-2 syscalls.
//!
//! The wait machinery lives in [`wanix_wasi::wait`] (it also backs the
//! blocking stdio reads inside [`wanix_wasi::WasiCtx::fd_read`]); this module
//! re-exports it for the linker's `fd_read` wait — which covers *every* fd of
//! a command-style wasm task, not just stdio — and for the `poll_oneoff`
//! backoff. See that module for the parking contract: regular byte files are
//! always ready (including at EOF), queue-backed device fds park with bounded
//! backoff, and a readiness error falls through so the read reports the errno.

pub(crate) use wanix_wasi::wait::{Backoff, wait_read_ready};

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
