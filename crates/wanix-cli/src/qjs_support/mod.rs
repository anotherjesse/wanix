use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::OnceLock;
use std::time::Duration;

use wanix_fs::{FileSystem, FsError, LocalFs, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::{QuickJsRunner, QuickJsTaskRuntime};
use wanix_task::{Fd, Task};
use wanix_vfs::BindOptions;

use crate::qjs_args::HostMount;
use crate::{CliError, CliOutput};

mod script;
mod task;

pub(crate) use script::{
    copy_script_directory, copy_script_directory_into, guest_path_in_cwd, read_file,
    read_utf8_script,
};
pub(crate) use task::{bind_child_output_to_parent, configure_qjs_task};

pub(crate) const QJS_GUEST_SCRIPT: &str = "main.js";

pub(crate) fn attach_task_stdio(
    task: &Task,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<(Arc<MemFs>, Arc<MemFs>), CliError> {
    attach_task_stdin(task, stdin_bytes)?;
    let stdout = attach_output_file(task, Fd::STDOUT, "stdout")?;
    let stderr = attach_output_file(task, Fd::STDERR, "stderr")?;
    Ok((stdout, stderr))
}

fn attach_task_stdin(task: &Task, stdin_bytes: Option<Vec<u8>>) -> Result<(), CliError> {
    let Some(stdin_bytes) = stdin_bytes else {
        return Ok(());
    };
    let stdin = mem_file("stdin", stdin_bytes)?;
    insert_task_mem_file(task, Fd::STDIN, &stdin, "stdin", OpenOptions::read())
}

fn attach_output_file(task: &Task, fd: Fd, name: &str) -> Result<Arc<MemFs>, CliError> {
    let fs = mem_file(name, b"")?;
    insert_task_mem_file(task, fd, &fs, name, OpenOptions::read_write())?;
    Ok(fs)
}

fn mem_file(name: &str, bytes: impl AsRef<[u8]>) -> Result<Arc<MemFs>, CliError> {
    let fs = Arc::new(MemFs::new());
    fs.write_file(name, bytes.as_ref())?;
    Ok(fs)
}

fn insert_task_mem_file(
    task: &Task,
    fd: Fd,
    fs: &Arc<MemFs>,
    path: &str,
    options: OpenOptions,
) -> Result<(), CliError> {
    let path = NormalizedPath::new(path)?;
    task.insert_fd(fd, fs.open(&path, options)?, path)?;
    Ok(())
}

pub(crate) fn finish_cli_task_output(
    command: &str,
    result: Result<(), CliError>,
    task: &Task,
    stdout: &Arc<MemFs>,
    stderr: &Arc<MemFs>,
) -> Result<CliOutput, CliError> {
    let (stdout, stderr) = read_task_output(stdout, stderr)?;
    Ok(match result {
        Ok(()) => CliOutput::new(stdout, stderr, parse_exit(&task.exit())),
        Err(error) => error_task_output(command, error, stdout, stderr),
    })
}

fn read_task_output(
    stdout: &Arc<MemFs>,
    stderr: &Arc<MemFs>,
) -> Result<(Vec<u8>, Vec<u8>), CliError> {
    Ok((
        read_file(stdout.as_ref(), "stdout")?,
        read_file(stderr.as_ref(), "stderr")?,
    ))
}

fn error_task_output(
    command: &str,
    error: CliError,
    stdout: Vec<u8>,
    mut stderr: Vec<u8>,
) -> CliOutput {
    if !stderr.is_empty() && !stderr.ends_with(b"\n") {
        stderr.push(b'\n');
    }
    stderr.extend_from_slice(format!("wanix-rust {command}: {error}\n").as_bytes());
    CliOutput::new(stdout, stderr, 1)
}

pub(crate) fn bind_host_mounts(task: &Task, mounts: &[HostMount]) -> Result<(), CliError> {
    for mount in mounts {
        let local = Arc::new(LocalFs::new(&mount.host_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to mount {} at {}: {error}",
                    mount.host_path.display(),
                    mount.guest_path
                ),
                1,
            )
        })?);
        task.bind(
            local,
            ".",
            mount.guest_path.as_str(),
            BindOptions::default(),
        )?;
    }
    Ok(())
}

pub(crate) fn ensure_snapshot_task_fds_closed(task: &Task) -> Result<(), CliError> {
    let dynamic_fds = task
        .fd_numbers()
        .into_iter()
        .filter(|fd| fd.get() > Fd::STDERR.get())
        .map(|fd| fd.get().to_string())
        .collect::<Vec<_>>();
    if dynamic_fds.is_empty() {
        return Ok(());
    }
    Err(CliError::new(
        format!(
            "cannot snapshot qjs task with open Wanix task fds: {}",
            dynamic_fds.join(", ")
        ),
        1,
    ))
}

pub(crate) fn apply_qjs_task_runtime_limits(
    runtime: &mut QuickJsTaskRuntime,
    interrupt_poll_budget: Option<usize>,
    bytes: Option<u32>,
) -> Result<(), CliError> {
    if let Some(polls) = interrupt_poll_budget {
        runtime.set_interrupt_poll_budget(polls)?;
    }
    if let Some(bytes) = bytes {
        runtime.set_memory_limit(bytes)?;
    }
    Ok(())
}

pub(crate) fn eval_qjs_source(
    runtime: &mut QuickJsTaskRuntime,
    source: &str,
    filename: &str,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
) -> Result<(), CliError> {
    if uses_module_syntax(source) {
        runtime.eval_module_discard_with_event_loop_limits(
            source,
            filename,
            event_loop_wait_budget,
            ready_io_turns,
        )?;
    } else {
        runtime.eval_discard_with_event_loop_limits(
            source,
            event_loop_wait_budget,
            ready_io_turns,
        )?;
    }
    Ok(())
}

fn uses_module_syntax(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("export ")
    })
}

pub(crate) fn quickjs_runner() -> Result<Arc<QuickJsRunner>, CliError> {
    match std::env::var_os("WANIX_QJS_WASM") {
        Some(path) => QuickJsRunner::from_wasm_file(PathBuf::from(path)).map(Arc::new),
        None => bundled_quickjs_runner(),
    }
    .map_err(CliError::from)
}

#[cfg(not(test))]
fn bundled_quickjs_runner() -> Result<Arc<QuickJsRunner>, FsError> {
    QuickJsRunner::from_bundled_wasm().map(Arc::new)
}

#[cfg(test)]
fn bundled_quickjs_runner() -> Result<Arc<QuickJsRunner>, FsError> {
    static RUNNER: OnceLock<Result<Arc<QuickJsRunner>, String>> = OnceLock::new();
    match RUNNER.get_or_init(|| {
        QuickJsRunner::from_bundled_wasm()
            .map(Arc::new)
            .map_err(|err| err.to_string())
    }) {
        Ok(runner) => Ok(Arc::clone(runner)),
        Err(error) => Err(FsError::Other(error.clone())),
    }
}

pub(crate) fn parse_exit(exit: &str) -> i32 {
    exit.trim().parse().unwrap_or(0)
}
