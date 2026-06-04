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

use std::fs;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::os::wasi::io::AsRawFd;

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
    if args.get(1).map(String::as_str) == Some("--rmdir") {
        rmdir(args.get(2).map_or("", String::as_str));
        return;
    }
    if args.get(1).map(String::as_str) == Some("--symlink") {
        symlink(
            args.get(2).map_or("", String::as_str),
            args.get(3).map_or("", String::as_str),
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("--readlink") {
        readlink(args.get(2).map_or("", String::as_str));
        return;
    }
    if args.get(1).map(String::as_str) == Some("--tell") {
        tell(
            args.get(2).map_or("", String::as_str),
            args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("--truncate") {
        truncate(
            args.get(2).map_or("", String::as_str),
            args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("--pi") {
        let n = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
        println!("{:.10}", leibniz_pi(n));
        return;
    }
    if args.get(1).map(String::as_str) == Some("--echo") {
        // Proves --env and --stdin reach the guest through the runner.
        use std::io::Read;
        println!("env MSG={}", std::env::var("MSG").unwrap_or_default());
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).ok();
        println!("stdin={}", s.trim());
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

/// Approximates pi with `n` Leibniz terms: 4 * sum (-1)^k / (2k+1).
/// A pure floating-point hot loop — the CPU benchmark kernel, identical to the
/// native Rust and QuickJS versions so the three execution paths are comparable.
fn leibniz_pi(n: u64) -> f64 {
    let mut acc = 0.0f64;
    let mut sign = 1.0f64;
    for k in 0..n {
        acc += sign / (2 * k + 1) as f64;
        sign = -sign;
    }
    4.0 * acc
}

/// Atomically moves `src` to `dst` via `fs::rename` (the `path_rename` syscall).
fn rename(src: &str, dst: &str) {
    match fs::rename(src, dst) {
        Ok(()) => println!("rust-wasm: renamed {src} -> {dst}"),
        Err(err) => println!("rust-wasm: rename failed {src} -> {dst}: {err}"),
    }
}

/// Removes the empty directory `dir` via `fs::remove_dir` (the
/// `path_remove_directory` syscall).
fn rmdir(dir: &str) {
    match fs::remove_dir(dir) {
        Ok(()) => println!("rust-wasm: removed dir {dir}"),
        Err(err) => println!("rust-wasm: rmdir failed {dir}: {err}"),
    }
}

// Raw WASI `path_symlink` import (the stable std wrapper is nightly-only).
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn path_symlink(
        old_path: *const u8,
        old_path_len: usize,
        fd: u32,
        new_path: *const u8,
        new_path_len: usize,
    ) -> u16;
    fn fd_tell(fd: u32, out: *mut u64) -> u16;
}

/// Seeks `path` to absolute offset `seek`, then reports the cursor via the raw
/// `fd_tell` syscall (not `fd_seek`, so this genuinely exercises `fd_tell`).
fn tell(path: &str, seek: u64) {
    let mut file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) => {
            println!("rust-wasm: tell failed to open {path}: {err}");
            return;
        }
    };
    if let Err(err) = file.seek(SeekFrom::Start(seek)) {
        println!("rust-wasm: tell seek failed {path}: {err}");
        return;
    }
    let mut pos: u64 = 0;
    let errno = unsafe { fd_tell(file.as_raw_fd() as u32, &mut pos) };
    if errno == 0 {
        println!("rust-wasm: tell {path} at {pos}");
    } else {
        println!("rust-wasm: tell failed {path}: errno {errno}");
    }
}

/// Creates a symbolic link `link` pointing at `target` via the `path_symlink`
/// syscall, then prints `ok`. Resolves `link` against the preopened root (fd 3)
/// so the absolute `link` path becomes namespace-relative.
fn symlink(target: &str, link: &str) {
    let rel = link.strip_prefix('/').unwrap_or(link);
    let errno = unsafe {
        path_symlink(
            target.as_ptr(),
            target.len(),
            3,
            rel.as_ptr(),
            rel.len(),
        )
    };
    if errno == 0 {
        println!("ok");
    } else {
        println!("rust-wasm: symlink failed {link} -> {target}: errno {errno}");
    }
}

/// Prints the target of the symbolic link `link` via `fs::read_link` (the
/// `path_readlink` syscall).
fn readlink(link: &str) {
    match fs::read_link(link) {
        Ok(target) => println!("{}", target.to_string_lossy()),
        Err(err) => println!("rust-wasm: readlink failed {link}: {err}"),
    }
}

/// Sets `path`'s length to `len` via `File::set_len` (the
/// `fd_filestat_set_size` syscall), opening it for writing first.
fn truncate(path: &str, len: u64) {
    let file = match OpenOptions::new().write(true).open(path) {
        Ok(file) => file,
        Err(err) => {
            println!("rust-wasm: truncate failed to open {path}: {err}");
            return;
        }
    };
    match file.set_len(len) {
        Ok(()) => println!("rust-wasm: truncated {path} to {len}"),
        Err(err) => println!("rust-wasm: truncate failed {path}: {err}"),
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
