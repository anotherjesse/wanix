use std::io::Write;

use wanix_fs::FsError;

use crate::help;

/// Captured native CLI output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

impl CliOutput {
    pub(crate) fn new(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    /// Returns stdout bytes that should be written to the native process.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns stderr bytes that should be written to the native process.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Returns the native process exit code.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

/// CLI execution error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    message: String,
    exit_code: i32,
}

impl CliError {
    pub(crate) fn new(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }

    pub(crate) fn usage(message: impl AsRef<str>) -> Self {
        Self::new(format!("{}\n\n{}", message.as_ref(), help::USAGE), 2)
    }

    /// Returns the native process exit code for this error.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<FsError> for CliError {
    fn from(error: FsError) -> Self {
        Self::new(error.to_string(), 1)
    }
}

pub(crate) fn write_process_output(
    output: &mut dyn Write,
    label: &str,
    bytes: &[u8],
) -> Result<(), CliError> {
    output
        .write_all(bytes)
        .map_err(|error| CliError::new(format!("failed to write process {label}: {error}"), 1))?;
    output
        .flush()
        .map_err(|error| CliError::new(format!("failed to flush process {label}: {error}"), 1))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use super::*;

    #[test]
    fn cli_output_accessors_preserve_captured_streams_and_status() {
        let output = CliOutput::new(b"out".to_vec(), b"err".to_vec(), 17);

        assert_eq!(output.stdout(), b"out");
        assert_eq!(output.stderr(), b"err");
        assert_eq!(output.exit_code(), 17);
    }

    #[test]
    fn cli_error_display_and_exit_code_are_stable() {
        let error = CliError::new("failed", 1);

        assert_eq!(error.to_string(), "failed");
        assert_eq!(error.exit_code(), 1);
        assert!(std::error::Error::source(&error).is_none());
    }

    #[test]
    fn cli_usage_error_includes_usage_and_uses_usage_exit_code() {
        let error = CliError::usage("bad args");

        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().starts_with("bad args\n\nusage:"));
    }

    #[test]
    fn fs_errors_convert_to_cli_errors_with_failure_exit_code() {
        let error = CliError::from(FsError::NotFound);

        assert_eq!(error.exit_code(), 1);
        assert_eq!(error.to_string(), "file does not exist");
    }

    #[test]
    fn write_process_output_writes_and_flushes_bytes() {
        let mut output = RecordingOutput::default();

        write_process_output(&mut output, "stdout", b"hello").unwrap();

        assert_eq!(output.bytes, b"hello");
        assert_eq!(output.flushes, 1);
    }

    #[test]
    fn write_process_output_reports_write_and_flush_errors() {
        let mut write_error = FailingOutput::fail_write();
        let error = write_process_output(&mut write_error, "stdout", b"hello").unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to write process stdout"));

        let mut flush_error = FailingOutput::fail_flush();
        let error = write_process_output(&mut flush_error, "stderr", b"hello").unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to flush process stderr"));
    }

    #[derive(Default)]
    struct RecordingOutput {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for RecordingOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    enum FailureMode {
        Write,
        Flush,
    }

    struct FailingOutput {
        mode: FailureMode,
    }

    impl FailingOutput {
        fn fail_write() -> Self {
            Self {
                mode: FailureMode::Write,
            }
        }

        fn fail_flush() -> Self {
            Self {
                mode: FailureMode::Flush,
            }
        }
    }

    impl Write for FailingOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.mode {
                FailureMode::Write => Err(io::Error::other("write failed")),
                FailureMode::Flush => Ok(buf.len()),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            match self.mode {
                FailureMode::Write => Ok(()),
                FailureMode::Flush => Err(io::Error::other("flush failed")),
            }
        }
    }
}
