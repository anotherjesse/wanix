use std::ffi::OsString;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::NativeRawTerminalMode;

#[cfg(not(unix))]
use crate::CliError;

/// Returns true when the command asks the native binary to put stdin in raw mode.
///
/// `qjs-shell` opts in with `--raw`; interactive `sh` (no `-c`) is raw by
/// default because the guest REPL owns echo and line editing (ADR 0003) —
/// `sh -c LINE` and help requests stay in cooked mode.
#[must_use]
pub fn command_requests_raw_tty(args: &[OsString]) -> bool {
    match args {
        [command, rest @ ..] if command == "qjs-shell" => rest.iter().any(|arg| arg == "--raw"),
        [command, rest @ ..] if command == "sh" => !rest
            .iter()
            .any(|arg| arg == "-c" || arg == "--help" || arg == "-h"),
        _ => false,
    }
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
            "raw terminal mode (qjs-shell --raw, interactive sh) is currently supported only \
             on Unix hosts",
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

    #[test]
    fn interactive_sh_is_raw_but_sh_dash_c_and_help_stay_cooked() {
        assert!(super::command_requests_raw_tty(&[OsString::from("sh")]));
        assert!(super::command_requests_raw_tty(&[
            OsString::from("sh"),
            OsString::from("--mount-mesh"),
            OsString::from("iroh://abc=/vol"),
        ]));
        assert!(!super::command_requests_raw_tty(&[
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("echo hi"),
        ]));
        assert!(!super::command_requests_raw_tty(&[
            OsString::from("sh"),
            OsString::from("--help"),
        ]));
    }
}
