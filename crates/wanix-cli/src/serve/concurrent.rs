use std::io::{self, Write};
use std::net::TcpListener;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use crate::{CliError, write_process_output};

use super::connection::serve_connection;
use super::roots::ServeRoots;

pub(super) fn serve_concurrent_connections(
    listener: TcpListener,
    roots: ServeRoots,
    process_stderr: &mut dyn Write,
    connection_limit: Option<usize>,
) -> Result<i32, CliError> {
    listener.set_nonblocking(true).map_err(|error| {
        CliError::new(
            format!("failed to configure serve listener as nonblocking: {error}"),
            1,
        )
    })?;
    let roots = Arc::new(roots);
    let (error_sender, error_receiver) = mpsc::channel::<String>();
    let mut accepted = 0usize;
    let mut handles = Vec::new();
    let mut had_error = false;

    loop {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    had_error = true;
                    write_process_output(
                        process_stderr,
                        "stderr",
                        format!(
                            "wanix-rust serve: connection {peer_addr} failed: \
                             could not configure blocking mode: {error}\n"
                        )
                        .as_bytes(),
                    )?;
                    continue;
                }
                accepted += 1;
                let connection_roots = Arc::clone(&roots);
                let connection_errors = error_sender.clone();
                let handle = thread::spawn(move || {
                    if let Err(error) = serve_connection(&connection_roots, stream, peer_addr) {
                        let _ = connection_errors.send(format!(
                            "wanix-rust serve: connection {peer_addr} failed: {error}\n"
                        ));
                    }
                });
                if connection_limit.is_some() {
                    handles.push(handle);
                }
                if connection_limit.is_some_and(|limit| accepted >= limit) {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                had_error |= drain_connection_errors(&error_receiver, process_stderr)?;
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(CliError::new(format!("serve accept failed: {error}"), 1)),
        }
        had_error |= drain_connection_errors(&error_receiver, process_stderr)?;
    }

    drop(error_sender);
    for handle in handles {
        if handle.join().is_err() {
            had_error = true;
            write_process_output(
                process_stderr,
                "stderr",
                b"wanix-rust serve: connection worker panicked\n",
            )?;
        }
    }
    had_error |= drain_connection_errors(&error_receiver, process_stderr)?;
    Ok(i32::from(had_error))
}

fn drain_connection_errors(
    error_receiver: &mpsc::Receiver<String>,
    process_stderr: &mut dyn Write,
) -> Result<bool, CliError> {
    let mut had_error = false;
    while let Ok(message) = error_receiver.try_recv() {
        had_error = true;
        write_process_output(process_stderr, "stderr", message.as_bytes())?;
        write_process_output(
            process_stderr,
            "stderr",
            b"wanix-rust serve: continuing after connection error\n",
        )?;
    }
    Ok(had_error)
}
