//! Native CLI and demos for the Rust-native Wanix port.

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let _raw_mode = if wanix_cli::command_requests_raw_tty(&args) {
        match wanix_cli::NativeRawTerminalMode::enter_stdin_if_tty() {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(error.exit_code());
            }
        }
    } else {
        None
    };
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
