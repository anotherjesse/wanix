use std::fs::OpenOptions;
use std::fs::{self, DirEntry, ReadDir};

pub(crate) fn copy_transform(input: &str, output: &str) {
    let seen = match fs::read_to_string(input) {
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
    match fs::write(output, payload.as_bytes()) {
        Ok(()) => println!("rust-wasm: wrote {} bytes to {output}", payload.len()),
        Err(err) => println!("rust-wasm: could not write {output}: {err}"),
    }
}

/// Atomically moves `src` to `dst` via `fs::rename` (the `path_rename` syscall).
pub(crate) fn rename(src: &str, dst: &str) {
    match fs::rename(src, dst) {
        Ok(()) => println!("rust-wasm: renamed {src} -> {dst}"),
        Err(err) => println!("rust-wasm: rename failed {src} -> {dst}: {err}"),
    }
}

/// Removes the empty directory `dir` via `fs::remove_dir` (the
/// `path_remove_directory` syscall).
pub(crate) fn rmdir(dir: &str) {
    match fs::remove_dir(dir) {
        Ok(()) => println!("rust-wasm: removed dir {dir}"),
        Err(err) => println!("rust-wasm: rmdir failed {dir}: {err}"),
    }
}

/// Prints the target of the symbolic link `link` via `fs::read_link` (the
/// `path_readlink` syscall).
pub(crate) fn readlink(link: &str) {
    match fs::read_link(link) {
        Ok(target) => println!("{}", target.to_string_lossy()),
        Err(err) => println!("rust-wasm: readlink failed {link}: {err}"),
    }
}

/// Sets `path`'s length to `len` via `File::set_len` (the
/// `fd_filestat_set_size` syscall), opening it for writing first.
pub(crate) fn truncate(path: &str, len: u64) {
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
pub(crate) fn list_dir(dir: &str) {
    if let Some(rows) = read_rows(dir) {
        print_rows(dir, rows);
    }
}

fn read_rows(dir: &str) -> Option<Vec<(String, &'static str)>> {
    let entries = fs::read_dir(dir)
        .map_err(|err| println!("rust-wasm: could not list {dir}: {err}"))
        .ok()?;
    let mut rows = dir_rows(entries, dir)?;
    rows.sort();
    Some(rows)
}

fn print_rows(dir: &str, rows: Vec<(String, &'static str)>) {
    println!("rust-wasm: {dir} has {} entries", rows.len());
    for (name, kind) in rows {
        println!("{name} {kind}");
    }
}

fn dir_rows(entries: ReadDir, dir: &str) -> Option<Vec<(String, &'static str)>> {
    let mut rows = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => rows.push(dir_row(entry)),
            Err(err) => {
                println!("rust-wasm: bad entry in {dir}: {err}");
                return None;
            }
        }
    }
    Some(rows)
}

fn dir_row(entry: DirEntry) -> (String, &'static str) {
    let name = entry.file_name().to_string_lossy().into_owned();
    let kind = match entry.file_type() {
        Ok(ft) if ft.is_dir() => "dir",
        Ok(ft) if ft.is_symlink() => "symlink",
        Ok(_) => "file",
        Err(_) => "unknown",
    };
    (name, kind)
}

pub(crate) fn echo() {
    // Proves --env and --stdin reach the guest through the runner.
    use std::io::Read;

    println!("env MSG={}", std::env::var("MSG").unwrap_or_default());
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).ok();
    println!("stdin={}", s.trim());
}
