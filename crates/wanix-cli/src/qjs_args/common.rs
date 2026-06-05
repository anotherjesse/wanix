use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use wanix_fs::NormalizedPath;

use super::{
    HostMount, QjsCommand, QjsSnapshotFileCommand, QjsStdin, os_arg_to_string,
    parse_duration_millis, parse_host_mount, parse_u32, parse_usize, set_qjs_stdin,
    validate_env_line,
};
use crate::CliError;

mod handlers;
mod options;

use handlers::COMMON_QJS_OPTION_HANDLERS;
use options::CommonQjsOption;

pub(super) struct QjsRunOptions {
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<QjsStdin>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
    interrupt_poll_budget: Option<usize>,
    memory_limit_bytes: Option<u32>,
    mounts: Vec<HostMount>,
}

impl QjsRunOptions {
    pub(super) fn new() -> Result<Self, CliError> {
        Ok(Self {
            env: Vec::new(),
            cwd: NormalizedPath::new(".")?,
            stdin: None,
            event_loop_wait_budget: Duration::ZERO,
            ready_io_turns: 1,
            interrupt_poll_budget: None,
            memory_limit_bytes: None,
            mounts: Vec::new(),
        })
    }

    pub(super) fn into_command(self, script_path: PathBuf, args: Vec<String>) -> QjsCommand {
        QjsCommand {
            script_path,
            args,
            env: self.env,
            cwd: self.cwd,
            stdin: self.stdin,
            event_loop_wait_budget: self.event_loop_wait_budget,
            ready_io_turns: self.ready_io_turns,
            interrupt_poll_budget: self.interrupt_poll_budget,
            memory_limit_bytes: self.memory_limit_bytes,
            mounts: self.mounts,
        }
    }

    pub(super) fn into_snapshot_command(
        self,
        script_path: PathBuf,
        snapshot_path: PathBuf,
        args: Vec<String>,
    ) -> QjsSnapshotFileCommand {
        QjsSnapshotFileCommand {
            script_path,
            snapshot_path,
            args,
            env: self.env,
            cwd: self.cwd,
            stdin: self.stdin,
            event_loop_wait_budget: self.event_loop_wait_budget,
            ready_io_turns: self.ready_io_turns,
            interrupt_poll_budget: self.interrupt_poll_budget,
            memory_limit_bytes: self.memory_limit_bytes,
            mounts: self.mounts,
        }
    }

    fn add_env(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        let value = os_arg_to_string(value, label)?;
        validate_env_line(&value, label)?;
        self.env.push(value);
        Ok(())
    }

    fn set_cwd(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.cwd = NormalizedPath::new(os_arg_to_string(value, label)?)?;
        Ok(())
    }

    fn set_stdin_bytes(
        &mut self,
        value: &OsString,
        command: &str,
        label: &str,
    ) -> Result<(), CliError> {
        set_qjs_stdin(
            &mut self.stdin,
            QjsStdin::Bytes(os_arg_to_string(value, label)?.into_bytes()),
            command,
        )
    }

    fn set_stdin_file(&mut self, value: &OsString, command: &str) -> Result<(), CliError> {
        let source = if value == "-" {
            QjsStdin::Process
        } else {
            QjsStdin::File(PathBuf::from(value))
        };
        set_qjs_stdin(&mut self.stdin, source, command)
    }

    fn set_event_loop_ms(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.event_loop_wait_budget = parse_duration_millis(value, label)?;
        Ok(())
    }

    fn set_ready_io_turns(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.ready_io_turns = parse_usize(value, label)?;
        Ok(())
    }

    fn set_interrupt_after(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.interrupt_poll_budget = Some(parse_usize(value, label)?);
        Ok(())
    }

    fn set_memory_limit_bytes(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.memory_limit_bytes = Some(parse_u32(value, label)?);
        Ok(())
    }

    fn add_mount(&mut self, value: &OsString, label: &str) -> Result<(), CliError> {
        self.mounts
            .push(parse_host_mount(&os_arg_to_string(value, label)?, label)?);
        Ok(())
    }
}

pub(super) enum QjsOptionParse {
    Consumed,
    Separator,
    Unknown,
}

pub(super) fn parse_common_qjs_option(
    args: &[OsString],
    index: &mut usize,
    command: &str,
    options: &mut QjsRunOptions,
) -> Result<QjsOptionParse, CliError> {
    let Some(option) = CommonQjsOption::from_arg(&args[*index]) else {
        return Ok(QjsOptionParse::Unknown);
    };
    if option == CommonQjsOption::Separator {
        *index += 1;
        return Ok(QjsOptionParse::Separator);
    }
    let value = qjs_option_value(args, index, command, option.name(), option.expected())?;
    apply_common_qjs_option(option, value, command, options)?;
    Ok(QjsOptionParse::Consumed)
}

fn apply_common_qjs_option(
    option: CommonQjsOption,
    value: &OsString,
    command: &str,
    options: &mut QjsRunOptions,
) -> Result<(), CliError> {
    let Some(handler) = COMMON_QJS_OPTION_HANDLERS
        .iter()
        .find_map(|(candidate, handler)| (*candidate == option).then_some(handler))
    else {
        return Err(CliError::new(
            format!("internal qjs parser missing handler for {}", option.name()),
            1,
        ));
    };
    handler(
        value,
        command,
        &format!("{command} {}", option.name()),
        options,
    )
}

fn qjs_option_value<'a>(
    args: &'a [OsString],
    index: &mut usize,
    command: &str,
    option: &str,
    expected: &str,
) -> Result<&'a OsString, CliError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| CliError::usage(format!("{command} {option} expects {expected}")))?;
    *index += 1;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{QjsOptionParse, QjsRunOptions, parse_common_qjs_option};
    use crate::qjs_args::QjsStdin;

    #[test]
    fn common_qjs_options_apply_state_and_advance_index() {
        let args = os_args([
            "--env",
            "MODE=test",
            "--cwd",
            "work/dir",
            "--event-loop-ms",
            "50",
            "--ready-io-turns",
            "3",
            "--interrupt-after",
            "99",
            "--memory-limit-bytes",
            "4096",
            "--mount",
            "/host/data=guest/data",
        ]);
        let mut options = QjsRunOptions::new().unwrap();
        let mut index = 0;

        while index < args.len() {
            assert!(matches!(
                parse_common_qjs_option(&args, &mut index, "qjs", &mut options).unwrap(),
                QjsOptionParse::Consumed
            ));
        }

        let command = options.into_command(PathBuf::from("main.js"), vec!["arg".to_owned()]);
        assert_eq!(command.script_path, PathBuf::from("main.js"));
        assert_eq!(command.args, vec!["arg"]);
        assert_eq!(command.env, vec!["MODE=test"]);
        assert_eq!(command.cwd.as_str(), "work/dir");
        assert_eq!(command.event_loop_wait_budget, Duration::from_millis(50));
        assert_eq!(command.ready_io_turns, 3);
        assert_eq!(command.interrupt_poll_budget, Some(99));
        assert_eq!(command.memory_limit_bytes, Some(4096));
        assert_eq!(command.mounts.len(), 1);
        assert_eq!(command.mounts[0].host_path, PathBuf::from("/host/data"));
        assert_eq!(command.mounts[0].guest_path.as_str(), "guest/data");
    }

    #[test]
    fn common_qjs_stdin_options_select_source_variants() {
        let bytes =
            parse_single_option("--stdin", "hello").into_command(PathBuf::new(), Vec::new());
        assert_eq!(bytes.stdin, Some(QjsStdin::Bytes(b"hello".to_vec())));

        let file = parse_single_option("--stdin-file", "input.txt")
            .into_command(PathBuf::new(), Vec::new());
        assert_eq!(file.stdin, Some(QjsStdin::File(PathBuf::from("input.txt"))));

        let process =
            parse_single_option("--stdin-file", "-").into_command(PathBuf::new(), Vec::new());
        assert_eq!(process.stdin, Some(QjsStdin::Process));
    }

    #[test]
    fn common_qjs_options_project_into_snapshot_command() {
        let options = parse_options(["--env", "MODE=test", "--cwd", "snapshot/cwd"]);

        let command = options.into_snapshot_command(
            PathBuf::from("before.js"),
            PathBuf::from("state.bin"),
            vec!["one".to_owned()],
        );

        assert_eq!(command.script_path, PathBuf::from("before.js"));
        assert_eq!(command.snapshot_path, PathBuf::from("state.bin"));
        assert_eq!(command.args, vec!["one"]);
        assert_eq!(command.env, vec!["MODE=test"]);
        assert_eq!(command.cwd.as_str(), "snapshot/cwd");
    }

    #[test]
    fn common_qjs_option_parser_reports_separator_unknown_and_missing_value() {
        let mut options = QjsRunOptions::new().unwrap();
        let args = os_args(["--", "--env"]);
        let mut index = 0;
        assert!(matches!(
            parse_common_qjs_option(&args, &mut index, "qjs", &mut options).unwrap(),
            QjsOptionParse::Separator
        ));
        assert_eq!(index, 1);

        let args = os_args(["--not-qjs", "value"]);
        let mut index = 0;
        assert!(matches!(
            parse_common_qjs_option(&args, &mut index, "qjs", &mut options).unwrap(),
            QjsOptionParse::Unknown
        ));
        assert_eq!(index, 0);

        let args = os_args(["--mount"]);
        let mut index = 0;
        let error = match parse_common_qjs_option(&args, &mut index, "qjs", &mut options) {
            Ok(_) => panic!("missing qjs option value should fail"),
            Err(error) => error,
        };
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("qjs --mount expects HOST=GUEST"));
    }

    fn parse_single_option(option: &str, value: &str) -> QjsRunOptions {
        parse_options([option, value])
    }

    fn parse_options<const N: usize>(args: [&str; N]) -> QjsRunOptions {
        let args = os_args(args);
        let mut options = QjsRunOptions::new().unwrap();
        let mut index = 0;
        while index < args.len() {
            assert!(matches!(
                parse_common_qjs_option(&args, &mut index, "qjs", &mut options).unwrap(),
                QjsOptionParse::Consumed
            ));
        }
        options
    }

    fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }
}
