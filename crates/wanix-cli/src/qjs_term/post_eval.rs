use std::io::{Read, Write};
use std::path::Path;

use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

use super::command::PostEvalFeed;
use super::process::{
    run_process_line_feed_session_after_eval, run_process_raw_byte_feed_session_after_eval,
    split_feed_lines,
};
use super::{
    CliError, TermResize, TerminalPumpState, feed_terminal_batch_and_pump,
    feed_terminal_resize_and_pump, flush_terminal_feed_batch, task_exited,
};

pub(super) fn run_post_eval_feeds(
    feeds: Vec<PostEvalFeed>,
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let mut runner = PostEvalFeedRunner {
        process_stdin,
        terminal,
        terminal_id,
        runtime,
        pump_state,
        process_stdout,
        current_batch: Vec::new(),
    };
    runner.run(feeds)
}

struct PostEvalFeedRunner<'a> {
    process_stdin: &'a mut dyn Read,
    terminal: &'a TermDevice,
    terminal_id: &'a str,
    runtime: &'a mut QuickJsTaskRuntime,
    pump_state: TerminalPumpState,
    process_stdout: &'a mut dyn Write,
    current_batch: Vec<Vec<u8>>,
}

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
        if self.flush_batch_and_task_exited()? {
            return Ok(true);
        }
        run_process_line_feed_session_after_eval(
            self.process_stdin,
            self.terminal,
            self.terminal_id,
            self.runtime,
            &mut self.pump_state,
            self.process_stdout,
        )?;
        Ok(false)
    }

    fn run_raw_bytes_process(&mut self) -> Result<bool, CliError> {
        if self.flush_batch_and_task_exited()? {
            return Ok(true);
        }
        run_process_raw_byte_feed_session_after_eval(
            self.process_stdin,
            self.terminal,
            self.terminal_id,
            self.runtime,
            &mut self.pump_state,
            self.process_stdout,
        )?;
        Ok(false)
    }

    fn run_resize(&mut self, resize: TermResize) -> Result<bool, CliError> {
        if self.flush_batch_and_task_exited()? {
            return Ok(true);
        }
        let policy = self.pump_state.policy;
        feed_terminal_resize_and_pump(
            self.terminal,
            self.terminal_id,
            self.runtime,
            &resize,
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            self.process_stdout,
        )?;
        self.task_exited()
    }

    fn read_process_stdin(&mut self) -> Result<Vec<u8>, CliError> {
        let mut bytes = Vec::new();
        self.process_stdin
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
        let policy = self.pump_state.policy;
        flush_terminal_feed_batch(
            self.terminal,
            self.terminal_id,
            self.runtime,
            &mut self.current_batch,
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            self.process_stdout,
        )
    }

    fn feed_batch(&mut self, batch: &[Vec<u8>]) -> Result<(), CliError> {
        let policy = self.pump_state.policy;
        feed_terminal_batch_and_pump(
            self.terminal,
            self.terminal_id,
            self.runtime,
            batch,
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            self.process_stdout,
        )
    }

    fn task_exited(&self) -> Result<bool, CliError> {
        task_exited(self.runtime)
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
