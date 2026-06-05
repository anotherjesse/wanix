use std::ffi::OsString;
use std::path::PathBuf;

use wanix_fs::NormalizedPath;

use super::{QjsRestoreCommand, os_arg_to_string, parse_host_mount, validate_env_line};
use crate::CliError;

struct QjsRestoreOptions {
    cwd: NormalizedPath,
    mounts: Vec<super::HostMount>,
    before_args: Vec<String>,
    after_args: Vec<String>,
    before_env: Vec<String>,
    after_env: Vec<String>,
}

impl QjsRestoreOptions {
    fn new() -> Result<Self, CliError> {
        Ok(Self {
            cwd: NormalizedPath::new(".")?,
            mounts: Vec::new(),
            before_args: Vec::new(),
            after_args: Vec::new(),
            before_env: Vec::new(),
            after_env: Vec::new(),
        })
    }

    fn into_command(
        self,
        before_script_path: PathBuf,
        after_script_path: PathBuf,
    ) -> QjsRestoreCommand {
        QjsRestoreCommand {
            before_script_path,
            after_script_path,
            before_args: self.before_args,
            after_args: self.after_args,
            before_env: self.before_env,
            after_env: self.after_env,
            cwd: self.cwd,
            mounts: self.mounts,
        }
    }

    fn push_arg(&mut self, phase: RestorePhase, value: String) {
        match phase {
            RestorePhase::Before => self.before_args.push(value),
            RestorePhase::After => self.after_args.push(value),
        }
    }

    fn push_env(&mut self, phase: RestorePhase, value: String) {
        match phase {
            RestorePhase::Before => self.before_env.push(value),
            RestorePhase::After => self.after_env.push(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestoreOption {
    Cwd,
    BeforeEnv,
    AfterEnv,
    BeforeArg,
    AfterArg,
    Mount,
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestorePhase {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestoreOptionValue {
    Cwd,
    Env(RestorePhase),
    Arg(RestorePhase),
    Mount,
    Separator,
}

impl RestoreOption {
    fn from_arg(arg: &OsString) -> Option<Self> {
        match arg.to_str()? {
            "--cwd" => Some(Self::Cwd),
            "--before-env" => Some(Self::BeforeEnv),
            "--after-env" => Some(Self::AfterEnv),
            "--before-arg" => Some(Self::BeforeArg),
            "--after-arg" => Some(Self::AfterArg),
            "--mount" => Some(Self::Mount),
            "--" => Some(Self::Separator),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cwd => "qjs-restore --cwd",
            Self::BeforeEnv => "qjs-restore --before-env",
            Self::AfterEnv => "qjs-restore --after-env",
            Self::BeforeArg => "qjs-restore --before-arg",
            Self::AfterArg => "qjs-restore --after-arg",
            Self::Mount => "qjs-restore --mount",
            Self::Separator => "qjs-restore --",
        }
    }

    fn expected(self) -> &'static str {
        match self {
            Self::Cwd => "a Wanix path",
            Self::BeforeEnv | Self::AfterEnv => "KEY=VALUE",
            Self::BeforeArg | Self::AfterArg => "VALUE",
            Self::Mount => "HOST=GUEST",
            Self::Separator => "",
        }
    }

    fn value_kind(self) -> RestoreOptionValue {
        match self {
            Self::Cwd => RestoreOptionValue::Cwd,
            Self::BeforeEnv => RestoreOptionValue::Env(RestorePhase::Before),
            Self::AfterEnv => RestoreOptionValue::Env(RestorePhase::After),
            Self::BeforeArg => RestoreOptionValue::Arg(RestorePhase::Before),
            Self::AfterArg => RestoreOptionValue::Arg(RestorePhase::After),
            Self::Mount => RestoreOptionValue::Mount,
            Self::Separator => RestoreOptionValue::Separator,
        }
    }

    fn apply(self, value: &OsString, options: &mut QjsRestoreOptions) -> Result<(), CliError> {
        match self.value_kind() {
            RestoreOptionValue::Cwd => options.cwd = parse_restore_cwd(self, value)?,
            RestoreOptionValue::Env(phase) => {
                options.push_env(phase, parse_restore_env(self, value)?);
            }
            RestoreOptionValue::Arg(phase) => {
                options.push_arg(phase, os_arg_to_string(value, self.label())?);
            }
            RestoreOptionValue::Mount => {
                options.mounts.push(parse_restore_mount(self, value)?);
            }
            RestoreOptionValue::Separator => {}
        }
        Ok(())
    }
}

fn parse_restore_cwd(option: RestoreOption, value: &OsString) -> Result<NormalizedPath, CliError> {
    Ok(NormalizedPath::new(os_arg_to_string(
        value,
        option.label(),
    )?)?)
}

fn parse_restore_env(option: RestoreOption, value: &OsString) -> Result<String, CliError> {
    let value = os_arg_to_string(value, option.label())?;
    validate_env_line(&value, option.label())?;
    Ok(value)
}

fn parse_restore_mount(
    option: RestoreOption,
    value: &OsString,
) -> Result<super::HostMount, CliError> {
    parse_host_mount(&os_arg_to_string(value, option.label())?, option.label())
}

pub(crate) fn parse_qjs_restore_command(args: &[OsString]) -> Result<QjsRestoreCommand, CliError> {
    let mut options = QjsRestoreOptions::new()?;
    let mut i = 0;
    while i < args.len() {
        let Some(option) = RestoreOption::from_arg(&args[i]) else {
            break;
        };
        if option == RestoreOption::Separator {
            i += 1;
            break;
        }
        i += 1;
        let value = args.get(i).ok_or_else(|| {
            CliError::usage(format!("{} expects {}", option.label(), option.expected()))
        })?;
        option.apply(value, &mut options)?;
        i += 1;
    }

    let before_script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs-restore expects before.js and after.js"))?;
    let before_script_path = PathBuf::from(before_script);
    i += 1;

    let after_script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs-restore expects before.js and after.js"))?;
    let after_script_path = PathBuf::from(after_script);
    i += 1;

    if let Some(extra) = args.get(i) {
        return Err(CliError::usage(format!(
            "unexpected qjs-restore argument: {}",
            extra.to_string_lossy()
        )));
    }

    Ok(options.into_command(before_script_path, after_script_path))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::parse_qjs_restore_command;

    #[test]
    fn parse_qjs_restore_command_applies_options_and_scripts() {
        let command = parse_qjs_restore_command(&os_args([
            "--cwd",
            "work/dir",
            "--before-env",
            "BEFORE=one",
            "--after-env",
            "AFTER=two",
            "--before-arg",
            "pre",
            "--after-arg",
            "post",
            "--mount",
            "/host/data=guest/data",
            "before.js",
            "after.js",
        ]))
        .unwrap();

        assert_eq!(command.before_script_path, PathBuf::from("before.js"));
        assert_eq!(command.after_script_path, PathBuf::from("after.js"));
        assert_eq!(command.cwd.as_str(), "work/dir");
        assert_eq!(command.before_env, vec!["BEFORE=one"]);
        assert_eq!(command.after_env, vec!["AFTER=two"]);
        assert_eq!(command.before_args, vec!["pre"]);
        assert_eq!(command.after_args, vec!["post"]);
        assert_eq!(command.mounts.len(), 1);
        assert_eq!(command.mounts[0].host_path, PathBuf::from("/host/data"));
        assert_eq!(command.mounts[0].guest_path.as_str(), "guest/data");
    }

    #[test]
    fn parse_qjs_restore_command_separator_stops_option_parsing() {
        let command = parse_qjs_restore_command(&os_args([
            "--before-env",
            "BEFORE=one",
            "--",
            "--after-env",
            "after.js",
        ]))
        .unwrap();

        assert_eq!(command.before_script_path, PathBuf::from("--after-env"));
        assert_eq!(command.after_script_path, PathBuf::from("after.js"));
        assert_eq!(command.before_env, vec!["BEFORE=one"]);
        assert!(command.after_env.is_empty());
    }

    #[test]
    fn parse_qjs_restore_command_reports_usage_boundaries() {
        assert_usage_error(
            parse_qjs_restore_command(&[]),
            "qjs-restore expects before.js and after.js",
        );
        assert_usage_error(
            parse_qjs_restore_command(&os_args(["--before-env"])),
            "qjs-restore --before-env expects KEY=VALUE",
        );
        assert_usage_error(
            parse_qjs_restore_command(&os_args(["before.js"])),
            "qjs-restore expects before.js and after.js",
        );
        assert_usage_error(
            parse_qjs_restore_command(&os_args(["before.js", "after.js", "extra.js"])),
            "unexpected qjs-restore argument: extra.js",
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
