use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use wanix_fs::{MemFs, NormalizedPath};
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::{Task, TaskTable};
use wanix_vfs::BindOptions;

use crate::qjs_args::{HostMount, QjsCommand, QjsSnapshotFileCommand, QjsStdin, read_qjs_stdin};
use crate::{
    CliError, CliOutput, QJS_GUEST_SCRIPT, attach_task_stdio, bind_host_mounts, configure_qjs_task,
    copy_script_directory, guest_path_in_cwd, parse_exit, quickjs_runner, read_file,
    read_utf8_script,
};

pub(super) struct QjsFileScript {
    pub(super) path: std::path::PathBuf,
    pub(super) source: String,
    pub(super) stdin_bytes: Option<Vec<u8>>,
}

struct PreparedQjsFileRoot {
    root: Arc<MemFs>,
    guest_script: String,
}

pub(super) struct PreparedQjsFileTask {
    pub(super) stdout: Arc<MemFs>,
    pub(super) stderr: Arc<MemFs>,
    pub(super) guest_script: String,
}

pub(super) trait QjsFileCommand {
    fn args(&self) -> &[String];
    fn env(&self) -> &[String];
    fn cwd(&self) -> &NormalizedPath;
    fn mounts(&self) -> &[HostMount];
}

impl QjsFileCommand for QjsCommand {
    fn args(&self) -> &[String] {
        &self.args
    }

    fn env(&self) -> &[String] {
        &self.env
    }

    fn cwd(&self) -> &NormalizedPath {
        &self.cwd
    }

    fn mounts(&self) -> &[HostMount] {
        &self.mounts
    }
}

impl QjsFileCommand for QjsSnapshotFileCommand {
    fn args(&self) -> &[String] {
        &self.args
    }

    fn env(&self) -> &[String] {
        &self.env
    }

    fn cwd(&self) -> &NormalizedPath {
        &self.cwd
    }

    fn mounts(&self) -> &[HostMount] {
        &self.mounts
    }
}

pub(super) fn read_qjs_file_script(
    script_path: &Path,
    stdin: Option<QjsStdin>,
    process_stdin: &mut dyn Read,
) -> Result<QjsFileScript, CliError> {
    Ok(QjsFileScript {
        path: script_path.to_path_buf(),
        source: read_utf8_script(script_path)?,
        stdin_bytes: read_qjs_stdin(stdin, process_stdin)?,
    })
}

pub(super) fn build_qjs_driver(command: &QjsCommand) -> Result<QuickJsTaskDriver, CliError> {
    Ok(apply_qjs_driver_limits(
        QuickJsTaskDriver::new(quickjs_runner()?)
            .with_event_loop_wait_budget(command.event_loop_wait_budget)
            .with_ready_io_turns(command.ready_io_turns),
        command,
    ))
}

fn apply_qjs_driver_limits(
    mut driver: QuickJsTaskDriver,
    command: &QjsCommand,
) -> QuickJsTaskDriver {
    if let Some(budget) = command.interrupt_poll_budget {
        driver = driver.with_interrupt_poll_budget(budget);
    }
    if let Some(bytes) = command.memory_limit_bytes {
        driver = driver.with_memory_limit_bytes(bytes);
    }
    driver
}

pub(super) fn allocate_qjs_task(
    table: &TaskTable,
    driver: QuickJsTaskDriver,
) -> Result<Task, CliError> {
    table.register_driver("qjs", Arc::new(driver))?;
    table.allocate_root("qjs").map_err(CliError::from)
}

pub(super) fn prepare_qjs_file_task(
    task: &Task,
    script: &QjsFileScript,
    command: &impl QjsFileCommand,
) -> Result<PreparedQjsFileTask, CliError> {
    let root = prepare_qjs_file_root(script.path.as_path(), &script.source, command.cwd())?;
    bind_qjs_file_namespace(task, root.root, command.mounts())?;
    let (stdout, stderr) = configure_qjs_file_task(task, script, command)?;
    Ok(PreparedQjsFileTask {
        stdout,
        stderr,
        guest_script: root.guest_script,
    })
}

fn prepare_qjs_file_root(
    script_path: &Path,
    source: &str,
    cwd: &NormalizedPath,
) -> Result<PreparedQjsFileRoot, CliError> {
    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root, cwd)?;
    let guest_script = guest_path_in_cwd(cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), source.as_bytes())?;
    Ok(PreparedQjsFileRoot { root, guest_script })
}

fn bind_qjs_file_namespace(
    task: &Task,
    root: Arc<MemFs>,
    mounts: &[HostMount],
) -> Result<(), CliError> {
    task.bind(root, ".", ".", BindOptions::default())?;
    bind_host_mounts(task, mounts)
}

fn configure_qjs_file_task(
    task: &Task,
    script: &QjsFileScript,
    command: &impl QjsFileCommand,
) -> Result<(Arc<MemFs>, Arc<MemFs>), CliError> {
    let stdio = attach_task_stdio(task, script.stdin_bytes.clone())?;
    configure_qjs_task(
        task,
        QJS_GUEST_SCRIPT,
        command.args(),
        command.env(),
        command.cwd(),
    )?;
    Ok(stdio)
}

pub(super) fn finish_qjs_start<E: std::fmt::Display>(
    result: Result<(), E>,
    task: &Task,
    prepared: &PreparedQjsFileTask,
) -> Result<CliOutput, CliError> {
    let stdout = read_file(&*prepared.stdout, "stdout")?;
    let mut stderr = read_file(&*prepared.stderr, "stderr")?;
    match result {
        Ok(()) => Ok(CliOutput::new(stdout, stderr, parse_exit(&task.exit()))),
        Err(error) => {
            append_qjs_error(&mut stderr, "qjs", &error.to_string());
            Ok(CliOutput::new(stdout, stderr, 1))
        }
    }
}

fn append_qjs_error(stderr: &mut Vec<u8>, command: &str, error: &str) {
    if !stderr.is_empty() && !stderr.ends_with(b"\n") {
        stderr.push(b'\n');
    }
    stderr.extend_from_slice(format!("wanix {command}: {error}\n").as_bytes());
}
