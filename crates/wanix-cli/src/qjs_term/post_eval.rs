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
    mut context: PostEvalFeedContext<'_>,
) -> Result<(), CliError> {
    let mut current_batch = Vec::new();
    for feed in feeds {
        if run_post_eval_feed(feed, &mut context, &mut current_batch)? {
            return Ok(());
        }
    }
    flush_batch(&mut context, &mut current_batch)
}

pub(super) struct PostEvalFeedContext<'a> {
    pub(super) process_stdin: &'a mut dyn Read,
    pub(super) terminal: &'a TermDevice,
    pub(super) terminal_id: &'a str,
    pub(super) runtime: &'a mut QuickJsTaskRuntime,
    pub(super) pump_state: TerminalPumpState,
    pub(super) process_stdout: &'a mut dyn Write,
}

type ProcessFeedSession = for<'a> fn(ProcessFeedContext<'a>) -> Result<(), CliError>;

fn run_post_eval_feed(
    feed: PostEvalFeed,
    context: &mut PostEvalFeedContext<'_>,
    current_batch: &mut Vec<Vec<u8>>,
) -> Result<bool, CliError> {
    match feed {
        PostEvalFeed::Bytes(bytes) => {
            current_batch.push(bytes);
            Ok(false)
        }
        PostEvalFeed::File(path) => {
            current_batch.push(read_post_eval_feed_file(&path, "post-eval feed file")?);
            Ok(false)
        }
        PostEvalFeed::Process => {
            current_batch.push(read_process_stdin(context.process_stdin)?);
            Ok(false)
        }
        PostEvalFeed::LinesFile(path) => run_lines_file(&path, context, current_batch),
        PostEvalFeed::LinesProcess => run_process_feed(
            run_process_line_feed_session_after_eval,
            context,
            current_batch,
        ),
        PostEvalFeed::RawBytesProcess => run_process_feed(
            run_process_raw_byte_feed_session_after_eval,
            context,
            current_batch,
        ),
        PostEvalFeed::Resize(resize) => run_resize(resize, context, current_batch),
    }
}

fn run_lines_file(
    path: &Path,
    context: &mut PostEvalFeedContext<'_>,
    current_batch: &mut Vec<Vec<u8>>,
) -> Result<bool, CliError> {
    if flush_batch_and_task_exited(context, current_batch)? {
        return Ok(true);
    }
    let bytes = read_post_eval_feed_file(path, "post-eval feed lines file")?;
    run_lines(split_feed_lines(bytes), context)
}

fn run_lines(lines: Vec<Vec<u8>>, context: &mut PostEvalFeedContext<'_>) -> Result<bool, CliError> {
    for line in lines {
        feed_batch(context, &[line])?;
        if task_exited(context.runtime)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn run_process_feed(
    session: ProcessFeedSession,
    context: &mut PostEvalFeedContext<'_>,
    current_batch: &mut Vec<Vec<u8>>,
) -> Result<bool, CliError> {
    if flush_batch_and_task_exited(context, current_batch)? {
        return Ok(true);
    }
    session(ProcessFeedContext {
        process_stdin: &mut *context.process_stdin,
        terminal: context.terminal,
        terminal_id: context.terminal_id,
        runtime: &mut *context.runtime,
        pump_state: &mut context.pump_state,
        process_stdout: &mut *context.process_stdout,
    })?;
    Ok(false)
}

fn run_resize(
    resize: TermResize,
    context: &mut PostEvalFeedContext<'_>,
    current_batch: &mut Vec<Vec<u8>>,
) -> Result<bool, CliError> {
    if flush_batch_and_task_exited(context, current_batch)? {
        return Ok(true);
    }
    let policy = context.pump_state.policy;
    let mut pump_context = pump_context(context);
    feed_terminal_resize_and_pump(&mut pump_context, &resize, policy)?;
    task_exited(context.runtime)
}

fn read_process_stdin(process_stdin: &mut dyn Read) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    process_stdin.read_to_end(&mut bytes).map_err(|error| {
        CliError::new(
            format!("failed to read process stdin after eval: {error}"),
            1,
        )
    })?;
    Ok(bytes)
}

fn flush_batch_and_task_exited(
    context: &mut PostEvalFeedContext<'_>,
    current_batch: &mut Vec<Vec<u8>>,
) -> Result<bool, CliError> {
    flush_batch(context, current_batch)?;
    task_exited(context.runtime)
}

fn flush_batch(
    context: &mut PostEvalFeedContext<'_>,
    current_batch: &mut Vec<Vec<u8>>,
) -> Result<(), CliError> {
    let policy = context.pump_state.policy;
    let mut pump_context = pump_context(context);
    flush_terminal_feed_batch(&mut pump_context, current_batch, policy)
}

fn feed_batch(context: &mut PostEvalFeedContext<'_>, batch: &[Vec<u8>]) -> Result<(), CliError> {
    let policy = context.pump_state.policy;
    let mut pump_context = pump_context(context);
    feed_terminal_batch_and_pump(&mut pump_context, batch, policy)
}

fn pump_context<'a>(context: &'a mut PostEvalFeedContext<'_>) -> TerminalPumpContext<'a> {
    TerminalPumpContext {
        terminal: context.terminal,
        terminal_id: context.terminal_id,
        runtime: &mut *context.runtime,
        process_stdout: &mut *context.process_stdout,
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
