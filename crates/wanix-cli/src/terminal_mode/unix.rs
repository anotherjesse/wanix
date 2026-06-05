use crate::CliError;
use crate::unix_fd::with_borrowed_fd;
use rustix::termios::{
    InputModes, LocalModes, OptionalActions, OutputModes, SpecialCodeIndex, Termios, isatty,
    tcgetattr, tcsetattr,
};

/// Restore-on-drop guard for native terminal mode.
#[derive(Debug)]
pub struct NativeRawTerminalMode {
    fd: libc::c_int,
    original: Termios,
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
        if !with_borrowed_fd(fd, |fd| isatty(fd)) {
            return Ok(None);
        }

        let original = with_borrowed_fd(fd, |fd| tcgetattr(fd))
            .map_err(|error| termios_error("read native terminal mode", error))?;
        let mut raw = original.clone();
        raw.local_modes -= LocalModes::ECHO | LocalModes::ICANON | LocalModes::IEXTEN;
        raw.input_modes -= InputModes::ICRNL | InputModes::IXON;
        raw.output_modes -= OutputModes::OPOST;
        raw.special_codes[SpecialCodeIndex::VMIN] = 1;
        raw.special_codes[SpecialCodeIndex::VTIME] = 0;
        with_borrowed_fd(fd, |fd| tcsetattr(fd, OptionalActions::Flush, &raw))
            .map_err(|error| termios_error("enter native raw terminal mode", error))?;
        Ok(Some(Self { fd, original }))
    }
}

impl Drop for NativeRawTerminalMode {
    fn drop(&mut self) {
        let _ = with_borrowed_fd(self.fd, |fd| {
            tcsetattr(fd, OptionalActions::Flush, &self.original)
        });
    }
}

fn termios_error(action: &str, error: rustix::io::Errno) -> CliError {
    CliError::new(format!("failed to {action}: {error}"), 1)
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
