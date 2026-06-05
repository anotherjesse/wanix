use std::io::{Read, Write};

use crate::QjsCommand;

use super::super::PostEvalFeed;
use super::super::program_spec::QjsTermProgram;
use super::super::pump::ProcessEventSources;

pub(in crate::qjs_term) struct QjsTermProgramRequest {
    pub(in crate::qjs_term) qjs_command: QjsCommand,
    pub(in crate::qjs_term) feed_after_eval: Vec<PostEvalFeed>,
    pub(in crate::qjs_term) program: QjsTermProgram,
    pub(in crate::qjs_term) event_sources: ProcessEventSources,
}

impl QjsTermProgramRequest {
    pub(in crate::qjs_term) fn blocking(
        qjs_command: QjsCommand,
        feed_after_eval: Vec<PostEvalFeed>,
        program: QjsTermProgram,
    ) -> Self {
        Self {
            qjs_command,
            feed_after_eval,
            program,
            event_sources: ProcessEventSources::blocking(),
        }
    }
}

pub(in crate::qjs_term) struct QjsTermProgramIo<'a> {
    pub(in crate::qjs_term) process_stdin: &'a mut dyn Read,
    pub(in crate::qjs_term) process_stdout: &'a mut dyn Write,
    pub(in crate::qjs_term) process_stderr: &'a mut dyn Write,
}
