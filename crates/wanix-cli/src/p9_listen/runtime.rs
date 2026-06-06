use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

use wanix_9p::{P9Server, P9TransportError};
use wanix_fs::{FileSystem, LocalFs};

use crate::{CliError, write_process_output};

use super::P9ListenCommand;

pub(crate) fn run_p9_listen_streaming(
    command: P9ListenCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let listener = TcpListener::bind(&command.addr).map_err(|error| {
        CliError::new(
            format!("failed to bind p9-listen address {}: {error}", command.addr),
            1,
        )
    })?;
    run_p9_listen_with_listener(command, listener, process_stderr)
}

pub(super) fn run_p9_listen_with_listener(
    command: P9ListenCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let root = p9_listen_root(&command)?;
    write_p9_listen_startup(&listener, process_stderr)?;
    serve_p9_listener(command.once, &listener, root, process_stderr)
}

fn p9_listen_root(command: &P9ListenCommand) -> Result<Arc<dyn FileSystem>, CliError> {
    let root = LocalFs::new(&command.root_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open p9-listen root {}: {error}",
                command.root_path.display()
            ),
            1,
        )
    })?;
    Ok(Arc::new(root))
}

fn write_p9_listen_startup(
    listener: &TcpListener,
    process_stderr: &mut dyn Write,
) -> Result<(), CliError> {
    let local_addr = listener.local_addr().map_err(|error| {
        CliError::new(format!("failed to inspect p9-listen address: {error}"), 1)
    })?;
    write_process_output(
        process_stderr,
        "stderr",
        p9_listen_startup_message(local_addr).as_bytes(),
    )
}

fn serve_p9_listener(
    once: bool,
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    if once {
        return serve_one_connection(listener, root, process_stderr);
    }

    serve_p9_listener_loop(listener, root, process_stderr)
}

fn serve_p9_listener_loop(
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    loop {
        let exit_code = serve_one_connection(listener, Arc::clone(&root), process_stderr)?;
        write_p9_listen_continue_after_error(exit_code, process_stderr)?;
    }
}

fn write_p9_listen_continue_after_error(
    exit_code: i32,
    process_stderr: &mut dyn Write,
) -> Result<(), CliError> {
    if exit_code == 0 {
        return Ok(());
    }

    write_process_output(
        process_stderr,
        "stderr",
        b"wanix-rust p9-listen: continuing after connection error\n",
    )
}

fn serve_one_connection(
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let (stream, peer_addr) = listener
        .accept()
        .map_err(|error| CliError::new(format!("p9-listen accept failed: {error}"), 1))?;
    match serve_stream_connection(root, stream) {
        Ok(_) => Ok(0),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust p9-listen: connection {peer_addr} failed: {error}\n")
                    .as_bytes(),
            )?;
            Ok(1)
        }
    }
}

fn serve_stream_connection(
    root: Arc<dyn FileSystem>,
    stream: TcpStream,
) -> Result<wanix_9p::P9TransportStats, P9TransportError> {
    let reader = stream.try_clone().map_err(P9TransportError::Io)?;
    let mut server = P9Server::new(root);
    server.serve_stream(reader, stream)
}

pub(super) fn p9_listen_startup_message(local_addr: SocketAddr) -> String {
    format!("wanix-rust p9-listen: listening on {local_addr}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p9_listen_loop_continue_message_is_only_written_after_connection_errors() {
        let mut stderr = Vec::new();

        write_p9_listen_continue_after_error(0, &mut stderr).unwrap();
        assert!(stderr.is_empty());

        write_p9_listen_continue_after_error(1, &mut stderr).unwrap();
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "wanix-rust p9-listen: continuing after connection error\n"
        );
    }
}
