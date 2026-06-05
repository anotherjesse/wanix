use std::io::Write;

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

use crate::write_process_output;

use super::CliError;

mod events;

const TERMINAL_OUTPUT_READ_CHUNK_BYTES: usize = 1024;

pub(super) use events::{
    ProcessEventSources, ProcessInputMode, TermResize, TerminalPumpPolicy, TerminalPumpState,
};

pub(super) struct TerminalPumpContext<'a> {
    pub(super) terminal: &'a TermDevice,
    pub(super) terminal_id: &'a str,
    pub(super) runtime: &'a mut QuickJsTaskRuntime,
    pub(super) process_stdout: &'a mut dyn Write,
}

pub(super) fn flush_terminal_feed_batch(
    context: &mut TerminalPumpContext<'_>,
    batch: &mut Vec<Vec<u8>>,
    policy: TerminalPumpPolicy,
) -> Result<(), CliError> {
    if batch.is_empty() {
        return Ok(());
    }
    let flushed = std::mem::take(batch);
    feed_terminal_batch_and_pump(context, &flushed, policy)
}

pub(super) fn feed_terminal_batch_and_pump(
    context: &mut TerminalPumpContext<'_>,
    batch: &[Vec<u8>],
    policy: TerminalPumpPolicy,
) -> Result<(), CliError> {
    run_and_drain_terminal_output(context, |context| {
        for chunk in batch {
            feed_terminal_after_eval(context.terminal, context.terminal_id, chunk)?;
        }
        context
            .runtime
            .run_event_loop_turns(policy.event_loop_wait_budget, policy.ready_io_turns)?;
        Ok(())
    })
}

pub(super) fn feed_terminal_chunk_and_pump(
    context: &mut TerminalPumpContext<'_>,
    chunk: &[u8],
    policy: TerminalPumpPolicy,
) -> Result<(), CliError> {
    run_and_drain_terminal_output(context, |context| {
        feed_terminal_after_eval(context.terminal, context.terminal_id, chunk)?;
        context
            .runtime
            .run_event_loop_turns(policy.event_loop_wait_budget, policy.ready_io_turns)?;
        Ok(())
    })
}

pub(super) fn pump_terminal_idle(
    context: &mut TerminalPumpContext<'_>,
    policy: TerminalPumpPolicy,
) -> Result<(), CliError> {
    run_and_drain_terminal_output(context, |context| {
        context
            .runtime
            .run_event_loop_turns(policy.event_loop_wait_budget, policy.ready_io_turns)
            .map_err(CliError::from)
    })
}

pub(super) fn pump_terminal_resize_if_changed(
    context: &mut TerminalPumpContext<'_>,
    pump_state: &mut TerminalPumpState,
) -> Result<(), CliError> {
    let Some(resize) = pump_state.resize_source.next_resize()? else {
        return Ok(());
    };
    feed_terminal_resize_and_pump(context, &resize, pump_state.policy)
}

pub(super) fn feed_terminal_resize_and_pump(
    context: &mut TerminalPumpContext<'_>,
    resize: &TermResize,
    policy: TerminalPumpPolicy,
) -> Result<(), CliError> {
    run_and_drain_terminal_output(context, |context| {
        feed_terminal_resize_after_eval(context.terminal, context.terminal_id, resize)?;
        context
            .runtime
            .run_event_loop_turns(policy.event_loop_wait_budget, policy.ready_io_turns)?;
        Ok(())
    })
}

fn run_and_drain_terminal_output(
    context: &mut TerminalPumpContext<'_>,
    run: impl FnOnce(&mut TerminalPumpContext<'_>) -> Result<(), CliError>,
) -> Result<(), CliError> {
    let result = run(context);
    drain_terminal_output(
        context.terminal,
        context.terminal_id,
        context.process_stdout,
    )?;
    result
}

pub(super) fn task_exited(runtime: &QuickJsTaskRuntime) -> Result<bool, CliError> {
    Ok(runtime.exit_code()?.is_some())
}

pub(super) fn feed_terminal_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    chunk: &[u8],
) -> Result<(), CliError> {
    if chunk.is_empty() {
        return Ok(());
    }
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    data.write(chunk)?;
    Ok(())
}

pub(super) fn feed_terminal_resize_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    resize: &TermResize,
) -> Result<(), CliError> {
    let mut winch = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/winch"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    winch.write(&terminal_resize_payload(resize))?;
    Ok(())
}

fn terminal_resize_payload(resize: &TermResize) -> Vec<u8> {
    format!("{} {}\n", resize.columns, resize.rows).into_bytes()
}

pub(super) fn drain_terminal_output(
    terminal: &TermDevice,
    terminal_id: &str,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    write_process_output(
        process_stdout,
        "stdout",
        &drain_terminal_output_bytes(terminal, terminal_id)?,
    )
}

pub(super) fn drain_terminal_output_bytes(
    terminal: &TermDevice,
    terminal_id: &str,
) -> Result<Vec<u8>, CliError> {
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            read: true,
            ..OpenOptions::default()
        },
    )?;
    let mut output = Vec::new();
    let mut buf = [0; TERMINAL_OUTPUT_READ_CHUNK_BYTES];
    loop {
        let count = data.read(&mut buf)?;
        if count == 0 {
            return Ok(output);
        }
        output.extend_from_slice(&buf[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::{TermResize, terminal_resize_payload};

    #[test]
    fn terminal_resize_payload_matches_winch_contract() {
        let resize = TermResize {
            columns: 132,
            rows: 43,
        };

        assert_eq!(terminal_resize_payload(&resize), b"132 43\n");
    }
}
