//! WASI Preview 1 fd read/write imports.

use wanix_wasi::WasiFd;
use wasmtime::{Caller, Linker, Result};

use super::super::ERRNO_SUCCESS;
use super::super::WasiHost;
use super::super::mem::{errno, memory, read_bytes, read_iovs, write_bytes, write_u32};

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
            let mem = memory(&mut caller)?;
            let mut total = 0usize;
            for (ptr, len) in read_iovs(&mem, &mut caller, iovs, iovs_len)? {
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
