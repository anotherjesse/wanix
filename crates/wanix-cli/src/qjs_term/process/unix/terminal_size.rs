use std::io;

use super::super::super::CliError;
use super::super::super::pump::TermResize;

pub(in crate::qjs_term) fn terminal_size_for_fd(
    fd: libc::c_int,
) -> Result<Option<TermResize>, CliError> {
    // SAFETY: `isatty` only observes the supplied file descriptor.
    if unsafe { libc::isatty(fd) } == 0 {
        return Ok(None);
    }
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    // SAFETY: `size` points to valid writable memory for TIOCGWINSZ.
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
        return Err(CliError::new(
            format!(
                "failed to read native terminal size: {}",
                io::Error::last_os_error()
            ),
            1,
        ));
    }
    // SAFETY: ioctl succeeded and initialized the winsize value.
    let size = unsafe { size.assume_init() };
    if size.ws_col == 0 || size.ws_row == 0 {
        return Ok(None);
    }
    Ok(Some(TermResize {
        columns: size.ws_col,
        rows: size.ws_row,
    }))
}
