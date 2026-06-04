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

    fn apply(self, value: &OsString, options: &mut QjsRestoreOptions) -> Result<(), CliError> {
        match self {
            Self::Cwd => {
                options.cwd = NormalizedPath::new(os_arg_to_string(value, self.label())?)?;
            }
            Self::BeforeEnv => {
                let value = os_arg_to_string(value, self.label())?;
                validate_env_line(&value, self.label())?;
                options.before_env.push(value);
            }
            Self::AfterEnv => {
                let value = os_arg_to_string(value, self.label())?;
                validate_env_line(&value, self.label())?;
                options.after_env.push(value);
            }
            Self::BeforeArg => options
                .before_args
                .push(os_arg_to_string(value, self.label())?),
            Self::AfterArg => options
                .after_args
                .push(os_arg_to_string(value, self.label())?),
            Self::Mount => options.mounts.push(parse_host_mount(
                &os_arg_to_string(value, self.label())?,
                self.label(),
            )?),
            Self::Separator => {}
        }
        Ok(())
    }
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
