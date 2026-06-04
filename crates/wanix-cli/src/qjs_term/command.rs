use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::pump::TermResize;
use super::{CliError, QJS_SHELL_READY_IO_TURNS, QJS_SHELL_SCRIPT_SENTINEL};
use crate::{QjsCommand, os_arg_to_string, parse_qjs_command_for};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QjsTermCommand {
    pub(super) qjs: QjsCommand,
    pub(super) feed_after_eval: Vec<PostEvalFeed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QjsShellCommand {
    pub(super) qjs: QjsCommand,
    pub(super) raw: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PostEvalFeed {
    Bytes(Vec<u8>),
    File(PathBuf),
    Process,
    LinesFile(PathBuf),
    LinesProcess,
    RawBytesProcess,
    Resize(TermResize),
}

pub(crate) fn parse_qjs_term_command(args: &[OsString]) -> Result<QjsTermCommand, CliError> {
    let mut qjs_args = Vec::new();
    let mut feed_after_eval = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--feed-after-eval" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-term --feed-after-eval expects text"))?;
            feed_after_eval.push(PostEvalFeed::Bytes(
                os_arg_to_string(value, "qjs-term --feed-after-eval")?.into_bytes(),
            ));
            i += 1;
        } else if args[i] == "--feed-after-eval-file" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage("qjs-term --feed-after-eval-file expects PATH or -")
            })?;
            if value == "-" {
                feed_after_eval.push(PostEvalFeed::Process);
            } else {
                feed_after_eval.push(PostEvalFeed::File(PathBuf::from(value)));
            }
            i += 1;
        } else if args[i] == "--feed-after-eval-lines" {
            i += 1;
            let value = args.get(i).ok_or_else(|| {
                CliError::usage("qjs-term --feed-after-eval-lines expects PATH or -")
            })?;
            if value == "-" {
                feed_after_eval.push(PostEvalFeed::LinesProcess);
            } else {
                feed_after_eval.push(PostEvalFeed::LinesFile(PathBuf::from(value)));
            }
            i += 1;
        } else if args[i] == "--resize-after-eval" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs-term --resize-after-eval expects COLSxROWS"))?;
            feed_after_eval.push(PostEvalFeed::Resize(parse_term_resize(
                value,
                "qjs-term --resize-after-eval",
            )?));
            i += 1;
        } else if args[i] == "--" {
            qjs_args.extend_from_slice(&args[i..]);
            break;
        } else if qjs_option_takes_value(&args[i]) {
            qjs_args.push(args[i].clone());
            i += 1;
            if let Some(value) = args.get(i) {
                qjs_args.push(value.clone());
                i += 1;
            }
        } else {
            qjs_args.extend_from_slice(&args[i..]);
            break;
        }
    }
    Ok(QjsTermCommand {
        qjs: parse_qjs_command_for(&qjs_args, "qjs-term")?,
        feed_after_eval,
    })
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

fn qjs_option_takes_value(arg: &OsString) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "--env"
                | "--cwd"
                | "--stdin"
                | "--stdin-file"
                | "--event-loop-ms"
                | "--ready-io-turns"
                | "--interrupt-after"
                | "--memory-limit-bytes"
                | "--mount"
        )
    )
}

fn parse_term_resize(arg: &OsString, label: &str) -> Result<TermResize, CliError> {
    let value = os_arg_to_string(arg, label)?;
    let Some((columns, rows)) = value.split_once('x').or_else(|| value.split_once('X')) else {
        return Err(CliError::usage(format!("{label} expects COLSxROWS")));
    };
    let columns = parse_positive_u16(columns, &format!("{label} columns"))?;
    let rows = parse_positive_u16(rows, &format!("{label} rows"))?;
    Ok(TermResize { columns, rows })
}

fn parse_positive_u16(value: &str, label: &str) -> Result<u16, CliError> {
    let number = value
        .parse::<u16>()
        .map_err(|_| CliError::usage(format!("{label} expects an integer from 1 to 65535")))?;
    if number == 0 {
        return Err(CliError::usage(format!(
            "{label} expects an integer from 1 to 65535"
        )));
    }
    Ok(number)
}

pub(super) fn qjs_shell_command(mut command: QjsCommand, raw: bool) -> QjsCommand {
    command.ready_io_turns = command.ready_io_turns.max(QJS_SHELL_READY_IO_TURNS);
    if raw {
        command.env.push("WANIX_QJS_SHELL_RAW=1".to_owned());
    }
    command
}
