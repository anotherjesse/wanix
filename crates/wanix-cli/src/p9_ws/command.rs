use std::ffi::OsString;
use std::path::PathBuf;

use crate::{CliError, command_args::named_arg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct P9WsCommand {
    pub(in crate::p9_ws) root_path: PathBuf,
    pub(in crate::p9_ws) addr: String,
    pub(in crate::p9_ws) once: bool,
}

pub(crate) fn parse_p9_ws_command(args: &[OsString]) -> Result<P9WsCommand, CliError> {
    let mut parts = P9WsCommandParts::default();
    let mut i = 0;
    while i < args.len() {
        let Some(option) = P9WsOption::from_arg(&args[i]) else {
            return Err(CliError::usage(format!(
                "unexpected p9-ws argument: {}",
                args[i].to_string_lossy()
            )));
        };
        match option.value_name() {
            Some(value_name) => {
                i += 1;
                let value = args.get(i).ok_or_else(|| {
                    CliError::usage(format!("{} expects {value_name}", option.label()))
                })?;
                parts.apply_value(option, value)?;
            }
            None => parts.apply_flag(option)?,
        }
        i += 1;
    }

    parts.finish()
}

#[derive(Default)]
struct P9WsCommandParts {
    root_path: Option<PathBuf>,
    addr: Option<String>,
    once: bool,
}

impl P9WsCommandParts {
    fn apply_value(&mut self, option: P9WsOption, value: &OsString) -> Result<(), CliError> {
        match option {
            P9WsOption::Root => {
                set_single_path(&mut self.root_path, value, "p9-ws accepts only one --root")
            }
            P9WsOption::Addr => {
                if self.addr.is_some() {
                    return Err(CliError::usage("p9-ws accepts only one --addr"));
                }
                self.addr = Some(value.to_string_lossy().into_owned());
                Ok(())
            }
            P9WsOption::Once => Ok(()),
        }
    }

    fn apply_flag(&mut self, option: P9WsOption) -> Result<(), CliError> {
        match option {
            P9WsOption::Once => {
                if self.once {
                    return Err(CliError::usage("p9-ws accepts only one --once"));
                }
                self.once = true;
            }
            P9WsOption::Root | P9WsOption::Addr => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<P9WsCommand, CliError> {
        let root_path = self
            .root_path
            .ok_or_else(|| CliError::usage("p9-ws requires --root DIR"))?;
        let addr = self
            .addr
            .ok_or_else(|| CliError::usage("p9-ws requires --addr HOST:PORT"))?;
        Ok(P9WsCommand {
            root_path,
            addr,
            once: self.once,
        })
    }
}

#[derive(Clone, Copy)]
enum P9WsOption {
    Root,
    Addr,
    Once,
}

impl P9WsOption {
    fn from_arg(arg: &OsString) -> Option<Self> {
        named_arg(arg, P9_WS_OPTIONS)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Root => "p9-ws --root",
            Self::Addr => "p9-ws --addr",
            Self::Once => "p9-ws --once",
        }
    }

    fn value_name(self) -> Option<&'static str> {
        match self {
            Self::Root => Some("DIR"),
            Self::Addr => Some("HOST:PORT"),
            Self::Once => None,
        }
    }
}

const P9_WS_OPTIONS: &[(&str, P9WsOption)] = &[
    ("--root", P9WsOption::Root),
    ("--addr", P9WsOption::Addr),
    ("--once", P9WsOption::Once),
];

fn set_single_path(
    target: &mut Option<PathBuf>,
    value: &OsString,
    duplicate_message: &str,
) -> Result<(), CliError> {
    if target.is_some() {
        return Err(CliError::usage(duplicate_message));
    }
    *target = Some(PathBuf::from(value));
    Ok(())
}
