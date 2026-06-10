use std::ffi::OsString;
use std::io;

#[cfg(not(unix))]
use super::run_with_process_io;
use super::{CliError, NativeRawTerminalMode, command_requests_raw_tty};
#[cfg(unix)]
use super::{UnixTerminalFds, run_with_process_io_and_terminal_fds};

/// Runs the native CLI against the current process stdio handles.
///
/// This is the entrypoint used by the `wanix-rust` binary. It keeps raw terminal
/// mode scoped to command execution so terminal state is restored before the
/// process exits.
///
/// # Errors
///
/// Returns a CLI error when raw terminal mode cannot be entered or command
/// execution fails before command-managed output is available.
pub fn run_native_process<I, S>(args: I) -> Result<i32, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let _raw_mode = if command_requests_raw_tty(&args) {
        NativeRawTerminalMode::enter_stdin_if_tty()?
    } else {
        None
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    // Deliberately NOT `io::stderr().lock()`: a resident command (`serve`,
    // `app serve`) runs for the life of the process, and a lifetime-held
    // `StderrLock` deadlocks every `eprintln!` from a background thread (the
    // restart supervisor, the guest exit watcher) — the reentrant stderr lock
    // only re-enters on the *same* thread. `Stderr` locks per write instead.
    let stderr = io::stderr();
    #[cfg(unix)]
    {
        run_with_process_io_and_terminal_fds(
            args,
            stdin.lock(),
            UnixTerminalFds::new(libc::STDIN_FILENO, libc::STDOUT_FILENO),
            stdout.lock(),
            stderr,
        )
    }
    #[cfg(not(unix))]
    {
        run_with_process_io(args, stdin.lock(), stdout.lock(), stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::run_native_process;

    #[test]
    fn native_process_reports_usage_without_raw_terminal_mode() {
        let error = run_native_process(["unknown-command"]).unwrap_err();

        assert_eq!(error.exit_code(), 2);
    }
}
