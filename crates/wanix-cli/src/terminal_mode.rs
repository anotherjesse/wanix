use std::ffi::OsString;

use super::CliError;

/// Returns true when the command asks the native binary to put stdin in raw mode.
#[must_use]
pub fn command_requests_raw_tty(args: &[OsString]) -> bool {
    matches!(args, [command, rest @ ..] if command == "qjs-shell" && rest.iter().any(|arg| arg == "--raw"))
}

/// Restore-on-drop guard for native terminal mode.
#[cfg(unix)]
#[derive(Debug)]
pub struct NativeRawTerminalMode {
    fd: libc::c_int,
    original: libc::termios,
}

#[cfg(unix)]
impl NativeRawTerminalMode {
    /// Enters raw-ish terminal mode for native stdin when stdin is a TTY.
    ///
    /// Returns `Ok(None)` when stdin is not a terminal. The mode disables
    /// canonical input and OS echo but preserves signal generation, so Ctrl-C
    /// still reaches the host process.
    ///
    /// # Errors
    ///
    /// Returns a CLI error when termios state cannot be read or changed.
    pub fn enter_stdin_if_tty() -> Result<Option<Self>, CliError> {
        Self::enter_if_tty(libc::STDIN_FILENO)
    }

    fn enter_if_tty(fd: libc::c_int) -> Result<Option<Self>, CliError> {
        // SAFETY: `isatty` only observes the supplied file descriptor.
        if unsafe { libc::isatty(fd) } == 0 {
            return Ok(None);
        }

        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `original` points to valid writable memory for termios.
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
            return Err(termios_error("read native terminal mode"));
        }
        // SAFETY: `tcgetattr` succeeded and initialized `original`.
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN);
        raw.c_iflag &= !(libc::ICRNL | libc::IXON);
        raw.c_oflag &= !libc::OPOST;
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: `raw` is a termios value derived from the current stdin mode.
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(termios_error("enter native raw terminal mode"));
        }
        Ok(Some(Self { fd, original }))
    }
}

#[cfg(unix)]
impl Drop for NativeRawTerminalMode {
    fn drop(&mut self) {
        // SAFETY: `original` was captured from this fd with `tcgetattr`.
        let _ = unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original) };
    }
}

#[cfg(unix)]
fn termios_error(action: &str) -> CliError {
    CliError::new(
        format!("failed to {action}: {}", std::io::Error::last_os_error()),
        1,
    )
}

/// Restore-on-drop guard for native terminal mode.
#[cfg(not(unix))]
#[derive(Debug)]
pub struct NativeRawTerminalMode;

#[cfg(not(unix))]
impl NativeRawTerminalMode {
    /// Enters raw terminal mode for native stdin when supported.
    ///
    /// # Errors
    ///
    /// Returns a CLI error because raw terminal mode is currently implemented
    /// only on Unix hosts.
    pub fn enter_stdin_if_tty() -> Result<Option<Self>, CliError> {
        Err(CliError::new(
            "qjs-shell --raw is currently supported only on Unix hosts",
            1,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    #[test]
    fn raw_tty_request_is_command_specific() {
        assert!(super::command_requests_raw_tty(&[
            OsString::from("qjs-shell"),
            OsString::from("--raw")
        ]));
        assert!(!super::command_requests_raw_tty(&[
            OsString::from("qjs-term"),
            OsString::from("--raw")
        ]));
        assert!(!super::command_requests_raw_tty(&[
            OsString::from("qjs-shell"),
            OsString::from("--cwd"),
            OsString::from("app")
        ]));
        assert!(!super::command_requests_raw_tty(&[
            OsString::from("qjs-shell"),
            OsString::from("--raw-mode")
        ]));
    }

    #[cfg(unix)]
    #[test]
    fn raw_mode_noops_for_non_tty_fd() {
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let (reader, _writer) = UnixStream::pair().unwrap();

        assert!(
            super::NativeRawTerminalMode::enter_if_tty(reader.as_raw_fd())
                .unwrap()
                .is_none()
        );
    }
}
