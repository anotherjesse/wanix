//! Native CLI and demos for the Rust-native Wanix port.

use std::io::Write;

fn main() {
    let args = std::env::args_os().skip(1);
    let stdin = std::io::stdin();
    let output = match wanix_cli::run_with_process_stdin(args, stdin.lock()) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(error.exit_code());
        }
    };

    if let Err(error) = std::io::stdout().write_all(output.stdout()) {
        eprintln!("wanix-rust: failed to write stdout: {error}");
        std::process::exit(1);
    }
    if let Err(error) = std::io::stderr().write_all(output.stderr()) {
        eprintln!("wanix-rust: failed to write stderr: {error}");
        std::process::exit(1);
    }
    std::process::exit(output.exit_code());
}
