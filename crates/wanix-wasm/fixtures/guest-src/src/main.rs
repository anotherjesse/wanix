//! Tiny WASI guest used to prove a compiled Rust task shares the Wanix VFS with
//! a qjs task. It reads an input file, prints what it saw, and writes an output
//! file with transformed content — all through plain `std::fs` (WASI syscalls).
//!
//! Usage: `guest <input-path> <output-path>`

use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
