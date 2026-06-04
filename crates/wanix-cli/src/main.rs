//! Native CLI and demos for the Rust-native Wanix port.

fn main() {
    let exit_code = match wanix_cli::run_native_process(std::env::args_os().skip(1)) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error}");
            error.exit_code()
        }
    };
    std::process::exit(exit_code);
}
