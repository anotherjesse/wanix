use std::sync::Arc;
use std::time::Duration;

use wanix_fs::MemFs;
use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver};
use wanix_task::{Task, TaskTable};
use wanix_vfs::BindOptions;

use super::qjs_args::QjsRestoreCommand;
use super::{
    CliError, CliOutput, attach_task_stdio, bind_child_output_to_parent, bind_host_mounts,
    configure_qjs_task, copy_script_directory_into, ensure_snapshot_task_fds_closed,
    eval_qjs_source, guest_path_in_cwd, parse_exit, quickjs_runner, read_file, read_utf8_script,
};

const QJS_RESTORE_BEFORE_SCRIPT: &str = "__wanix_restore/before/main.js";
const QJS_RESTORE_AFTER_SCRIPT: &str = "__wanix_restore/after/main.js";
const QJS_RESTORE_BEFORE_DIR: &str = "__wanix_restore/before";
const QJS_RESTORE_AFTER_DIR: &str = "__wanix_restore/after";

pub(super) fn run_qjs_restore(command: QjsRestoreCommand) -> Result<CliOutput, CliError> {
    let workspace = prepare_qjs_restore_workspace(&command)?;
    let restore_result = run_qjs_restore_workspace(&workspace, &command);
    finish_qjs_restore_output(restore_result, &workspace.stdout, &workspace.stderr)
}

struct QjsRestoreWorkspace {
    runner: Arc<QuickJsRunner>,
    table: TaskTable,
    before_task: Task,
    stdout: Arc<MemFs>,
    stderr: Arc<MemFs>,
    before_script: String,
    after_script: String,
    before_guest_script: String,
    after_guest_script: String,
}

fn prepare_qjs_restore_workspace(
    command: &QjsRestoreCommand,
) -> Result<QjsRestoreWorkspace, CliError> {
    let before_script_path = command.before_script_path.as_path();
    let after_script_path = command.after_script_path.as_path();
    let before_script = read_utf8_script(before_script_path)?;
    let after_script = read_utf8_script(after_script_path)?;

    let runner = quickjs_runner()?;
    let table = TaskTable::new();
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(&runner))))?;
    let before_task = table.allocate_root("qjs")?;

    let root = Arc::new(MemFs::new());
    copy_script_directory_into(
        before_script_path,
        &root,
        &command.cwd,
        QJS_RESTORE_BEFORE_DIR,
    )?;
    copy_script_directory_into(
        after_script_path,
        &root,
        &command.cwd,
        QJS_RESTORE_AFTER_DIR,
    )?;
    let before_guest_script = guest_path_in_cwd(&command.cwd, QJS_RESTORE_BEFORE_SCRIPT)?;
    let after_guest_script = guest_path_in_cwd(&command.cwd, QJS_RESTORE_AFTER_SCRIPT)?;
    root.write_file(before_guest_script.as_str(), before_script.as_bytes())?;
    root.write_file(after_guest_script.as_str(), after_script.as_bytes())?;
    before_task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(&before_task, &command.mounts)?;

    let (stdout, stderr) = attach_task_stdio(&before_task, None)?;
    configure_qjs_task(
        &before_task,
        QJS_RESTORE_BEFORE_SCRIPT,
        &command.before_args,
        &command.before_env,
        &command.cwd,
    )?;

    Ok(QjsRestoreWorkspace {
        runner,
        table,
        before_task,
        stdout,
        stderr,
        before_script,
        after_script,
        before_guest_script,
        after_guest_script,
    })
}

fn run_qjs_restore_workspace(
    workspace: &QjsRestoreWorkspace,
    command: &QjsRestoreCommand,
) -> Result<i32, CliError> {
    let mut runtime = workspace
        .runner
        .create_task_runtime(&workspace.before_task)?;
    eval_qjs_source(
        &mut runtime,
        &workspace.before_script,
        &workspace.before_guest_script,
        Duration::ZERO,
        1,
    )?;
    ensure_snapshot_task_fds_closed(&workspace.before_task)?;
    let snapshot = runtime.snapshot_bytes()?;
    drop(runtime);

    let after_task = workspace
        .table
        .allocate_child_of("qjs", &workspace.before_task)?;
    configure_qjs_task(
        &after_task,
        QJS_RESTORE_AFTER_SCRIPT,
        &command.after_args,
        &command.after_env,
        &command.cwd,
    )?;
    bind_child_output_to_parent(&after_task, &workspace.before_task)?;

    let mut restored = workspace
        .runner
        .restore_task_runtime_from_bytes(&after_task, &snapshot)?;
    if let Err(error) = eval_qjs_source(
        &mut restored,
        &workspace.after_script,
        &workspace.after_guest_script,
        Duration::ZERO,
        1,
    ) {
        let _ = after_task.set_exit("1");
        return Err(error);
    }
    restored.finish()?;
    Ok(parse_exit(&after_task.exit()))
}

fn finish_qjs_restore_output(
    restore_result: Result<i32, CliError>,
    stdout: &Arc<MemFs>,
    stderr: &Arc<MemFs>,
) -> Result<CliOutput, CliError> {
    let stdout = read_file(stdout.as_ref(), "stdout")?;
    let mut stderr = read_file(stderr.as_ref(), "stderr")?;
    match restore_result {
        Ok(exit_code) => Ok(CliOutput::new(stdout, stderr, exit_code)),
        Err(error) => {
            if !stderr.is_empty() && !stderr.ends_with(b"\n") {
                stderr.push(b'\n');
            }
            stderr.extend_from_slice(format!("wanix-rust qjs-restore: {error}\n").as_bytes());
            Ok(CliOutput::new(stdout, stderr, 1))
        }
    }
}
