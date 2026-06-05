use std::ffi::OsString;
use std::path::PathBuf;

use super::CliError;
use super::pump::TermResize;
use crate::{QjsCommand, parse_qjs_command_for};

mod options;
mod parser;
mod shell;

use parser::QjsTermArgParser;
pub(in crate::qjs_term) use shell::qjs_shell_command;
pub(crate) use shell::{QjsShellCommand, parse_qjs_shell_command};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QjsTermCommand {
    pub(super) qjs: QjsCommand,
    pub(super) feed_after_eval: Vec<PostEvalFeed>,
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
    let parsed = QjsTermArgParser::new(args).parse()?;
    Ok(QjsTermCommand {
        qjs: parse_qjs_command_for(&parsed.qjs_args, "qjs-term")?,
        feed_after_eval: parsed.feed_after_eval,
    })
}
