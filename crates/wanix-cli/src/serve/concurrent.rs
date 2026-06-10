use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use crate::{CliError, write_process_output};

use super::connection::serve_connection;
use super::roots::ServeRoots;

const CONNECTION_POLL_SLEEP: Duration = Duration::from_millis(10);

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
    let mut connections = ConcurrentConnections::new(roots, connection_limit);

    loop {
        if connections.accept_or_wait(&listener, process_stderr)? {
            break;
        }
    }

    connections.finish(process_stderr)
}

struct ConcurrentConnections {
    roots: Arc<ServeRoots>,
    error_sender: mpsc::Sender<String>,
    error_receiver: mpsc::Receiver<String>,
    accepted: usize,
    handles: Vec<thread::JoinHandle<()>>,
    had_error: bool,
    connection_limit: Option<usize>,
}

impl ConcurrentConnections {
    fn new(roots: ServeRoots, connection_limit: Option<usize>) -> Self {
        let (error_sender, error_receiver) = mpsc::channel::<String>();
        Self {
            roots: Arc::new(roots),
            error_sender,
            error_receiver,
            accepted: 0,
            handles: Vec::new(),
            had_error: false,
            connection_limit,
        }
    }

    fn accept_or_wait(
        &mut self,
        listener: &TcpListener,
        process_stderr: &mut dyn Write,
    ) -> Result<bool, CliError> {
        let connection = accept_next_connection(listener)?;
        self.handle_connection_poll(connection, process_stderr)?;
        self.drain_errors(process_stderr)?;
        Ok(self.reached_limit())
    }

    fn handle_connection_poll(
        &mut self,
        connection: Option<(TcpStream, SocketAddr)>,
        process_stderr: &mut dyn Write,
    ) -> Result<(), CliError> {
        match connection {
            Some((stream, peer_addr)) => self.spawn_connection(stream, peer_addr, process_stderr),
            None => self.wait_for_connection(process_stderr),
        }
    }

    fn wait_for_connection(&mut self, process_stderr: &mut dyn Write) -> Result<(), CliError> {
        self.drain_errors(process_stderr)?;
        thread::sleep(CONNECTION_POLL_SLEEP);
        Ok(())
    }

    fn reached_limit(&self) -> bool {
        self.connection_limit
            .is_some_and(|limit| self.accepted >= limit)
    }

    fn spawn_connection(
        &mut self,
        stream: TcpStream,
        peer_addr: SocketAddr,
        process_stderr: &mut dyn Write,
    ) -> Result<(), CliError> {
        if let Err(error) = stream.set_nonblocking(false) {
            self.had_error = true;
            write_process_output(
                process_stderr,
                "stderr",
                format!(
                    "wanix serve: connection {peer_addr} failed: \
                     could not configure blocking mode: {error}\n"
                )
                .as_bytes(),
            )?;
            return Ok(());
        }

        self.accepted += 1;
        let connection_roots = Arc::clone(&self.roots);
        let connection_errors = self.error_sender.clone();
        let handle = thread::spawn(move || {
            if let Err(error) = serve_connection(&connection_roots, stream, peer_addr) {
                let _ = connection_errors.send(format!(
                    "wanix serve: connection {peer_addr} failed: {error}\n"
                ));
            }
        });
        if self.connection_limit.is_some() {
            self.handles.push(handle);
        }
        Ok(())
    }

    fn finish(mut self, process_stderr: &mut dyn Write) -> Result<i32, CliError> {
        for handle in std::mem::take(&mut self.handles) {
            if handle.join().is_err() {
                self.had_error = true;
                write_process_output(
                    process_stderr,
                    "stderr",
                    b"wanix serve: connection worker panicked\n",
                )?;
            }
        }
        self.drain_errors(process_stderr)?;
        Ok(i32::from(self.had_error))
    }

    fn drain_errors(&mut self, process_stderr: &mut dyn Write) -> Result<(), CliError> {
        if drain_connection_errors(&self.error_receiver, process_stderr)? {
            self.had_error = true;
        }
        Ok(())
    }
}

fn accept_next_connection(
    listener: &TcpListener,
) -> Result<Option<(TcpStream, SocketAddr)>, CliError> {
    match listener.accept() {
        Ok(connection) => Ok(Some(connection)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(CliError::new(format!("serve accept failed: {error}"), 1)),
    }
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
            b"wanix serve: continuing after connection error\n",
        )?;
    }
    Ok(had_error)
}
