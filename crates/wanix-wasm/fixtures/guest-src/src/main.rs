//! Tiny WASI guest used to prove a compiled Rust task shares the Wanix VFS with
//! a qjs task. It reads an input file, prints what it saw, and writes an output
//! file with transformed content — all through plain `std::fs` (WASI syscalls).
//!
//! Usage:
//!   `guest <input-path> <output-path>` — copy/transform a file.
//!   `guest --list <dir>`               — list a directory's entries + types.
//!   `guest --rename <src> <dst>`       — atomically move a file/dir.
//!   `guest --rmdir <dir>`              — remove an empty directory.
//!   `guest --truncate <path> <len>`    — set a file's length (ftruncate).
//!   `guest --symlink <target> <link>`  — create a symbolic link.
//!   `guest --readlink <link>`          — print a symbolic link's target.
//!   `guest --tell <path> <seek>`       — seek then report position via fd_tell.

mod commands;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if commands::run(&args) {
        return;
    }

    let input = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/shared/in.txt".to_string());
    let output = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "/shared/out.txt".to_string());
    commands::copy_transform(&input, &output);
}
