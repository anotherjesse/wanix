//! Native CLI and demos for the Rust-native Wanix port.

fn main() {
    let args = std::env::args_os().skip(1);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let exit_code =
        match wanix_cli::run_with_process_io(args, stdin.lock(), stdout.lock(), stderr.lock()) {
            Ok(exit_code) => exit_code,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(error.exit_code());
            }
        };
    std::process::exit(exit_code);
}
