use std::ffi::OsString;
use std::path::Path;

use super::{CliError, QJS_SHELL_READY_IO_TURNS, QJS_SHELL_SCRIPT_SENTINEL};
use crate::{QjsCommand, parse_qjs_command_for};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QjsShellCommand {
    pub(in crate::qjs_term) qjs: QjsCommand,
    pub(in crate::qjs_term) raw: bool,
}

pub(crate) fn parse_qjs_shell_command(args: &[OsString]) -> Result<QjsShellCommand, CliError> {
    let mut qjs_args = Vec::new();
    let mut raw = false;
    for arg in args {
        if arg == "--raw" {
            raw = true;
        } else {
            qjs_args.push(arg.clone());
        }
    }
    qjs_args.push(OsString::from(QJS_SHELL_SCRIPT_SENTINEL));
    let qjs = parse_qjs_command_for(&qjs_args, "qjs-shell")?;
    if qjs.script_path != Path::new(QJS_SHELL_SCRIPT_SENTINEL) || !qjs.args.is_empty() {
        return Err(CliError::usage(
            "qjs-shell does not accept a script path or script arguments",
        ));
    }
    if qjs.stdin.is_some() {
        return Err(CliError::usage(
            "qjs-shell reads native stdin as terminal input; use qjs-term for explicit stdin fixtures",
        ));
    }
    Ok(QjsShellCommand { qjs, raw })
}

pub(in crate::qjs_term) fn qjs_shell_command(mut command: QjsCommand, raw: bool) -> QjsCommand {
    command.ready_io_turns = command.ready_io_turns.max(QJS_SHELL_READY_IO_TURNS);
    if raw {
        command.env.push("WANIX_QJS_SHELL_RAW=1".to_owned());
    }
    command
}
