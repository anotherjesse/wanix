use std::ffi::OsString;
use std::path::PathBuf;

use super::QjsSnapshotFileCommand;
use super::common::{QjsOptionParse, QjsRunOptions, parse_common_qjs_option, parse_script_args};
use crate::CliError;

pub(crate) fn parse_qjs_snapshot_file_command(
    args: &[OsString],
    command: &str,
) -> Result<QjsSnapshotFileCommand, CliError> {
    let mut options = QjsRunOptions::new()?;
    let mut snapshot_path = None;
    let mut i = 0;
    while i < args.len() {
        match parse_common_qjs_option(args, &mut i, command, &mut options)? {
            QjsOptionParse::Consumed => continue,
            QjsOptionParse::Separator => break,
            QjsOptionParse::Unknown => {}
        }

        if args[i] == "--snapshot" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage(format!("{command} --snapshot expects FILE")))?;
            if snapshot_path.is_some() {
                return Err(CliError::usage(format!(
                    "{command} accepts only one --snapshot"
                )));
            }
            snapshot_path = Some(PathBuf::from(value));
            i += 1;
        } else {
            break;
        }
    }

    let snapshot_path = snapshot_path
        .ok_or_else(|| CliError::usage(format!("{command} requires --snapshot FILE")))?;
    let script = args
        .get(i)
        .ok_or_else(|| CliError::usage(format!("{command} expects a script path")))?;
    let script_path = PathBuf::from(script);
    i += 1;

    let js_args = parse_script_args(args, &mut i, command)?;

    Ok(options.into_snapshot_command(script_path, snapshot_path, js_args))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::parse_qjs_snapshot_file_command;

    #[test]
    fn parse_qjs_snapshot_command_applies_options_snapshot_and_script_args() {
        let command = parse_qjs_snapshot_file_command(
            &os_args([
                "--env",
                "MODE=test",
                "--event-loop-ms",
                "25",
                "--snapshot",
                "state.bin",
                "main.js",
                "--",
                "--guest-flag",
                "value",
            ]),
            "qjs-snapshot",
        )
        .unwrap();

        assert_eq!(command.script_path, PathBuf::from("main.js"));
        assert_eq!(command.snapshot_path, PathBuf::from("state.bin"));
        assert_eq!(command.args, vec!["--guest-flag", "value"]);
        assert_eq!(command.env, vec!["MODE=test"]);
        assert_eq!(command.event_loop_wait_budget, Duration::from_millis(25));
    }

    #[test]
    fn parse_qjs_snapshot_command_stops_snapshot_options_at_script_path() {
        let command = parse_qjs_snapshot_file_command(
            &os_args([
                "--snapshot",
                "state.bin",
                "main.js",
                "--snapshot",
                "guest-value",
            ]),
            "qjs-snapshot",
        )
        .unwrap();

        assert_eq!(command.script_path, PathBuf::from("main.js"));
        assert_eq!(command.snapshot_path, PathBuf::from("state.bin"));
        assert_eq!(command.args, vec!["--snapshot", "guest-value"]);
    }

    #[test]
    fn parse_qjs_snapshot_command_reports_snapshot_usage_errors() {
        assert_usage_error(
            parse_qjs_snapshot_file_command(&[], "qjs-snapshot"),
            "qjs-snapshot requires --snapshot FILE",
        );
        assert_usage_error(
            parse_qjs_snapshot_file_command(&os_args(["--snapshot"]), "qjs-snapshot"),
            "qjs-snapshot --snapshot expects FILE",
        );
        assert_usage_error(
            parse_qjs_snapshot_file_command(
                &os_args(["--snapshot", "one.bin", "--snapshot", "two.bin", "main.js"]),
                "qjs-snapshot",
            ),
            "qjs-snapshot accepts only one --snapshot",
        );
        assert_usage_error(
            parse_qjs_snapshot_file_command(&os_args(["--snapshot", "state.bin"]), "qjs-snapshot"),
            "qjs-snapshot expects a script path",
        );
    }

    fn assert_usage_error<T: std::fmt::Debug>(result: Result<T, crate::CliError>, expected: &str) {
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains(expected));
    }

    fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }
}
