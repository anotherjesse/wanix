use std::ffi::OsString;
use std::path::PathBuf;

use super::QjsCommand;
use super::common::{QjsOptionParse, QjsRunOptions, parse_common_qjs_option, parse_script_args};
use crate::CliError;

pub(crate) fn parse_qjs_command(args: &[OsString]) -> Result<QjsCommand, CliError> {
    parse_qjs_command_for(args, "qjs")
}

pub(crate) fn parse_qjs_command_for(
    args: &[OsString],
    command: &str,
) -> Result<QjsCommand, CliError> {
    let mut options = QjsRunOptions::new()?;
    let mut i = 0;
    while i < args.len() {
        match parse_common_qjs_option(args, &mut i, command, &mut options)? {
            QjsOptionParse::Consumed => {}
            QjsOptionParse::Separator | QjsOptionParse::Unknown => break,
        }
    }

    let script = args
        .get(i)
        .ok_or_else(|| CliError::usage(format!("{command} expects a script path")))?;
    let script_path = PathBuf::from(script);
    i += 1;

    let js_args = parse_script_args(args, &mut i, command)?;

    Ok(options.into_command(script_path, js_args))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{parse_qjs_command, parse_qjs_command_for};

    #[test]
    fn parse_qjs_command_applies_common_options_before_script() {
        let command = parse_qjs_command(&os_args([
            "--env",
            "MODE=test",
            "--cwd",
            "work",
            "--event-loop-ms",
            "25",
            "main.js",
            "--",
            "--guest-flag",
            "value",
        ]))
        .unwrap();

        assert_eq!(command.script_path, PathBuf::from("main.js"));
        assert_eq!(command.args, vec!["--guest-flag", "value"]);
        assert_eq!(command.env, vec!["MODE=test"]);
        assert_eq!(command.cwd.as_str(), "work");
        assert_eq!(command.event_loop_wait_budget, Duration::from_millis(25));
    }

    #[test]
    fn parse_qjs_command_stops_common_options_at_script_path() {
        let command =
            parse_qjs_command(&os_args(["main.js", "--env", "GUEST_ARG=not-an-option"])).unwrap();

        assert_eq!(command.script_path, PathBuf::from("main.js"));
        assert_eq!(command.args, vec!["--env", "GUEST_ARG=not-an-option"]);
        assert!(command.env.is_empty());
    }

    #[test]
    fn parse_qjs_command_for_uses_command_label_in_errors() {
        let error = parse_qjs_command_for(&[], "qjs-term").unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("qjs-term expects a script path"));

        let error = parse_qjs_command_for(&os_args(["--ready-io-turns"]), "qjs-term").unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("qjs-term --ready-io-turns expects a count")
        );
    }

    fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }
}
