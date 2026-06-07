//! The acceptor: node Y running a CPU job against the caller's exported world.
//!
//! [`run_job`] is the network generalization of the exact local launch pattern
//! (`allocate_root` → `task.bind(world, ".", ".")` → configure → `start`). The
//! difference is the *world*: instead of a local `MemFs`, the task is bound
//! against a [`wanix_9p_client::RemoteFs`] that proxies into the caller's
//! reverse-exported scoped namespace over the export stream. Compute travels to
//! the data.
//!
//! After `start` returns, the job's captured stdout/stderr and exit status are
//! delivered as a single batch of [`crate::CpuEvent`] frames on the control
//! stream. This is the honest v1 model: the task model runs the guest to
//! completion before its output is available, so output is batched, not
//! streamed. Incremental streaming is a named follow-up.
//!
//! # Who closes the export
//!
//! The acceptor does **not** half-close the export when [`run_job`] returns.
//! `TaskTable::allocate_task` binds the task's own `#task` filesystem back into
//! its namespace, so the table holds an `Arc` cycle (table → task → namespace →
//! `#task` → table) that no `Drop`/`Weak` breaks: dropping the local `table`
//! does not drop the namespace, the bound [`RemoteFs`], or its export stream.
//! The export is closed by the *caller* through the control protocol — it drains
//! the terminal [`crate::CpuEvent::Exit`] and then shuts its export-stream half
//! down (see [`crate::serve_export`] and the dialer). The acceptor only writes
//! the result batch and returns.

use std::io::Write;

use wanix_9p_client::RemoteFs;
use wanix_fs::FsResult;
use wanix_task::{Task, TaskTable};
use wanix_vfs::BindOptions;

use crate::CpuEvent;
use crate::error::{CpuError, CpuResult};
use crate::event::MAX_EVENT_PAYLOAD;
use crate::spec::CpuJobSpec;
use crate::stdio::CapturedStdio;
use crate::transport::Duplex;

/// Runs a CPU job: bind the exported world, start the task, batch the result.
///
/// `table` is consumed **by value**: it must already have the driver for
/// `spec.kind` registered (e.g. `qjs`, `wasm`, `noop`), and `wanix-cpu` never
/// depends on a concrete task runtime, so the caller of `run_job` owns driver
/// registration. Taking it owned gives each job its own fresh task table, the
/// right isolation boundary for a remote run.
///
/// Returning from `run_job` does **not** close the export. The table holds a
/// `#task` self-binding `Arc` cycle (no `Drop`/`Weak` breaks it), so dropping
/// the local `table` here drops neither the bound [`RemoteFs`] world nor its
/// export stream. The export is closed by the **caller** through the control
/// protocol: it reads the terminal [`CpuEvent::Exit`] this function writes, then
/// shuts down its own export-stream half, which is what makes the caller's
/// reverse [`serve_export`](crate::serve_export) read EOF and finish. The
/// acceptor's job is only to write the result batch.
///
/// `export` is the role-sorted export stream carrying the caller's reverse 9P
/// server; `control` is the role-sorted control stream the result batch is
/// written to.
///
/// The guest runs to completion inside `start`, then [`CpuEvent::Stdout`],
/// [`CpuEvent::Stderr`], and a terminal [`CpuEvent::Exit`] are written to
/// `control` in that order.
///
/// # Errors
///
/// Returns [`CpuError::Export`] when the export 9P session cannot be negotiated,
/// [`CpuError::Task`] when allocation/bind/configuration/start fails, and
/// [`CpuError::Control`] when writing the result batch to the control stream
/// fails. A guest non-zero exit is **not** an error: it is reported in the
/// [`CpuEvent::Exit`] frame, exactly like a local task's exit status.
pub fn run_job<C: Write>(
    table: TaskTable,
    spec: &CpuJobSpec,
    export: Box<dyn Duplex>,
    control: &mut C,
) -> CpuResult<()> {
    let batch = build_batch(&table, spec, export)?;
    // Writing the terminal Exit frame is the acceptor's last act. Dropping
    // `table` here does NOT half-close the export — the `#task` self-binding
    // cycle keeps the bound RemoteFs world alive — so the caller closes the
    // export itself after it reads this Exit (see the module doc).
    write_batch(control, &batch)
}

/// Runs the job to completion and assembles its result batch, dropping nothing.
fn build_batch(
    table: &TaskTable,
    spec: &CpuJobSpec,
    export: Box<dyn Duplex>,
) -> CpuResult<Vec<CpuEvent>> {
    let world = RemoteFs::connect(export).map_err(|err| CpuError::Export(err.to_string()))?;
    let task = configure_job(table, spec, world).map_err(|err| CpuError::Task(err.to_string()))?;
    let captured = CapturedStdio::attach(&task).map_err(|err| CpuError::Task(err.to_string()))?;

    // start runs the guest to completion; a driver error is reported as a
    // failing exit batch rather than aborting the control stream, so the caller
    // always observes a terminal Exit event.
    let start = table.start(task.id());
    result_batch(&task, &captured, start)
}

/// Allocates the task, binds the exported world at the root, and configures it.
fn configure_job(table: &TaskTable, spec: &CpuJobSpec, world: RemoteFs) -> FsResult<Task> {
    let task = table.allocate_root(&spec.kind)?;
    // The exported world is the task's filesystem root, exactly as a local
    // `task.bind(local_root, ".", ".")`. Every file the guest touches resolves
    // through the reverse 9P session back into the caller's scoped namespace.
    task.bind(std::sync::Arc::new(world), ".", ".", BindOptions::default())?;
    apply_spec(&task, spec)?;
    Ok(task)
}

/// Configures the task command, argv, env, and cwd from the job spec.
fn apply_spec(task: &Task, spec: &CpuJobSpec) -> FsResult<()> {
    let mut task_spec = wanix_task::TaskSpec::new(&spec.program)?;
    task_spec.args = spec.args.clone();
    task_spec.env = env_map(&spec.env);
    task_spec.cwd = spec.cwd.clone();
    task.set_spec(task_spec)?;
    // Mirror the observable `#task` fields the local launch sets, so a remote job
    // is inspectable through `#task` exactly like a local one.
    task.set_cmd(raw_cmd(&spec.program, &spec.args))?;
    task.set_env_lines(spec.env.join("\n"))?;
    task.set_dir(spec.cwd.to_string())?;
    Ok(())
}

/// Builds the captured-output result batch after the job has run.
fn result_batch(
    task: &Task,
    captured: &CapturedStdio,
    start: FsResult<()>,
) -> CpuResult<Vec<CpuEvent>> {
    let stdout = captured
        .stdout_bytes()
        .map_err(|err| CpuError::Task(err.to_string()))?;
    let mut stderr = captured
        .stderr_bytes()
        .map_err(|err| CpuError::Task(err.to_string()))?;
    let exit = match start {
        Ok(()) => parse_exit(&task.exit()),
        Err(err) => {
            append_error_line(&mut stderr, &err.to_string());
            // A driver failure that left no explicit exit is reported as 1.
            nonzero_exit(&task.exit())
        }
    };
    Ok(batch_events(stdout, stderr, exit))
}

/// Assembles the ordered event batch, chunking large stdout/stderr.
fn batch_events(stdout: Vec<u8>, stderr: Vec<u8>, exit: i32) -> Vec<CpuEvent> {
    let mut events = Vec::new();
    for chunk in stdout.chunks(MAX_EVENT_PAYLOAD) {
        events.push(CpuEvent::Stdout(chunk.to_vec()));
    }
    for chunk in stderr.chunks(MAX_EVENT_PAYLOAD) {
        events.push(CpuEvent::Stderr(chunk.to_vec()));
    }
    events.push(CpuEvent::Exit(exit));
    events
}

/// Writes the batch to the control stream in order.
fn write_batch<C: Write>(control: &mut C, batch: &[CpuEvent]) -> CpuResult<()> {
    for event in batch {
        event
            .write_to(control)
            .map_err(|err| CpuError::Control(err.to_string()))?;
    }
    Ok(())
}

/// Parses the task exit status, defaulting to 0.
fn parse_exit(exit: &str) -> i32 {
    exit.trim().parse().unwrap_or(0)
}

/// Parses the task exit status, defaulting a missing value to 1 (failure).
fn nonzero_exit(exit: &str) -> i32 {
    let parsed = exit.trim().parse().unwrap_or(1);
    if parsed == 0 { 1 } else { parsed }
}

/// Builds the raw `#task` command text from program and args.
fn raw_cmd(program: &str, args: &[String]) -> String {
    wanix_task::quote_cmd_argv(std::iter::once(program).chain(args.iter().map(String::as_str)))
}

/// Converts `KEY=value` lines into the spec environment map.
fn env_map(lines: &[String]) -> std::collections::BTreeMap<String, String> {
    lines
        .iter()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        })
        .collect()
}

/// Appends a trailing error line to captured stderr, ensuring a newline first.
fn append_error_line(stderr: &mut Vec<u8>, message: &str) {
    if !stderr.is_empty() && !stderr.ends_with(b"\n") {
        stderr.push(b'\n');
    }
    stderr.extend_from_slice(format!("wanix-cpu: {message}\n").as_bytes());
}
