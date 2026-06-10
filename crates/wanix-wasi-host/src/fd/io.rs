//! WASI Preview 1 fd read/write imports.

use wanix_wasi::{Errno, WasiFd};
use wasmtime::{Caller, Linker, Result};

use super::super::ERRNO_SUCCESS;
use super::super::WasiHost;
use super::super::mem::{errno, memory, read_bytes, read_iovs, write_bytes, write_u32};
use super::super::wait::wait_read_ready;

pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "fd_write",
        |mut caller: Caller<'_, S>, fd: i32, iovs: i32, iovs_len: i32, nout: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let mut written = 0usize;
            for (ptr, len) in read_iovs(&mem, &mut caller, iovs, iovs_len)? {
                let buf = read_bytes(&mem, &mut caller, ptr, len)?;
                match caller
                    .data_mut()
                    .wasi()
                    .fd_write(WasiFd::new(fd as u32), &buf)
                {
                    Ok(n) => {
                        written += n;
                        if n < buf.len() {
                            break;
                        }
                    }
                    Err(e) => return errno(&mem, &mut caller, nout, written, e),
                }
            }
            let mem = memory(&mut caller)?;
            write_u32(&mem, &mut caller, nout, written as u32)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    linker.func_wrap(
        m,
        "fd_read",
        |mut caller: Caller<'_, S>, fd: i32, iovs: i32, iovs_len: i32, nout: i32| -> Result<i32> {
            // Blocking tier-2 read (ADR 0010): park this host thread until the
            // fd would produce data. Regular files are always ready (incl. at
            // EOF, where the read honestly returns 0); a queue-backed device fd
            // (#term/#pipe stdin) waits here for bytes. A readiness error falls
            // through so the read below reports its errno.
            wait_read_ready(caller.data_mut().wasi(), WasiFd::new(fd as u32));
            let mem = memory(&mut caller)?;
            // The kill seam: a cancelled (killed) task's park returns early —
            // report EINTR instead of entering a device read that could block
            // indefinitely (a quiet `#pipe`/`events` read blocks internally).
            // The wasm epoch interrupt then traps the guest on its next
            // instruction, so the run unwinds and records the killed exit.
            if caller.data_mut().wasi().is_cancelled() {
                return errno(&mem, &mut caller, nout, 0, Errno::Intr);
            }
            let mut total = 0usize;
            for (index, (ptr, len)) in read_iovs(&mem, &mut caller, iovs, iovs_len)?
                .into_iter()
                .enumerate()
            {
                // `WasiCtx::fd_read` itself parks stdio fds until ready, so a
                // continuation iov after a fully-filled one must only read
                // bytes that are already buffered (POSIX short read), never
                // wait for more.
                if index > 0
                    && !matches!(
                        caller
                            .data_mut()
                            .wasi()
                            .fd_read_ready(WasiFd::new(fd as u32)),
                        Ok(true)
                    )
                {
                    break;
                }
                let mut buf = vec![0u8; len];
                match caller
                    .data_mut()
                    .wasi()
                    .fd_read(WasiFd::new(fd as u32), &mut buf)
                {
                    Ok(n) => {
                        write_bytes(&mem, &mut caller, ptr, &buf[..n])?;
                        total += n;
                        if n < len {
                            break;
                        }
                    }
                    Err(e) => return errno(&mem, &mut caller, nout, total, e),
                }
            }
            let mem = memory(&mut caller)?;
            write_u32(&mem, &mut caller, nout, total as u32)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    Ok(())
}
