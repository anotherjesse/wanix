//! Native CLI and demos for the Rust-native Wanix port.

use std::io::Write;

fn main() {
    let output = match wanix_cli::run(std::env::args_os().skip(1)) {
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
