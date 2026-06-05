use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_wasi_quickjs::QuickJsRuntime;
use wanix_fs::{FsError, FsResult};
use wanix_task::{Fd, Task};

use crate::host_api::take_buffer;
use crate::task_context::WanixExitState;

pub(super) type OutputBuffer = Arc<Mutex<Vec<u8>>>;

/// Result of running QuickJS source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl RunOutput {
    pub(super) fn empty() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    pub(super) fn new(stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self { stdout, stderr }
    }

    /// Returns captured stdout bytes.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns captured stderr bytes.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

pub(super) fn new_output_buffers() -> (OutputBuffer, OutputBuffer) {
    (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    )
}

pub(super) fn collect_run_output(
    runtime: &mut QuickJsRuntime,
    stdout: OutputBuffer,
    stderr: OutputBuffer,
) -> Result<RunOutput, RunFailure> {
    let mut stdout = take_buffer(stdout).map_err(|error| RunFailure {
        error,
        output: RunOutput::empty(),
    })?;
    stdout.extend_from_slice(&runtime.take_captured_stdout());
    let mut stderr = take_buffer(stderr).map_err(|error| RunFailure {
        error,
        output: RunOutput::empty(),
    })?;
    stderr.extend_from_slice(&runtime.take_captured_stderr());
    Ok(RunOutput::new(stdout, stderr))
}

#[derive(Debug)]
pub(super) struct RunFailure {
    pub(super) error: FsError,
    pub(super) output: RunOutput,
}

#[derive(Debug, Clone)]
pub(super) struct RunControl {
    pub(super) exit_state: Option<WanixExitState>,
    pub(super) output_task: Option<Task>,
    pub(super) event_loop_wait_budget: Duration,
    pub(super) ready_io_turns: usize,
    pub(super) interrupt_poll_budget: Option<usize>,
    pub(super) memory_limit_bytes: Option<u32>,
}

impl Default for RunControl {
    fn default() -> Self {
        Self {
            exit_state: None,
            output_task: None,
            event_loop_wait_budget: Duration::ZERO,
            ready_io_turns: 1,
            interrupt_poll_budget: None,
            memory_limit_bytes: None,
        }
    }
}

pub(super) fn write_task_output(task: &Task, output: &RunOutput) -> FsResult<()> {
    if !output.stdout.is_empty() {
        task.write_fd(Fd::STDOUT, &output.stdout)?;
    }
    if !output.stderr.is_empty() {
        task.write_fd(Fd::STDERR, &output.stderr)?;
    }
    Ok(())
}
