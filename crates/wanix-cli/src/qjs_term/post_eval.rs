use std::io::{Read, Write};
use std::path::Path;

use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

use super::CliError;
use super::command::PostEvalFeed;
use super::process::{
    ProcessFeedContext, run_process_line_feed_session_after_eval,
    run_process_raw_byte_feed_session_after_eval, split_feed_lines,
};
use super::pump::{
    TermResize, TerminalPumpContext, TerminalPumpState, feed_terminal_batch_and_pump,
    feed_terminal_resize_and_pump, flush_terminal_feed_batch, task_exited,
};

pub(super) fn run_post_eval_feeds(
    feeds: Vec<PostEvalFeed>,
    context: PostEvalFeedContext<'_>,
) -> Result<(), CliError> {
    let mut runner = PostEvalFeedRunner {
        context,
        current_batch: Vec::new(),
    };
    runner.run(feeds)
}

pub(super) struct PostEvalFeedContext<'a> {
    pub(super) process_stdin: &'a mut dyn Read,
    pub(super) terminal: &'a TermDevice,
    pub(super) terminal_id: &'a str,
    pub(super) runtime: &'a mut QuickJsTaskRuntime,
    pub(super) pump_state: TerminalPumpState,
    pub(super) process_stdout: &'a mut dyn Write,
}

struct PostEvalFeedRunner<'a> {
    context: PostEvalFeedContext<'a>,
    current_batch: Vec<Vec<u8>>,
}

type ProcessFeedSession = for<'a> fn(ProcessFeedContext<'a>) -> Result<(), CliError>;

impl PostEvalFeedRunner<'_> {
    fn run(&mut self, feeds: Vec<PostEvalFeed>) -> Result<(), CliError> {
        for feed in feeds {
            if self.run_feed(feed)? {
                return Ok(());
            }
        }
        self.flush_batch()
    }

    fn run_feed(&mut self, feed: PostEvalFeed) -> Result<bool, CliError> {
        match feed {
            PostEvalFeed::Bytes(bytes) => {
                self.current_batch.push(bytes);
                Ok(false)
            }
            PostEvalFeed::File(path) => {
                self.current_batch
                    .push(read_post_eval_feed_file(&path, "post-eval feed file")?);
                Ok(false)
            }
            PostEvalFeed::Process => {
                let bytes = self.read_process_stdin()?;
                self.current_batch.push(bytes);
                Ok(false)
            }
            PostEvalFeed::LinesFile(path) => self.run_lines_file(&path),
            PostEvalFeed::LinesProcess => self.run_lines_process(),
            PostEvalFeed::RawBytesProcess => self.run_raw_bytes_process(),
            PostEvalFeed::Resize(resize) => self.run_resize(resize),
        }
    }

    fn run_lines_file(&mut self, path: &Path) -> Result<bool, CliError> {
        if self.flush_batch_and_task_exited()? {
            return Ok(true);
        }
        let bytes = read_post_eval_feed_file(path, "post-eval feed lines file")?;
        self.run_lines(split_feed_lines(bytes))
    }

    fn run_lines(&mut self, lines: Vec<Vec<u8>>) -> Result<bool, CliError> {
        for line in lines {
            self.feed_batch(&[line])?;
            if self.task_exited()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn run_lines_process(&mut self) -> Result<bool, CliError> {
        self.run_process_feed(run_process_line_feed_session_after_eval)
    }

    fn run_raw_bytes_process(&mut self) -> Result<bool, CliError> {
        self.run_process_feed(run_process_raw_byte_feed_session_after_eval)
    }

    fn run_process_feed(&mut self, session: ProcessFeedSession) -> Result<bool, CliError> {
        if self.flush_batch_and_task_exited()? {
            return Ok(true);
        }
        session(ProcessFeedContext {
            process_stdin: &mut *self.context.process_stdin,
            terminal: self.context.terminal,
            terminal_id: self.context.terminal_id,
            runtime: &mut *self.context.runtime,
            pump_state: &mut self.context.pump_state,
            process_stdout: &mut *self.context.process_stdout,
        })?;
        Ok(false)
    }

    fn run_resize(&mut self, resize: TermResize) -> Result<bool, CliError> {
        if self.flush_batch_and_task_exited()? {
            return Ok(true);
        }
        let policy = self.context.pump_state.policy;
        let mut pump_context = self.pump_context();
        feed_terminal_resize_and_pump(&mut pump_context, &resize, policy)?;
        self.task_exited()
    }

    fn read_process_stdin(&mut self) -> Result<Vec<u8>, CliError> {
        let mut bytes = Vec::new();
        self.context
            .process_stdin
            .read_to_end(&mut bytes)
            .map_err(|error| {
                CliError::new(
                    format!("failed to read process stdin after eval: {error}"),
                    1,
                )
            })?;
        Ok(bytes)
    }

    fn flush_batch_and_task_exited(&mut self) -> Result<bool, CliError> {
        self.flush_batch()?;
        self.task_exited()
    }

    fn flush_batch(&mut self) -> Result<(), CliError> {
        let policy = self.context.pump_state.policy;
        let mut pump_context = TerminalPumpContext {
            terminal: self.context.terminal,
            terminal_id: self.context.terminal_id,
            runtime: &mut *self.context.runtime,
            process_stdout: &mut *self.context.process_stdout,
        };
        flush_terminal_feed_batch(&mut pump_context, &mut self.current_batch, policy)
    }

    fn feed_batch(&mut self, batch: &[Vec<u8>]) -> Result<(), CliError> {
        let policy = self.context.pump_state.policy;
        let mut pump_context = self.pump_context();
        feed_terminal_batch_and_pump(&mut pump_context, batch, policy)
    }

    fn task_exited(&self) -> Result<bool, CliError> {
        task_exited(self.context.runtime)
    }

    fn pump_context(&mut self) -> TerminalPumpContext<'_> {
        TerminalPumpContext {
            terminal: self.context.terminal,
            terminal_id: self.context.terminal_id,
            runtime: &mut *self.context.runtime,
            process_stdout: &mut *self.context.process_stdout,
        }
    }
}

fn read_post_eval_feed_file(path: &Path, description: &str) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|error| {
        CliError::new(
            format!("failed to read {description} {}: {error}", path.display()),
            1,
        )
    })
}
