//! `wanix-rust wasm <file.wasm> [args...]` — turnkey runner for a compiled
//! `wasm32-wasi` task, with the same one-command ergonomics as `wanix-rust qjs`.
//!
//! Sane defaults so the common case needs no host glue: the working directory
//! (`--cwd`, default `.`) is preopened as the namespace root (the guest
//! reads/writes real files), argv is `[program, args...]`, and stdout/stderr are
//! captured into [`CaptureFile`] sinks and returned as a [`CliOutput`] — the
//! collected handler model the rest of the CLI uses.
//!
//! This is a Tier-1 demo runner: a standalone WASI command host, not yet a
//! Wanix task driver, and the underlying linker speaks a command-style WASI
//! subset (no `poll_oneoff` readiness).

use std::io::Read;
use std::sync::Arc;

use wanix_fs::{File, FileSystem, LocalFs, MemFs, NormalizedPath, OpenOptions};
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::WasiConfig;
use wanix_wasm::{CaptureFile, WasiRunner};

use crate::qjs_args::read_qjs_stdin;
use crate::wasm_args::WasmCommand;
use crate::{CliError, CliOutput};

pub(super) fn run_wasm(
    command: WasmCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let bytes = std::fs::read(&command.path).map_err(|error| {
        CliError::new(
            format!("wasm: cannot read {}: {error}", command.path.display()),
            1,
        )
    })?;

    let program = command
        .path
        .file_name()
        .map_or_else(|| "guest".to_owned(), |s| s.to_string_lossy().into_owned());

    let namespace = preopen_cwd(&command.cwd)?;

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

    Ok(CliOutput::new(out.bytes(), err.bytes(), exit))
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
    Ok(fs.open(&NormalizedPath::new("stdin")?, OpenOptions::read_write())?)
}
