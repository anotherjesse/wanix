//! The `probe` verb: reads the path named by argv[1] and reports what it saw.
//!
//! The confinement proof's instrument: run from a resource it can read
//! `/res/...`; any path outside `/res` does not exist in its namespace, so
//! the read fails honestly (exit 1, the error on stderr) — there is no
//! ambient filesystem to fall back on.

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: probe PATH");
        std::process::exit(2);
    };
    match std::fs::read(&path) {
        Ok(bytes) => println!("ok {} bytes", bytes.len()),
        Err(err) => {
            eprintln!("probe: {path}: {err}");
            std::process::exit(1);
        }
    }
}
