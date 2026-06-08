//! The Wanix static-site generator compiled to `wasm32-wasip1`.
//!
//! Run as a Wanix `.wasm` task: the module is bound into a task namespace where
//! the markdown corpus is readable and an output directory is writable. It
//! reads the input and output directory paths from argv and delegates to the
//! shared [`wanix_site_gen::generate_site`] logic over a [`StdFsIo`] — on
//! `wasm32-wasip1` libstd's `std::fs` lowers to WASI calls (`fd_readdir`,
//! `path_open`, `path_create_directory`, `fd_write`) against the preopened
//! namespace, so the SSG reads the corpus and writes its HTML tree through the
//! Wanix filesystem.
//!
//! Usage: `ssg <input-dir> <output-dir>` (defaults: `content` → `site`).

use wanix_site_gen::{StdFsIo, generate_site};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = args.get(1).cloned().unwrap_or_else(|| "content".to_string());
    let output = args.get(2).cloned().unwrap_or_else(|| "site".to_string());

    // The IO root is the task's cwd / namespace root; `input`/`output` are
    // resolved relative to it.
    let io = StdFsIo::new(".");
    match generate_site(&io, &input, &output) {
        Ok(report) => {
            println!("generated {} pages: {input} -> {output}", report.page_count);
        }
        Err(err) => {
            eprintln!("site generation failed: {err}");
            std::process::exit(1);
        }
    }
}
