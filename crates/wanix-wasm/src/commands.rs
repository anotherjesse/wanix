//! The installable command set: compiled wasm commands shipped as fixtures.
//!
//! Each command is a standalone `wasm32-wasip1` program built from its own
//! isolated crate under `fixtures/commands/<name>/` and checked in as
//! `fixtures/commands/<name>.wasm`. [`command_bin`] assembles them into a `bin`
//! filesystem you bind into a task's namespace; the shell's command resolution
//! then finds `bin/<name>.wasm` when you type `<name>`.
//!
//! To add a command, see `fixtures/commands/HOW-TO-ADD-A-COMMAND.md`: create the
//! crate, build it to wasm, drop the `.wasm` next to the others, and add one
//! [`COMMANDS`] entry.

use std::sync::Arc;

use wanix_fs::MemFs;

/// The available commands as `(name, wasm bytes)`.
///
/// Bytes are embedded at build time; the shell resolves a bare `name` to
/// `bin/<name>.wasm` (see [`crate::command_bin`]).
pub const COMMANDS: &[(&str, &[u8])] = &[("jaq", include_bytes!("../fixtures/commands/jaq.wasm"))];

/// The bundled `wanix-sh` shell (`crates/wanix-wasm/fixtures/shell-src`),
/// compiled to `wasm32-wasip1`.
///
/// The shell is an ordinary command guest — seed it into a task namespace (for
/// example next to [`command_bin`]'s commands) and run it through
/// [`crate::WasmTaskDriver`]; `-c LINE` runs one line, no `-c` starts the REPL.
pub const SHELL_WASM: &[u8] = include_bytes!("../fixtures/shell.wasm");

/// Builds an in-memory `bin` filesystem holding every command as `<name>.wasm`.
///
/// Bind the returned filesystem at `bin` in a task's namespace; children inherit
/// it through namespace cloning, and the shell finds `bin/<name>.wasm`.
#[must_use]
pub fn command_bin() -> Arc<MemFs> {
    let fs = Arc::new(MemFs::new());
    for &(name, bytes) in COMMANDS {
        fs.write_file(format!("{name}.wasm"), bytes)
            .expect("seed command into bin filesystem");
    }
    fs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_bin_holds_jaq() {
        let bin = command_bin();
        let bytes = bin.read_file("jaq.wasm").expect("jaq.wasm present in bin");
        assert!(!bytes.is_empty(), "jaq.wasm should be non-empty");
    }

    #[test]
    fn commands_list_is_nonempty_and_named() {
        assert!(COMMANDS.iter().any(|&(name, _)| name == "jaq"));
    }
}
