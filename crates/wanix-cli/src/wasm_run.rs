//! `wanix-rust wasm <file.wasm> [args...]` — turnkey runner for a compiled
//! `wasm32-wasi` task, with the same one-command ergonomics as `wanix-rust qjs`.
//!
//! Sane defaults so the common case needs no host glue: a host directory
//! (`--cwd`, default `.`) is preopened as the namespace root (the guest
//! reads/writes real files), argv is `[program, args...]`, and stdout/stderr go
//! to the process streams. Shares `--env`/`--cwd`/`--stdin`/`--stdin-file`
//! parsing with `qjs` via [`crate::try_parse_common_flag`].

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use wanix_fs::{FileSystem, LocalFs, MemFs, NormalizedPath, OpenOptions};
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::WasiConfig;
use wanix_wasm::{CaptureFile, WasiRunner};

use crate::{CliError, QjsStdin, read_qjs_stdin, try_parse_common_flag};

/// A parsed `wasm` command: the module path, guest args, and shared invocation
/// flags (env, cwd, stdin) parsed by the same code path as `qjs`.
pub struct WasmCommand {
    path: PathBuf,
    args: Vec<String>,
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<QjsStdin>,
}

/// Parses `wasm [--env K=V] [--cwd DIR] [--stdin TEXT|--stdin-file PATH] FILE.wasm [args...]`.
///
/// # Errors
///
/// Returns a usage error if no module path is given or a flag is malformed.
pub fn parse_wasm_command(rest: &[OsString]) -> Result<WasmCommand, CliError> {
    let mut env = Vec::new();
    let mut cwd = NormalizedPath::new(".")?;
    let mut stdin = None;
    let mut i = 0;
    while i < rest.len() {
        if try_parse_common_flag(rest, &mut i, "wasm", &mut env, &mut cwd, &mut stdin)? {
            continue;
        }
        if rest[i] == "--" {
            i += 1;
        }
        break;
    }

    let path = rest
        .get(i)
        .ok_or_else(|| CliError::usage("wasm expects a FILE.wasm path"))?;
    i += 1;
    let args = rest[i..]
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    Ok(WasmCommand {
        path: PathBuf::from(path),
        args,
        env,
        cwd,
        stdin,
    })
}

/// Runs a compiled `wasm32-wasi` module as a Wanix task, streaming its captured
/// stdout/stderr to the process and returning its exit code.
///
/// # Errors
///
/// Returns an error if the module or its directory cannot be read, or the task
/// traps.
pub fn run_wasm_streaming(
    command: WasmCommand,
    process_stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let bytes = std::fs::read(&command.path).map_err(|error| {
        CliError::new(
            format!("wasm: cannot read {}: {error}", command.path.display()),
            1,
        )
    })?;

    let program = command
        .path
        .file_name()
        .map_or_else(|| "guest".to_string(), |s| s.to_string_lossy().into_owned());

    // Preopen the working directory (--cwd, default ".") as the namespace root,
    // so paths resolve where the user ran the command — matching `wanix-rust qjs`.
    let dir = command.cwd.as_str();
    let mut namespace = Namespace::new();
    let local = Arc::new(
        LocalFs::new(dir)
            .map_err(|error| CliError::new(format!("wasm: cannot open {dir}: {error}"), 1))?,
    );
    namespace
        .bind(local, ".", ".", BindOptions::default())
        .map_err(|error| CliError::new(format!("wasm: bind failed: {error}"), 1))?;

    let out = CaptureFile::new();
    let err = CaptureFile::new();
    let mut argv = vec![program];
    argv.extend(command.args);
    let mut config = WasiConfig::new(namespace)
        .with_args(argv)
        .with_env(command.env)
        .with_stdout(Box::new(out.clone()), "stdout")
        .with_stderr(Box::new(err.clone()), "stderr");
    if let Some(bytes) = read_qjs_stdin(command.stdin, process_stdin)? {
        config = config.with_stdin(stdin_file(&bytes)?, "stdin");
    }

    let runner = WasiRunner::from_bytes(&bytes)
        .map_err(|error| CliError::new(format!("wasm: {error:#}"), 1))?;
    let exit = runner
        .run(config)
        .map_err(|error| CliError::new(format!("wasm: {error:#}"), 1))?;

    write_all(stdout, &out.bytes(), "stdout")?;
    write_all(stderr, &err.bytes(), "stderr")?;
    Ok(exit)
}

/// Wraps `bytes` in a readable in-memory file for use as task stdin.
fn stdin_file(bytes: &[u8]) -> Result<Box<dyn wanix_fs::File>, CliError> {
    let fs = MemFs::new();
    fs.write_file("stdin", bytes)?;
    Ok(fs.open(&NormalizedPath::new("stdin")?, OpenOptions::read_write())?)
}

fn write_all(sink: &mut dyn Write, bytes: &[u8], label: &str) -> Result<(), CliError> {
    sink.write_all(bytes)
        .map_err(|error| CliError::new(format!("failed to write process {label}: {error}"), 1))
}
