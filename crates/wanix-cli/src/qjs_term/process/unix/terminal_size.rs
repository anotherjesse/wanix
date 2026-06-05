use super::super::super::CliError;
use super::super::super::pump::TermResize;
use crate::unix_fd::with_borrowed_fd;
use rustix::termios::{isatty, tcgetwinsize};

pub(in crate::qjs_term) fn terminal_size_for_fd(
    fd: libc::c_int,
) -> Result<Option<TermResize>, CliError> {
    if !with_borrowed_fd(fd, |fd| isatty(fd)) {
        return Ok(None);
    }

    let size = with_borrowed_fd(fd, |fd| tcgetwinsize(fd)).map_err(|error| {
        CliError::new(format!("failed to read native terminal size: {error}"), 1)
    })?;
    if size.ws_col == 0 || size.ws_row == 0 {
        return Ok(None);
    }
    Ok(Some(TermResize {
        columns: size.ws_col,
        rows: size.ws_row,
    }))
}
