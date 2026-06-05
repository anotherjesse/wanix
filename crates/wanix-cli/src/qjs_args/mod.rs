use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use wanix_fs::NormalizedPath;

use crate::CliError;

mod command;
mod common;
mod restore;
mod snapshot;

pub(super) use command::{parse_qjs_command, parse_qjs_command_for};
pub(super) use restore::parse_qjs_restore_command;
pub(super) use snapshot::parse_qjs_snapshot_file_command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsCommand {
    pub(super) script_path: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<String>,
    pub(super) cwd: NormalizedPath,
    pub(super) stdin: Option<QjsStdin>,
    pub(super) event_loop_wait_budget: Duration,
    pub(super) ready_io_turns: usize,
    pub(super) interrupt_poll_budget: Option<usize>,
    pub(super) memory_limit_bytes: Option<u32>,
    pub(super) mounts: Vec<HostMount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QjsStdin {
    Bytes(Vec<u8>),
    File(PathBuf),
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostMount {
    pub(super) host_path: PathBuf,
    pub(super) guest_path: NormalizedPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsRestoreCommand {
    pub(super) before_script_path: PathBuf,
    pub(super) after_script_path: PathBuf,
    pub(super) before_args: Vec<String>,
    pub(super) after_args: Vec<String>,
    pub(super) before_env: Vec<String>,
    pub(super) after_env: Vec<String>,
    pub(super) cwd: NormalizedPath,
    pub(super) mounts: Vec<HostMount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QjsSnapshotFileCommand {
    pub(super) script_path: PathBuf,
    pub(super) snapshot_path: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<String>,
    pub(super) cwd: NormalizedPath,
    pub(super) stdin: Option<QjsStdin>,
    pub(super) event_loop_wait_budget: Duration,
    pub(super) ready_io_turns: usize,
    pub(super) interrupt_poll_budget: Option<usize>,
    pub(super) memory_limit_bytes: Option<u32>,
    pub(super) mounts: Vec<HostMount>,
}

pub(super) fn read_qjs_stdin(
    source: Option<QjsStdin>,
    process_stdin: &mut dyn Read,
) -> Result<Option<Vec<u8>>, CliError> {
    match source {
        None => Ok(None),
        Some(QjsStdin::Bytes(bytes)) => Ok(Some(bytes)),
        Some(QjsStdin::File(path)) => std::fs::read(&path).map(Some).map_err(|error| {
            CliError::new(
                format!("failed to read stdin file {}: {error}", path.display()),
                1,
            )
        }),
        Some(QjsStdin::Process) => {
            let mut bytes = Vec::new();
            process_stdin.read_to_end(&mut bytes).map_err(|error| {
                CliError::new(format!("failed to read process stdin: {error}"), 1)
            })?;
            Ok(Some(bytes))
        }
    }
}

fn set_qjs_stdin(
    stdin: &mut Option<QjsStdin>,
    source: QjsStdin,
    command: &str,
) -> Result<(), CliError> {
    if stdin.is_some() {
        return Err(CliError::usage(format!(
            "{command} accepts only one of --stdin or --stdin-file"
        )));
    }
    *stdin = Some(source);
    Ok(())
}

fn parse_host_mount(value: &str, label: &str) -> Result<HostMount, CliError> {
    let Some((host, guest)) = value.split_once('=') else {
        return Err(CliError::usage(format!("{label} expects HOST=GUEST")));
    };
    if host.is_empty() || guest.is_empty() {
        return Err(CliError::usage(format!("{label} expects HOST=GUEST")));
    }
    let guest_path = NormalizedPath::new(guest)?;
    if guest_path.as_str() == "." {
        return Err(CliError::usage(format!(
            "{label} guest path must not be . in this demo"
        )));
    }
    Ok(HostMount {
        host_path: PathBuf::from(host),
        guest_path,
    })
}

pub(super) fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} must be valid UTF-8")))
}

fn parse_duration_millis(arg: &OsString, label: &str) -> Result<Duration, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let millis = value
        .parse::<u64>()
        .map_err(|_| CliError::usage(format!("{label} expects a non-negative integer")))?;
    Ok(Duration::from_millis(millis))
}

fn parse_usize(arg: &OsString, label: &str) -> Result<usize, CliError> {
    let value = os_arg_to_string(arg, label)?;
    value
        .parse::<usize>()
        .map_err(|_| CliError::usage(format!("{label} expects a non-negative integer")))
}

fn parse_u32(arg: &OsString, label: &str) -> Result<u32, CliError> {
    let value = os_arg_to_string(arg, label)?;
    value
        .parse::<u32>()
        .map_err(|_| CliError::usage(format!("{label} expects a 32-bit non-negative integer")))
}

fn validate_env_line(line: &str, label: &str) -> Result<(), CliError> {
    let Some((key, _value)) = line.split_once('=') else {
        return Err(CliError::usage(format!("{label} expects KEY=VALUE")));
    };
    if key.is_empty() || key.chars().any(char::is_whitespace) || line.contains('\n') {
        return Err(CliError::usage(format!("{label} expects KEY=VALUE")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        QjsStdin, parse_duration_millis, parse_host_mount, parse_u32, parse_usize, set_qjs_stdin,
        validate_env_line,
    };

    #[test]
    fn qjs_stdin_source_can_only_be_set_once() {
        let mut stdin = None;

        set_qjs_stdin(&mut stdin, QjsStdin::Bytes(b"hello".to_vec()), "qjs").unwrap();
        let error = set_qjs_stdin(&mut stdin, QjsStdin::Process, "qjs").unwrap_err();

        assert_eq!(stdin, Some(QjsStdin::Bytes(b"hello".to_vec())));
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("qjs accepts only one of --stdin or --stdin-file")
        );
    }

    #[test]
    fn qjs_host_mount_parser_accepts_host_equals_guest() {
        let mount = parse_host_mount("/host/data=guest/data", "qjs --mount").unwrap();

        assert_eq!(mount.host_path, PathBuf::from("/host/data"));
        assert_eq!(mount.guest_path.as_str(), "guest/data");
    }

    #[test]
    fn qjs_host_mount_parser_rejects_boundary_errors() {
        for value in ["missing-separator", "=guest", "host=", "host=."] {
            let error = parse_host_mount(value, "qjs --mount").unwrap_err();
            assert_eq!(error.exit_code(), 2);
            assert!(
                error.to_string().contains("qjs --mount expects HOST=GUEST")
                    || error
                        .to_string()
                        .contains("qjs --mount guest path must not be .")
            );
        }
    }

    #[test]
    fn qjs_numeric_parsers_accept_expected_widths() {
        assert_eq!(
            parse_duration_millis(&OsString::from("25"), "qjs --event-loop-ms").unwrap(),
            Duration::from_millis(25)
        );
        assert_eq!(
            parse_usize(&OsString::from("7"), "qjs --ready-io-turns").unwrap(),
            7
        );
        assert_eq!(
            parse_u32(&OsString::from("1024"), "qjs --memory-limit-bytes").unwrap(),
            1024
        );
    }

    #[test]
    fn qjs_numeric_parsers_report_invalid_values() {
        assert_usage_error(
            parse_duration_millis(&OsString::from("-1"), "qjs --event-loop-ms"),
            "qjs --event-loop-ms expects a non-negative integer",
        );
        assert_usage_error(
            parse_usize(&OsString::from("abc"), "qjs --ready-io-turns"),
            "qjs --ready-io-turns expects a non-negative integer",
        );
        assert_usage_error(
            parse_u32(&OsString::from("4294967296"), "qjs --memory-limit-bytes"),
            "qjs --memory-limit-bytes expects a 32-bit non-negative integer",
        );
    }

    #[test]
    fn qjs_env_lines_require_nonempty_plain_keys() {
        validate_env_line("MODE=test", "qjs --env").unwrap();
        validate_env_line("EMPTY=", "qjs --env").unwrap();

        for value in ["MODE", "=empty", "BAD KEY=value", "BAD\nKEY=value"] {
            let error = validate_env_line(value, "qjs --env").unwrap_err();
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().contains("qjs --env expects KEY=VALUE"));
        }
    }

    fn assert_usage_error<T: std::fmt::Debug>(result: Result<T, crate::CliError>, expected: &str) {
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains(expected));
    }
}
