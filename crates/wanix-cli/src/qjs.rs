use std::io::Read;
use std::sync::Arc;

use wanix_qjs::QuickJsTaskDriver;
use wanix_task::TaskTable;

use crate::qjs::file_task::{
    allocate_qjs_task, build_qjs_driver, finish_qjs_start, prepare_qjs_file_task,
    read_qjs_file_script,
};
use crate::qjs::snapshot::{
    QjsResumeTaskRequest, read_qjs_resume_inputs, resume_qjs_file_task, snapshot_qjs_file_task,
};
use crate::qjs_args::{QjsCommand, QjsSnapshotFileCommand};
use crate::{CliError, CliOutput, finish_cli_task_output, quickjs_runner};

mod file_task;
mod snapshot;

pub(super) fn run_qjs(
    command: QjsCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let script = read_qjs_file_script(
        command.script_path.as_path(),
        command.stdin.clone(),
        process_stdin,
    )?;
    let table = TaskTable::new();
    let task = allocate_qjs_task(&table, build_qjs_driver(&command)?)?;
    let prepared = prepare_qjs_file_task(&task, &script, &command)?;

    let start_result = table.start(task.id());
    finish_qjs_start(start_result, &task, &prepared)
}

pub(super) fn run_qjs_snapshot(
    command: QjsSnapshotFileCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let script = read_qjs_file_script(
        command.script_path.as_path(),
        command.stdin.clone(),
        process_stdin,
    )?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    let task = allocate_qjs_task(&table, QuickJsTaskDriver::new(Arc::clone(&runner)))?;
    let prepared = prepare_qjs_file_task(&task, &script, &command)?;

    let snapshot_result =
        snapshot_qjs_file_task(&runner, &task, &script.source, &prepared, &command);

    finish_cli_task_output(
        "qjs-snapshot",
        snapshot_result,
        &task,
        &prepared.stdout,
        &prepared.stderr,
    )
}

pub(super) fn run_qjs_resume(
    command: QjsSnapshotFileCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let (script, snapshot) = read_qjs_resume_inputs(&command, process_stdin)?;
    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    let task = allocate_qjs_task(&table, QuickJsTaskDriver::new(Arc::clone(&runner)))?;
    let prepared = prepare_qjs_file_task(&task, &script, &command)?;
    let resume_result = resume_qjs_file_task(QjsResumeTaskRequest {
        runner: &runner,
        task: &task,
        snapshot: &snapshot,
        script: &script.source,
        prepared: &prepared,
        command: &command,
    });

    finish_cli_task_output(
        "qjs-resume",
        resume_result,
        &task,
        &prepared.stdout,
        &prepared.stderr,
    )
}
