//! The `wanix-sh` shell as a `wasm32-wasip1` command guest.
//!
//! It backs the shell's [`NamespaceOps`] surface with WASI: standard I/O for the
//! shell's own output, and the `#task` device for launching child commands. The
//! Wanix `wanix-wasm` driver wires this guest's fd 0/1/2 to the task's stdio, so
//! writes here flow to the task's standard output/error.

use std::io::Write;

use wanix_sh::{NamespaceOps, ShellError, ShellResult, SpawnSpec, run_shell};

struct WasiNamespace;

impl NamespaceOps for WasiNamespace {
    fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()> {
        std::io::stdout()
            .write_all(bytes)
            .map_err(|err| ShellError::Io(err.to_string()))
    }

    fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()> {
        std::io::stderr()
            .write_all(bytes)
            .map_err(|err| ShellError::Io(err.to_string()))
    }

    fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32> {
        // Allocate a child task whose kind is auto-selected by the program's
        // extension (`.wasm` -> wasm driver, `.js` -> qjs driver, …).
        let self_id = read_service("#task/self/id")?;
        let child = read_service("#task/new/auto")?;
        let base = format!("#task/{child}");

        write_service(&format!("{base}/cmd"), &command_line(spec))?;
        // Inherit the shell's stdout/stderr so the child's output reaches the
        // same place the shell writes. (stdin inheritance + redirection arrive
        // with pipelines.)
        for fd in [1, 2] {
            write_service(
                &format!("{base}/ctl"),
                &format!("bind #task/{self_id}/fd/{fd} fd/{fd}"),
            )?;
        }
        write_service(&format!("{base}/ctl"), "start")?;

        let exit = read_service(&format!("{base}/exit"))?;
        exit.parse::<i32>()
            .map_err(|_| ShellError::Io(format!("invalid exit status {exit:?}")))
    }
}

/// Builds a shell-quoted command line from a spawn spec.
fn command_line(spec: &SpawnSpec) -> String {
    let mut line = quote(&spec.program);
    for arg in &spec.args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    line
}

/// Single-quotes a word so the `#task` cmd parser keeps it as one argument.
fn quote(word: &str) -> String {
    if !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b)) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn read_service(path: &str) -> ShellResult<String> {
    std::fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .map_err(|err| ShellError::Io(format!("{path}: {err}")))
}

fn write_service(path: &str, value: &str) -> ShellResult<()> {
    // Service files already exist; open write-only without create/truncate
    // (std::fs::write would set O_CREAT|O_TRUNC, which the host rejects with
    // EEXIST on a device file).
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|mut file| file.write_all(value.as_bytes()))
        .map_err(|err| ShellError::Io(format!("{path}: {err}")))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut ns = WasiNamespace;
    let code = run_shell(&args, &mut ns);
    std::process::exit(code);
}
