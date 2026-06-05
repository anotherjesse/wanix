use std::ffi::OsString;
use std::path::PathBuf;

use super::super::pump::TermResize;
use super::options::{QjsTermOption, QjsTermParseAction, value_name};
use super::{CliError, PostEvalFeed};
use crate::os_arg_to_string;

pub(super) struct ParsedQjsTermArgs {
    pub(super) qjs_args: Vec<OsString>,
    pub(super) feed_after_eval: Vec<PostEvalFeed>,
}

pub(super) struct QjsTermArgParser<'a> {
    args: &'a [OsString],
    index: usize,
    parsed: ParsedQjsTermArgs,
}

impl<'a> QjsTermArgParser<'a> {
    pub(super) fn new(args: &'a [OsString]) -> Self {
        Self {
            args,
            index: 0,
            parsed: ParsedQjsTermArgs {
                qjs_args: Vec::new(),
                feed_after_eval: Vec::new(),
            },
        }
    }

    pub(super) fn parse(mut self) -> Result<ParsedQjsTermArgs, CliError> {
        while let Some(action) = self.current_action() {
            if !self.apply_action(action)? {
                break;
            }
        }
        Ok(self.parsed)
    }

    fn current(&self) -> Option<&OsString> {
        self.args.get(self.index)
    }

    fn current_action(&self) -> Option<QjsTermParseAction> {
        QjsTermParseAction::from_arg(self.current()?)
    }

    fn apply_action(&mut self, action: QjsTermParseAction) -> Result<bool, CliError> {
        match action {
            QjsTermParseAction::TermOption(option) => self.apply_term_option(option)?,
            QjsTermParseAction::QjsValueOption => self.parse_qjs_value_option(),
            QjsTermParseAction::Rest => {
                self.parsed
                    .qjs_args
                    .extend_from_slice(&self.args[self.index..]);
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn apply_term_option(&mut self, option: QjsTermOption) -> Result<(), CliError> {
        match option {
            QjsTermOption::FeedBytes => self.parse_feed_bytes(),
            QjsTermOption::FeedFile => self.parse_feed_file(),
            QjsTermOption::FeedLines => self.parse_feed_lines(),
            QjsTermOption::Resize => self.parse_resize(),
        }
    }

    fn take_value(&mut self, label: &str) -> Result<&'a OsString, CliError> {
        self.index += 1;
        let value = self
            .args
            .get(self.index)
            .ok_or_else(|| CliError::usage(format!("{label} expects {}", value_name(label))))?;
        self.index += 1;
        Ok(value)
    }

    fn parse_feed_bytes(&mut self) -> Result<(), CliError> {
        let label = "qjs-term --feed-after-eval";
        let value = self.take_value(label)?;
        self.parsed.feed_after_eval.push(PostEvalFeed::Bytes(
            os_arg_to_string(value, label)?.into_bytes(),
        ));
        Ok(())
    }

    fn parse_feed_file(&mut self) -> Result<(), CliError> {
        let value = self.take_value("qjs-term --feed-after-eval-file")?;
        self.parsed.feed_after_eval.push(feed_path_or_process(
            value,
            PostEvalFeed::File,
            PostEvalFeed::Process,
        ));
        Ok(())
    }

    fn parse_feed_lines(&mut self) -> Result<(), CliError> {
        let value = self.take_value("qjs-term --feed-after-eval-lines")?;
        self.parsed.feed_after_eval.push(feed_path_or_process(
            value,
            PostEvalFeed::LinesFile,
            PostEvalFeed::LinesProcess,
        ));
        Ok(())
    }

    fn parse_resize(&mut self) -> Result<(), CliError> {
        let label = "qjs-term --resize-after-eval";
        let value = self.take_value(label)?;
        self.parsed
            .feed_after_eval
            .push(PostEvalFeed::Resize(parse_term_resize(value, label)?));
        Ok(())
    }

    fn parse_qjs_value_option(&mut self) {
        let arg = &self.args[self.index];
        self.parsed.qjs_args.push(arg.clone());
        self.index += 1;
        if let Some(value) = self.args.get(self.index) {
            self.parsed.qjs_args.push(value.clone());
            self.index += 1;
        }
    }
}

fn feed_path_or_process(
    value: &OsString,
    file: impl FnOnce(PathBuf) -> PostEvalFeed,
    process: PostEvalFeed,
) -> PostEvalFeed {
    if value == "-" {
        process
    } else {
        file(PathBuf::from(value))
    }
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
