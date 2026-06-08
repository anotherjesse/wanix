//! The `wanix-sh` shell as a `wasm32-wasip1` command guest.
//!
//! It backs the shell's [`NamespaceOps`] surface with WASI standard I/O and runs
//! one invocation, e.g. `shell -c "echo hi"`. The Wanix `wanix-wasm` driver wires
//! this guest's fd 0/1/2 to the task's stdio, so writes here flow to the task's
//! standard output/error.

use std::io::Write;

use wanix_sh::{NamespaceOps, ShellError, ShellResult, run_shell};

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
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut ns = WasiNamespace;
    let code = run_shell(&args, &mut ns);
    std::process::exit(code);
}
