use std::io::Read;

use wanix_task::Task;

use crate::qjs::file_task::{PreparedQjsFileTask, QjsFileScript};
use crate::qjs_args::{QjsSnapshotFileCommand, read_qjs_stdin};
use crate::{
    CliError, apply_qjs_task_runtime_limits, ensure_snapshot_task_fds_closed, eval_qjs_source,
    read_utf8_script,
};

pub(super) fn read_qjs_resume_inputs(
    command: &QjsSnapshotFileCommand,
    process_stdin: &mut dyn Read,
) -> Result<(QjsFileScript, Vec<u8>), CliError> {
    let source = read_utf8_script(command.script_path.as_path())?;
    let snapshot = read_snapshot(command)?;
    let stdin_bytes = read_qjs_stdin(command.stdin.clone(), process_stdin)?;
    Ok((
        QjsFileScript {
            path: command.script_path.clone(),
            source,
            stdin_bytes,
        },
        snapshot,
    ))
}

pub(super) fn snapshot_qjs_file_task(
    runner: &wanix_qjs::QuickJsRunner,
    task: &Task,
    script: &str,
    prepared: &PreparedQjsFileTask,
    command: &QjsSnapshotFileCommand,
) -> Result<(), CliError> {
    let mut runtime = runner.create_task_runtime(task)?;
    eval_snapshot_runtime(&mut runtime, task, script, prepared, command)?;
    finish_snapshot_runtime(runtime, command)
}

pub(super) struct QjsResumeTaskRequest<'a> {
    pub(super) runner: &'a wanix_qjs::QuickJsRunner,
    pub(super) task: &'a Task,
    pub(super) snapshot: &'a [u8],
    pub(super) script: &'a str,
    pub(super) prepared: &'a PreparedQjsFileTask,
    pub(super) command: &'a QjsSnapshotFileCommand,
}

fn eval_snapshot_runtime(
    runtime: &mut wanix_qjs::QuickJsTaskRuntime,
    task: &Task,
    script: &str,
    prepared: &PreparedQjsFileTask,
    command: &QjsSnapshotFileCommand,
) -> Result<(), CliError> {
    apply_qjs_task_runtime_limits(
        runtime,
        command.interrupt_poll_budget,
        command.memory_limit_bytes,
    )?;
    eval_qjs_source(
        runtime,
        script,
        &prepared.guest_script,
        command.event_loop_wait_budget,
        command.ready_io_turns,
    )?;
    ensure_snapshot_task_fds_closed(task)
}

pub(super) fn resume_qjs_file_task(request: QjsResumeTaskRequest<'_>) -> Result<(), CliError> {
    let mut runtime = request
        .runner
        .restore_task_runtime_from_bytes(request.task, request.snapshot)?;
    apply_qjs_task_runtime_limits(
        &mut runtime,
        request.command.interrupt_poll_budget,
        request.command.memory_limit_bytes,
    )?;
    eval_qjs_resume_source(
        &mut runtime,
        request.task,
        request.script,
        request.prepared,
        request.command,
    )?;
    Ok(runtime.finish()?)
}

fn finish_snapshot_runtime(
    mut runtime: wanix_qjs::QuickJsTaskRuntime,
    command: &QjsSnapshotFileCommand,
) -> Result<(), CliError> {
    write_snapshot(command, runtime.snapshot_bytes()?)?;
    Ok(runtime.finish()?)
}

fn eval_qjs_resume_source(
    runtime: &mut wanix_qjs::QuickJsTaskRuntime,
    task: &Task,
    script: &str,
    prepared: &PreparedQjsFileTask,
    command: &QjsSnapshotFileCommand,
) -> Result<(), CliError> {
    match eval_qjs_source(
        runtime,
        script,
        &prepared.guest_script,
        command.event_loop_wait_budget,
        command.ready_io_turns,
    ) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = task.set_exit("1");
            Err(error)
        }
    }
}

fn read_snapshot(command: &QjsSnapshotFileCommand) -> Result<Vec<u8>, CliError> {
    std::fs::read(&command.snapshot_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to read snapshot {}: {error}",
                command.snapshot_path.display()
            ),
            1,
        )
    })
}

fn write_snapshot(command: &QjsSnapshotFileCommand, snapshot: Vec<u8>) -> Result<(), CliError> {
    std::fs::write(&command.snapshot_path, snapshot).map_err(|error| {
        CliError::new(
            format!(
                "failed to write snapshot {}: {error}",
                command.snapshot_path.display()
            ),
            1,
        )
    })
}
