use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::OnceLock;
use std::time::Duration;

use wanix_fs::{FileSystem, FsError, LocalFs, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::{QuickJsRunner, QuickJsTaskRuntime};
use wanix_task::{Fd, Task};
use wanix_vfs::BindOptions;

use crate::qjs_args::{HostMount, MeshMountSpec};
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

/// Dials each `--mount-mesh` spec over the native mesh wire and binds the
/// imported remote into the task namespace at its guest path.
///
/// Returns the dialer [`crate::mesh::IrohMount`] keepalives. Each owns the tokio
/// runtime its imported `FileSystem` drives QUIC ops on, so **the caller must
/// hold the returned vector for as long as the task may touch the mount** —
/// dropping a keepalive shuts down that runtime and panics the next op. The
/// runtime path stores it on the prepared-execution object so it outlives the
/// task.
pub(crate) fn bind_mesh_mounts(
    task: &Task,
    mounts: &[MeshMountSpec],
) -> Result<Vec<crate::mesh::IrohMount>, CliError> {
    let mut keepalives = Vec::with_capacity(mounts.len());
    for mount in mounts {
        let dialed = crate::mesh::dial_iroh_remote(&mount.addr, "")?;
        task.bind(
            dialed.remote.clone(),
            ".",
            mount.guest_path.as_str(),
            BindOptions::default(),
        )?;
        keepalives.push(dialed);
    }
    Ok(keepalives)
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

#[cfg(test)]
mod mesh_mount_tests {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_id::NodeIdentity;
    use wanix_mesh::{MeshNode, NativeServeConfig};
    use wanix_task::TaskTable;
    use wanix_vfs::Namespace;

    use super::bind_mesh_mounts;
    use crate::mesh::IROH_SCHEME;
    use crate::qjs_args::MeshMountSpec;

    fn loopback() -> SocketAddr {
        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
    }

    /// Serves `host` over the native mesh wire and returns the node plus the
    /// dialable `iroh://` URL (built the way `mesh-serve` announces it).
    fn serve_native(host: Arc<dyn FileSystem>, identity_seed: u8) -> (MeshNode, String) {
        let identity = NodeIdentity::from_secret_bytes([identity_seed; 32]);
        let mut server = MeshNode::bind_local(&identity, loopback()).unwrap();
        server.serve_native(NativeServeConfig::open(host));
        let peer = server.peer_id();
        let addrs: Vec<String> = server
            .ticket()
            .ip_addrs()
            .map(|addr| format!("addr={addr}"))
            .collect();
        (server, format!("{IROH_SCHEME}{peer}?{}", addrs.join("&")))
    }

    /// A fresh single-task `noop` table, mirroring `allocate_qjs_term_task`'s
    /// self-contained table (the returned `Task` does not borrow the table).
    fn noop_task() -> wanix_task::Task {
        let table = TaskTable::new();
        table.register_noop_driver("noop").unwrap();
        table.allocate_root("noop").unwrap()
    }

    fn read_through(namespace: &Namespace, path: &str) -> Vec<u8> {
        let mut file = namespace
            .open(&NormalizedPath::new(path).unwrap(), OpenOptions::read())
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = file.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        bytes
    }

    /// Slice 1 runtime proof: a `--mount-mesh` spec dialed by `bind_mesh_mounts`
    /// binds a remote native-wire volume into a task namespace, and the task can
    /// read a seeded file and write a new one (the served root observes it) while
    /// the returned keepalive is held. Dropping the keepalive only after the task
    /// mirrors the field order the qjs-shell runtime path relies on.
    #[test]
    fn mesh_mount_binds_into_task_namespace_and_stays_alive() {
        let host = Arc::new(MemFs::new());
        host.write_file("seed.txt", b"served-by-A").unwrap();
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 3);
        let task = noop_task();

        let spec = MeshMountSpec {
            addr: url,
            guest_path: NormalizedPath::new("vol").unwrap(),
        };
        let keepalive = bind_mesh_mounts(&task, std::slice::from_ref(&spec)).unwrap();
        assert_eq!(keepalive.len(), 1);

        // Read a seeded file through the task namespace: the mount is wired and the
        // keepalive runtime is alive for the read.
        let namespace = task.namespace();
        assert_eq!(read_through(&namespace, "vol/seed.txt"), b"served-by-A");

        // Write through the mount; the served root observes it across the wire.
        let payload = b"from-task";
        let mut file = namespace
            .open(
                &NormalizedPath::new("vol/from-task.txt").unwrap(),
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        assert_eq!(file.write(payload).unwrap(), payload.len());
        drop(file);
        assert_eq!(host.read_file("from-task.txt").unwrap(), payload);

        // Mirror PreparedQjsTermExecution drop order: task before the keepalive.
        drop(task);
        drop(keepalive);
        drop(server);
    }

    /// Slice 2 proof: two independent mesh mounts (two dialer nodes, as two
    /// separate `qjs-shell` processes would be) against ONE served open volume —
    /// a write through one mount is visible through the other. This is the
    /// resource-composition experience before any naming layer exists.
    #[test]
    fn two_task_namespaces_share_one_open_volume() {
        let host = Arc::new(MemFs::new());
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 4);
        let spec = MeshMountSpec {
            addr: url,
            guest_path: NormalizedPath::new("vol").unwrap(),
        };

        // Two independent task namespaces, each dialing its own mount/keepalive.
        let task_a = noop_task();
        let keep_a = bind_mesh_mounts(&task_a, std::slice::from_ref(&spec)).unwrap();
        let task_b = noop_task();
        let keep_b = bind_mesh_mounts(&task_b, std::slice::from_ref(&spec)).unwrap();

        // Shell A writes /vol/shared.txt; shell B reads it back over its own mount.
        let payload = b"written-by-A-read-by-B";
        let namespace_a = task_a.namespace();
        let mut file = namespace_a
            .open(
                &NormalizedPath::new("vol/shared.txt").unwrap(),
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        assert_eq!(file.write(payload).unwrap(), payload.len());
        drop(file);

        let namespace_b = task_b.namespace();
        assert_eq!(read_through(&namespace_b, "vol/shared.txt"), payload);
        // The single served root holds the shared byte stream.
        assert_eq!(host.read_file("shared.txt").unwrap(), payload);

        drop(task_a);
        drop(task_b);
        drop(keep_a);
        drop(keep_b);
        drop(server);
    }
}
