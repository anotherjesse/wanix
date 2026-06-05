use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use tungstenite::accept;
use wanix_fs::{FileSystem, LocalFs};

use crate::{CliError, write_process_output};

use super::command::P9WsCommand;
use super::connection::{P9WsConnectionError, serve_websocket_connection};

pub(crate) fn run_p9_ws_streaming(
    command: P9WsCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let listener = TcpListener::bind(&command.addr).map_err(|error| {
        CliError::new(
            format!("failed to bind p9-ws address {}: {error}", command.addr),
            1,
        )
    })?;
    run_p9_ws_with_listener(command, listener, process_stderr)
}

pub(super) fn run_p9_ws_with_listener(
    command: P9WsCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let runtime = P9WsRuntime::new(command, listener)?;
    runtime.write_listening(process_stderr)?;
    runtime.serve(process_stderr)
}

struct P9WsRuntime {
    listener: TcpListener,
    local_addr: SocketAddr,
    root: Arc<dyn FileSystem>,
    once: bool,
}

impl P9WsRuntime {
    fn new(command: P9WsCommand, listener: TcpListener) -> Result<Self, CliError> {
        let local_addr = listener.local_addr().map_err(|error| {
            CliError::new(format!("failed to inspect p9-ws address: {error}"), 1)
        })?;
        let root = LocalFs::new(&command.root_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to open p9-ws root {}: {error}",
                    command.root_path.display()
                ),
                1,
            )
        })?;
        Ok(Self {
            listener,
            local_addr,
            root: Arc::new(root),
            once: command.once,
        })
    }

    fn write_listening(&self, process_stderr: &mut dyn Write) -> Result<(), CliError> {
        write_process_output(
            process_stderr,
            "stderr",
            p9_ws_listening_message(self.local_addr).as_bytes(),
        )
    }

    fn serve(self, process_stderr: &mut dyn Write) -> Result<i32, CliError> {
        if self.once {
            return serve_one_websocket(&self.listener, self.root, process_stderr);
        }
        self.serve_loop(process_stderr)
    }

    fn serve_loop(self, process_stderr: &mut dyn Write) -> Result<i32, CliError> {
        loop {
            let exit_code =
                serve_one_websocket(&self.listener, Arc::clone(&self.root), process_stderr)?;
            if exit_code != 0 {
                write_continue_after_error(process_stderr)?;
            }
        }
    }
}

fn serve_one_websocket(
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let (stream, peer_addr) = listener
        .accept()
        .map_err(|error| CliError::new(format!("p9-ws accept failed: {error}"), 1))?;
    let result = accept(stream)
        .map_err(|error| P9WsConnectionError::Handshake(error.to_string()))
        .and_then(|socket| serve_websocket_connection(root, socket));
    report_connection_result(peer_addr, result, process_stderr)
}

fn report_connection_result(
    peer_addr: SocketAddr,
    result: Result<(), P9WsConnectionError>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    match result {
        Ok(()) => Ok(0),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust p9-ws: connection {peer_addr} failed: {error}\n").as_bytes(),
            )?;
            Ok(1)
        }
    }
}

fn write_continue_after_error(process_stderr: &mut dyn Write) -> Result<(), CliError> {
    write_process_output(
        process_stderr,
        "stderr",
        b"wanix-rust p9-ws: continuing after websocket error\n",
    )
}

pub(in crate::p9_ws) fn p9_ws_listening_message(local_addr: SocketAddr) -> String {
    format!("wanix-rust p9-ws: listening on ws://{local_addr}/\n")
}
