use crate::CliError;

/// Restore-on-drop guard for native terminal mode.
#[derive(Debug)]
pub struct NativeRawTerminalMode {
    fd: libc::c_int,
    original: libc::termios,
}

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

impl Drop for NativeRawTerminalMode {
    fn drop(&mut self) {
        // SAFETY: `original` was captured from this fd with `tcgetattr`.
        let _ = unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original) };
    }
}

fn termios_error(action: &str) -> CliError {
    CliError::new(
        format!("failed to {action}: {}", std::io::Error::last_os_error()),
        1,
    )
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn raw_mode_noops_for_non_tty_fd() {
        let (reader, _writer) = UnixStream::pair().unwrap();

        assert!(
            super::NativeRawTerminalMode::enter_if_tty(reader.as_raw_fd())
                .unwrap()
                .is_none()
        );
    }
}
