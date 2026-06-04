//! Tiny WASI guest used to prove a compiled Rust task shares the Wanix VFS with
//! a qjs task. It reads an input file, prints what it saw, and writes an output
//! file with transformed content — all through plain `std::fs` (WASI syscalls).
//!
//! Usage:
//!   `guest <input-path> <output-path>` — copy/transform a file.
//!   `guest --list <dir>`               — list a directory's entries + types.
//!   `guest --rename <src> <dst>`       — atomically move a file/dir.

use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--list") {
        list_dir(args.get(2).map_or("/", String::as_str));
        return;
    }
    if args.get(1).map(String::as_str) == Some("--rename") {
        rename(
            args.get(2).map_or("", String::as_str),
            args.get(3).map_or("", String::as_str),
        );
        return;
    }

    let input = args.get(1).cloned().unwrap_or_else(|| "/shared/in.txt".to_string());
    let output = args.get(2).cloned().unwrap_or_else(|| "/shared/out.txt".to_string());

    let seen = match fs::read_to_string(&input) {
        Ok(text) => {
            println!("rust-wasm: read {} bytes from {input}", text.len());
            text
        }
        Err(err) => {
            println!("rust-wasm: could not read {input}: {err}");
            String::new()
        }
    };

    let payload = format!("rust-wasm saw: {}", seen.trim());
    match fs::write(&output, payload.as_bytes()) {
        Ok(()) => println!("rust-wasm: wrote {} bytes to {output}", payload.len()),
        Err(err) => println!("rust-wasm: could not write {output}: {err}"),
    }
}

/// Atomically moves `src` to `dst` via `fs::rename` (the `path_rename` syscall).
fn rename(src: &str, dst: &str) {
    match fs::rename(src, dst) {
        Ok(()) => println!("rust-wasm: renamed {src} -> {dst}"),
        Err(err) => println!("rust-wasm: rename failed {src} -> {dst}: {err}"),
    }
}

/// Lists `dir`'s entries, printing one sorted `name type` line each.
fn list_dir(dir: &str) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            println!("rust-wasm: could not list {dir}: {err}");
            return;
        }
    };

    let mut rows: Vec<(String, &'static str)> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                println!("rust-wasm: bad entry in {dir}: {err}");
                return;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = match entry.file_type() {
            Ok(ft) if ft.is_dir() => "dir",
            Ok(ft) if ft.is_symlink() => "symlink",
            Ok(_) => "file",
            Err(_) => "unknown",
        };
        rows.push((name, kind));
    }
    rows.sort();
    println!("rust-wasm: {dir} has {} entries", rows.len());
    for (name, kind) in rows {
        println!("{name} {kind}");
    }
}
