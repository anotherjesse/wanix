use std::ffi::OsString;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::NativeRawTerminalMode;

#[cfg(not(unix))]
use crate::CliError;

/// Returns true when the command asks the native binary to put stdin in raw mode.
#[must_use]
pub fn command_requests_raw_tty(args: &[OsString]) -> bool {
    matches!(args, [command, rest @ ..] if command == "qjs-shell" && rest.iter().any(|arg| arg == "--raw"))
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
}
