//! `wanix-rust wasm <file.wasm> [args...]` — turnkey runner for a compiled
//! `wasm32-wasi` task, with the same one-command ergonomics as `wanix-rust qjs`.
//!
//! Sane defaults so the common case needs no host glue: the wasm file's
//! directory is preopened as the namespace root (the guest reads/writes real
//! sibling files), argv is `[program, args...]`, and stdout/stderr go to the
//! process streams. The task runs in the same sandbox a `qjs` task would.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use wanix_fs::LocalFs;
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::WasiConfig;
use wanix_wasm::{CaptureFile, WasiRunner};

use crate::CliError;

/// A parsed `wasm` command: the module path plus guest arguments.
pub struct WasmCommand {
    path: PathBuf,
    args: Vec<String>,
}

/// Parses `wasm <file.wasm> [args...]`.
///
/// # Errors
///
/// Returns a usage error if no module path is given.
pub fn parse_wasm_command(rest: &[OsString]) -> Result<WasmCommand, CliError> {
    let mut it = rest.iter();
    let path = it
        .next()
        .ok_or_else(|| CliError::usage("wasm expects a FILE.wasm path"))?;
    Ok(WasmCommand {
        path: PathBuf::from(path),
        args: it.map(|s| s.to_string_lossy().into_owned()).collect(),
    })
}

/// Runs a compiled `wasm32-wasi` module as a Wanix task, streaming its captured
/// stdout/stderr to the process and returning its exit code.
///
/// # Errors
///
/// Returns an error if the module cannot be read, the host directory cannot be
/// opened, or the task traps.
pub fn run_wasm_streaming(
    command: WasmCommand,
    _stdin: &mut dyn Read,
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

    // Preopen the process working directory as the namespace root, so paths
    // resolve where the user ran the command — matching `wanix-rust qjs`.
    let mut namespace = Namespace::new();
    let local = Arc::new(
        LocalFs::new(".")
            .map_err(|error| CliError::new(format!("wasm: cannot open .: {error}"), 1))?,
    );
    namespace
        .bind(local, ".", ".", BindOptions::default())
        .map_err(|error| CliError::new(format!("wasm: bind failed: {error}"), 1))?;

    let out = CaptureFile::new();
    let err = CaptureFile::new();
    let mut argv = vec![program];
    argv.extend(command.args);
    let config = WasiConfig::new(namespace)
        .with_args(argv)
        .with_stdout(Box::new(out.clone()), "stdout")
        .with_stderr(Box::new(err.clone()), "stderr");

    let runner = WasiRunner::from_bytes(&bytes)
        .map_err(|error| CliError::new(format!("wasm: {error:#}"), 1))?;
    let exit = runner
        .run(config)
        .map_err(|error| CliError::new(format!("wasm: {error:#}"), 1))?;

    write_all(stdout, &out.bytes(), "stdout")?;
    write_all(stderr, &err.bytes(), "stderr")?;
    Ok(exit)
}

fn write_all(sink: &mut dyn Write, bytes: &[u8], label: &str) -> Result<(), CliError> {
    sink.write_all(bytes)
        .map_err(|error| CliError::new(format!("failed to write process {label}: {error}"), 1))
}
