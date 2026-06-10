//! The resident app guest: a detached qjs task speaking the file2chan
//! protocol over `#pipe`-backed stdio.
//!
//! The guest runs as an ordinary Wanix task (ADR 0002): its namespace holds
//! the app directory at the root, the durable state directory at `/state`,
//! and `#pipe`; fds 0/1 are the guest ends of two pipe channels whose host
//! ends back the [`wanix_appfs::AppSender`]/[`wanix_appfs::AppReceiver`]
//! halves the `wanix-appfs` adapter transacts over (see [`super::stdio`]).
//! Stdio reads are blocking (ADR 0010 tier 2), so the guest's
//! read-decide-reply loop parks between events instead of polling.
//!
//! Lifecycle honesty (no auto-restart in v0): when the guest exits or
//! crashes, its fds close, so the adapter's pump observes the broken pipe,
//! latches the channel down — discrete ops surface as
//! [`wanix_fs::FsError::Unreachable`], the honest variant, because the app
//! endpoint still exists but the implementation behind it is down (the same
//! "resource down, not missing" vocabulary the mesh uses for a dead peer) —
//! and closes the stream surface so blocked `stream` readers observe EOF
//! rather than parking forever. The exit watcher thread here additionally
//! logs the exit (with the guest's captured stderr) to the serve process's
//! stderr.

use std::path::Path;
use std::sync::Arc;

use wanix_appfs::{AppFsService, AppTree, LineBuffer};
use wanix_fs::{File, FileSystem, LocalFs, NormalizedPath, OpenOptions};
use wanix_pipe::PipeDevice;
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::{Fd, Task, TaskTable};
use wanix_vfs::{BindOptions, Namespace};

use super::AppManifest;
use super::stdio::{PipeReceiver, PipeSender, StderrCapture, drain_captured};
use crate::{CliError, configure_qjs_task, quickjs_runner};

/// The running guest behind one served app: the task (for exit observation)
/// and its table keepalive. Dropping it does not kill the detached guest;
/// the guest exits when the adapter channel (the service) drops and its
/// stdin reaches EOF.
pub(crate) struct AppGuest {
    /// Keepalive for the driver registry backing the detached task.
    _table: TaskTable,
    /// The guest task handle. Production observes exit through the watcher
    /// thread; tests wait on it directly to pin the lifecycle contracts.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) task: Task,
}

/// Starts the manifest's qjs guest as a detached task and returns it with
/// the [`AppFsService`] adapting its stdio line protocol.
///
/// # Errors
///
/// Returns a CLI error when the app/state directories cannot be opened, the
/// declared tree is invalid, or the task cannot be built and started.
pub(crate) fn start_app_guest(
    app_dir: &Path,
    state_dir: &Path,
    manifest: &AppManifest,
) -> Result<(AppGuest, AppFsService), CliError> {
    let pipes = PipeDevice::new();
    let stdin_pipe = pipes.alloc()?;
    let stdout_pipe = pipes.alloc()?;

    let (table, task) = allocate_guest_task(app_dir, state_dir, &pipes, manifest)?;
    attach_guest_stdio(&task, &pipes, &stdin_pipe, &stdout_pipe)?;
    let stderr = attach_guest_stderr(&task)?;
    // Host ends, opened before the guest starts so a guest-side EOF can only
    // mean the guest itself is gone (writes buffer until the guest reads).
    let sender = PipeSender {
        guest_stdin: open_pipe_end(&pipes, &stdin_pipe, write_only())?,
    };
    let receiver = PipeReceiver {
        guest_stdout: open_pipe_end(&pipes, &stdout_pipe, OpenOptions::read())?,
        pending: Vec::new(),
    };
    table.start_detached(task.id())?;

    let tree = declare_tree(manifest)?;
    let service = AppFsService::new(tree, Box::new(sender), Box::new(receiver));
    spawn_exit_watcher(&task, &service, stderr);
    Ok((
        AppGuest {
            _table: table,
            task,
        },
        service,
    ))
}

fn declare_tree(manifest: &AppManifest) -> Result<AppTree, CliError> {
    let files: Vec<&str> = manifest.files.iter().map(String::as_str).collect();
    let streams: Vec<&str> = manifest.streams.iter().map(String::as_str).collect();
    AppTree::declare(&files, &streams)
        .map_err(|message| CliError::new(format!("app serve: {message}"), 1))
}

/// Builds the guest namespace (app dir at the root, durable state at
/// `/state`, `#pipe` for the stdio fd paths) and allocates the qjs task.
fn allocate_guest_task(
    app_dir: &Path,
    state_dir: &Path,
    pipes: &PipeDevice,
    manifest: &AppManifest,
) -> Result<(TaskTable, Task), CliError> {
    let mut namespace = Namespace::new();
    let open_dir = |dir: &Path, what: &str| {
        LocalFs::new(dir).map_err(|error| {
            CliError::new(
                format!("app serve: cannot open {what} {}: {error}", dir.display()),
                1,
            )
        })
    };
    namespace.bind(
        Arc::new(open_dir(app_dir, "--app")?),
        ".",
        ".",
        BindOptions::default(),
    )?;
    namespace.bind(
        Arc::new(open_dir(state_dir, "--state")?),
        ".",
        "state",
        BindOptions::default(),
    )?;
    namespace.bind(
        Arc::new(pipes.clone()),
        ".",
        "#pipe",
        BindOptions::default(),
    )?;

    let table = TaskTable::new();
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
    let task = table.allocate_root_with_namespace("auto", namespace)?;
    configure_qjs_task(
        &task,
        &manifest.runtime.main,
        &[],
        &[],
        &NormalizedPath::new(".")?,
    )?;
    Ok((table, task))
}

/// Binds the guest ends: fd 0 reads the stdin pipe, fd 1 writes the stdout
/// pipe. The fd paths point into the bound `#pipe` device (ADR 0002).
fn attach_guest_stdio(
    task: &Task,
    pipes: &PipeDevice,
    stdin_pipe: &str,
    stdout_pipe: &str,
) -> Result<(), CliError> {
    let stdin = open_pipe_end(pipes, stdin_pipe, OpenOptions::read())?;
    task.insert_fd(
        Fd::STDIN,
        stdin,
        NormalizedPath::new(format!("#pipe/{stdin_pipe}/data"))?,
    )?;
    let stdout = open_pipe_end(pipes, stdout_pipe, write_only())?;
    task.insert_fd(
        Fd::STDOUT,
        stdout,
        NormalizedPath::new(format!("#pipe/{stdout_pipe}/data"))?,
    )?;
    Ok(())
}

/// Captures guest stderr into a bounded drop-oldest buffer the exit watcher
/// logs from. Bounded because the serve process is resident: a guest that
/// logs forever must not grow host memory without bound (the appfs
/// stream-buffer discipline; only the most recent backlog matters at exit).
fn attach_guest_stderr(task: &Task) -> Result<Arc<LineBuffer>, CliError> {
    let buffer = Arc::new(LineBuffer::default());
    task.insert_fd(
        Fd::STDERR,
        Box::new(StderrCapture::new(Arc::clone(&buffer))),
        NormalizedPath::new("stderr")?,
    )?;
    Ok(buffer)
}

fn open_pipe_end(
    pipes: &PipeDevice,
    id: &str,
    options: OpenOptions,
) -> Result<Box<dyn File>, CliError> {
    Ok(pipes.open(&NormalizedPath::new(format!("{id}/data"))?, options)?)
}

fn write_only() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

/// Watches for guest exit, then tears down the stream surface (blocked
/// readers observe EOF; idempotent with the adapter pump's own teardown) and
/// logs the exit. Holds only the task, the service's stream closer, and the
/// stderr capture — never the service itself, which owns the guest's stdin
/// pipe and would keep the guest alive forever.
fn spawn_exit_watcher(task: &Task, service: &AppFsService, stderr: Arc<LineBuffer>) {
    let task = task.clone();
    let closer = service.stream_closer();
    std::thread::spawn(move || {
        let exit = task
            .wait_exit()
            .unwrap_or_else(|error| format!("unknown ({error})"));
        closer.close_all();
        let diagnostics = drain_captured(&stderr);
        eprintln!(
            "wanix-rust app serve: guest exited with status {exit}; discrete ops now fail \
             Unreachable and stream readers see EOF (no auto-restart in v0){}{diagnostics}",
            if diagnostics.is_empty() {
                ""
            } else {
                "\nguest stderr:\n"
            },
        );
    });
}
