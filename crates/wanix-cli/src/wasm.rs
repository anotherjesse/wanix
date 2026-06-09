//! `wanix-rust wasm <file.wasm> [args...]` — turnkey runner for a compiled
//! `wasm32-wasi` task, with the same one-command ergonomics as `wanix-rust qjs`.
//!
//! Sane defaults so the common case needs no host glue: the working directory
//! (`--cwd`, default `.`) is preopened as the namespace root (the guest
//! reads/writes real files), argv is `[program, args...]`, and stdout/stderr are
//! captured into [`CaptureFile`] sinks and returned as a [`CliOutput`] — the
//! collected handler model the rest of the CLI uses.
//!
//! This is the standalone CLI runner path: a one-shot WASI command host that
//! compiles and runs a `.wasm` file directly. Task-driver integration (auto-start
//! as a `#task/new/wasm` Wanix task) lives separately in
//! [`wanix_wasm::WasmTaskDriver`]; this file does not go through the task model.
//! The underlying linker speaks a command-style WASI subset (blocking `fd_read`
//! and an `fd_read`-only `poll_oneoff`; see `wanix-wasi-host`).

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use wanix_fs::{File, FileSystem, LocalFs, MemFs, NormalizedPath, OpenOptions};
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::WasiConfig;
use wanix_wasm::{CaptureFile, WasiRunner, module_cache_dir};

use crate::mesh::IrohMount;
use crate::qjs_args::{QjsStdin, read_qjs_stdin};
use crate::wasm_args::WasmCommand;
use crate::{CliError, CliOutput, bind_mesh_mounts_into};

pub(super) fn run_wasm(
    command: WasmCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let bytes = read_wasm_module(&command)?;
    let captured = captured_wasm_config(command, process_stdin)?;
    let runner = WasiRunner::from_bytes_cached(&bytes, &module_cache_dir())
        .map_err(|error| CliError::new(format!("wasm: {error:#}"), 1))?;
    let CapturedWasmConfig {
        config,
        stdout,
        stderr,
        mesh_mounts: _mesh_mounts,
    } = captured;
    // `_mesh_mounts` stays alive across `run`: each keepalive owns the runtime
    // the guest's mesh-mounted filesystem ops run on.
    let exit = runner
        .run(config)
        .map_err(|error| CliError::new(format!("wasm: {error:#}"), 1))?;

    Ok(CliOutput::new(stdout.bytes(), stderr.bytes(), exit))
}

fn read_wasm_module(command: &WasmCommand) -> Result<Vec<u8>, CliError> {
    std::fs::read(&command.path).map_err(|error| {
        CliError::new(
            format!("wasm: cannot read {}: {error}", command.path.display()),
            1,
        )
    })
}

struct CapturedWasmConfig {
    config: WasiConfig,
    stdout: CaptureFile,
    stderr: CaptureFile,
    /// `--mount-mesh` dialer keepalives; must outlive the guest run (each owns
    /// the runtime its mounted filesystem drives QUIC ops on).
    mesh_mounts: Vec<IrohMount>,
}

fn captured_wasm_config(
    command: WasmCommand,
    process_stdin: &mut dyn Read,
) -> Result<CapturedWasmConfig, CliError> {
    let WasmCommand {
        path,
        args,
        env,
        cwd,
        stdin,
        mesh_mounts,
    } = command;
    let mut namespace = preopen_cwd(&cwd)?;
    let mesh_mounts = bind_mesh_mounts_into(&mut namespace, &mesh_mounts)?;
    let stdout = CaptureFile::new();
    let stderr = CaptureFile::new();
    let mut config = WasiConfig::new(namespace)
        .with_args(wasm_argv(&path, args))
        .with_env(env)
        .with_stdout(Box::new(stdout.clone()), "stdout")
        .with_stderr(Box::new(stderr.clone()), "stderr");
    config = attach_wasm_stdin(config, stdin, process_stdin)?;
    Ok(CapturedWasmConfig {
        config,
        stdout,
        stderr,
        mesh_mounts,
    })
}

fn wasm_argv(path: &Path, args: Vec<String>) -> Vec<String> {
    let mut argv = vec![wasm_program_name(path)];
    argv.extend(args);
    argv
}

fn wasm_program_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "guest".to_owned(), |s| s.to_string_lossy().into_owned())
}

fn attach_wasm_stdin(
    config: WasiConfig,
    stdin: Option<QjsStdin>,
    process_stdin: &mut dyn Read,
) -> Result<WasiConfig, CliError> {
    let Some(bytes) = read_qjs_stdin(stdin, process_stdin)? else {
        return Ok(config);
    };
    Ok(config.with_stdin(stdin_file(&bytes)?, "stdin"))
}

/// Preopens the working directory (`--cwd`, default `.`) as the namespace root,
/// so paths resolve where the user ran the command — matching `wanix-rust qjs`.
fn preopen_cwd(cwd: &NormalizedPath) -> Result<Namespace, CliError> {
    let dir = cwd.as_str();
    let mut namespace = Namespace::new();
    let local = Arc::new(
        LocalFs::new(dir)
            .map_err(|error| CliError::new(format!("wasm: cannot open {dir}: {error}"), 1))?,
    );
    namespace
        .bind(local, ".", ".", BindOptions::default())
        .map_err(|error| CliError::new(format!("wasm: bind failed: {error}"), 1))?;
    Ok(namespace)
}

/// Wraps `bytes` in a readable in-memory file for use as task stdin.
fn stdin_file(bytes: &[u8]) -> Result<Box<dyn File>, CliError> {
    let fs = MemFs::new();
    fs.write_file("stdin", bytes)?;
    Ok(fs.open(&NormalizedPath::new("stdin")?, OpenOptions::read())?)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{WasmCommand, run_wasm, stdin_file};
    use crate::mesh::mounts::test_support::serve_native;
    use crate::qjs_args::MeshMountSpec;
    use wanix_fs::{FileSystem, FsError, MemFs, NormalizedPath};

    const RUST_GUEST: &[u8] = include_bytes!("../../wanix-wasm/fixtures/rust-guest.wasm");

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = PathBuf::from(format!(
            "target/wanix-cli-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wasm_command(root: &std::path::Path, args: &[&str]) -> WasmCommand {
        WasmCommand {
            path: root.join("guest.wasm"),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            env: Vec::new(),
            cwd: NormalizedPath::new(root.to_str().unwrap()).unwrap(),
            stdin: None,
            mesh_mounts: Vec::new(),
        }
    }

    #[test]
    fn run_wasm_preopens_cwd_and_captures_stdio() {
        let root = temp_dir("run-wasm");
        std::fs::write(root.join("guest.wasm"), RUST_GUEST).unwrap();
        std::fs::write(root.join("in.txt"), b"hello").unwrap();

        let command = wasm_command(&root, &["/in.txt", "/out.txt"]);
        let output = run_wasm(command, &mut std::io::empty()).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert!(String::from_utf8_lossy(output.stdout()).contains("rust-wasm: read 5 bytes"));
        assert_eq!(
            std::fs::read_to_string(root.join("out.txt")).unwrap(),
            "rust-wasm saw: hello"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// The wasm-task mirror of `mesh_mount_binds_into_task_namespace_and_stays_alive`:
    /// a compiled WASI guest reads a seeded file through a `--mount-mesh` import
    /// and writes its output back through the same mount, with the served root
    /// observing the write — the keepalive runtime stays alive for the whole run.
    #[test]
    fn run_wasm_resolves_mesh_mounted_paths_from_the_guest() {
        let host = Arc::new(MemFs::new());
        host.write_file("seed.txt", b"served-bytes").unwrap();
        let (server, url) = serve_native(host.clone() as Arc<dyn FileSystem>, 8);

        let root = temp_dir("run-wasm-mesh");
        std::fs::write(root.join("guest.wasm"), RUST_GUEST).unwrap();

        let mut command = wasm_command(&root, &["/vol/seed.txt", "/vol/out.txt"]);
        command.mesh_mounts = vec![MeshMountSpec {
            addr: url,
            guest_path: NormalizedPath::new("vol").unwrap(),
        }];
        let output = run_wasm(command, &mut std::io::empty()).unwrap();

        assert_eq!(
            output.exit_code(),
            0,
            "stderr: {}",
            String::from_utf8_lossy(output.stderr())
        );
        assert!(String::from_utf8_lossy(output.stdout()).contains("rust-wasm: read 12 bytes"));
        // The guest's write crossed the wire into the served root.
        assert_eq!(
            host.read_file("out.txt").unwrap(),
            b"rust-wasm saw: served-bytes"
        );
        drop(server);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn stdin_file_exposes_bytes_through_readable_file() {
        let mut file = stdin_file(b"stdin bytes").unwrap();
        let mut bytes = vec![0; 16];

        let count = file.read(&mut bytes).unwrap();
        let eof = file.read(&mut bytes[count..]).unwrap();

        assert_eq!(&bytes[..count], b"stdin bytes");
        assert_eq!(eof, 0);
        assert_eq!(
            file.write(b"not stdin").unwrap_err(),
            FsError::PermissionDenied
        );
    }
}
