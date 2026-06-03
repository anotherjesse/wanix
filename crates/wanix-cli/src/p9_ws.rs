use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use tungstenite::{Error as WsError, Message, WebSocket, accept};
use wanix_9p::P9Server;
use wanix_fs::{FileSystem, LocalFs};
use wanix_protocol::{P9Error, P9FrameBuffer};

use crate::{CliError, write_process_output};

#[derive(Debug)]
pub(super) enum P9WsConnectionError {
    Handshake(String),
    WebSocket(WsError),
    Protocol(P9Error),
    Server(wanix_9p::Wanix9pError),
    TruncatedFrame { buffered_len: usize },
}

impl fmt::Display for P9WsConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake(error) => write!(f, "websocket handshake failed: {error}"),
            Self::WebSocket(error) => write!(f, "websocket I/O failed: {error}"),
            Self::Protocol(error) => write!(f, "9P protocol error: {error}"),
            Self::Server(error) => write!(f, "9P server error: {error}"),
            Self::TruncatedFrame { buffered_len } => {
                write!(f, "websocket closed with {buffered_len} buffered 9P bytes")
            }
        }
    }
}

impl Error for P9WsConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WebSocket(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Server(error) => Some(error),
            Self::Handshake(_) | Self::TruncatedFrame { .. } => None,
        }
    }
}

impl From<WsError> for P9WsConnectionError {
    fn from(error: WsError) -> Self {
        Self::WebSocket(error)
    }
}

impl From<P9Error> for P9WsConnectionError {
    fn from(error: P9Error) -> Self {
        Self::Protocol(error)
    }
}

impl From<wanix_9p::Wanix9pError> for P9WsConnectionError {
    fn from(error: wanix_9p::Wanix9pError) -> Self {
        Self::Server(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P9WsCommand {
    root_path: PathBuf,
    addr: String,
    once: bool,
}

pub(super) fn parse_p9_ws_command(args: &[OsString]) -> Result<P9WsCommand, CliError> {
    let mut root_path = None;
    let mut addr = None;
    let mut once = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("p9-ws --root expects DIR"))?;
            if root_path.is_some() {
                return Err(CliError::usage("p9-ws accepts only one --root"));
            }
            root_path = Some(PathBuf::from(value));
            i += 1;
        } else if args[i] == "--addr" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("p9-ws --addr expects HOST:PORT"))?;
            if addr.is_some() {
                return Err(CliError::usage("p9-ws accepts only one --addr"));
            }
            addr = Some(value.to_string_lossy().into_owned());
            i += 1;
        } else if args[i] == "--once" {
            if once {
                return Err(CliError::usage("p9-ws accepts only one --once"));
            }
            once = true;
            i += 1;
        } else {
            return Err(CliError::usage(format!(
                "unexpected p9-ws argument: {}",
                args[i].to_string_lossy()
            )));
        }
    }

    let root_path = root_path.ok_or_else(|| CliError::usage("p9-ws requires --root DIR"))?;
    let addr = addr.ok_or_else(|| CliError::usage("p9-ws requires --addr HOST:PORT"))?;
    Ok(P9WsCommand {
        root_path,
        addr,
        once,
    })
}

pub(super) fn run_p9_ws_streaming(
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

fn run_p9_ws_with_listener(
    command: P9WsCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| CliError::new(format!("failed to inspect p9-ws address: {error}"), 1))?;
    let root = LocalFs::new(&command.root_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open p9-ws root {}: {error}",
                command.root_path.display()
            ),
            1,
        )
    })?;
    let root: Arc<dyn FileSystem> = Arc::new(root);

    write_process_output(
        process_stderr,
        "stderr",
        format!("wanix-rust p9-ws: listening on ws://{local_addr}/\n").as_bytes(),
    )?;

    if command.once {
        return serve_one_websocket(&listener, root, process_stderr);
    }

    loop {
        let exit_code = serve_one_websocket(&listener, Arc::clone(&root), process_stderr)?;
        if exit_code != 0 {
            write_process_output(
                process_stderr,
                "stderr",
                b"wanix-rust p9-ws: continuing after websocket error\n",
            )?;
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
    let result = match accept(stream) {
        Ok(socket) => serve_websocket_connection(root, socket),
        Err(error) => Err(P9WsConnectionError::Handshake(error.to_string())),
    };
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

pub(super) fn serve_websocket_connection(
    root: Arc<dyn FileSystem>,
    mut socket: WebSocket<TcpStream>,
) -> Result<(), P9WsConnectionError> {
    let mut server = P9Server::new(root);
    let mut frames = P9FrameBuffer::new();

    loop {
        let message = match socket.read() {
            Ok(message) => message,
            Err(WsError::ConnectionClosed) if frames.buffered_len() == 0 => return Ok(()),
            Err(WsError::ConnectionClosed) => {
                return Err(P9WsConnectionError::TruncatedFrame {
                    buffered_len: frames.buffered_len(),
                });
            }
            Err(error) => return Err(P9WsConnectionError::WebSocket(error)),
        };
        match message {
            Message::Binary(bytes) => {
                for request in frames.push(&bytes)? {
                    let response = server.handle_frame(&request)?;
                    let response_bytes = response.encode()?;
                    socket.send(Message::binary(response_bytes))?;
                }
            }
            Message::Close(_) => {
                if frames.buffered_len() != 0 {
                    return Err(P9WsConnectionError::TruncatedFrame {
                        buffered_len: frames.buffered_len(),
                    });
                }
                return Ok(());
            }
            Message::Ping(bytes) => socket.send(Message::Pong(bytes))?,
            Message::Text(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tungstenite::{Message, connect};
    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLOPEN, P9_RREAD, P9_RVERSION, P9_RWALK, P9_VERSION_9P2000_L,
        P9Frame, p9_decode_rgetattr, p9_decode_rread, p9_tattach, p9_tgetattr, p9_tlopen, p9_tread,
        p9_tversion, p9_twalk,
    };

    use super::*;

    #[test]
    fn parse_p9_ws_requires_root_and_addr() {
        let error =
            parse_p9_ws_command(&[OsString::from("--root"), OsString::from(".")]).unwrap_err();
        assert!(error.to_string().contains("requires --addr HOST:PORT"));

        let command = parse_p9_ws_command(&[
            OsString::from("--root"),
            OsString::from("."),
            OsString::from("--addr"),
            OsString::from("127.0.0.1:0"),
            OsString::from("--once"),
        ])
        .unwrap();

        assert_eq!(command.root_path, PathBuf::from("."));
        assert_eq!(command.addr, "127.0.0.1:0");
        assert!(command.once);
    }

    #[test]
    fn p9_ws_once_serves_host_file_over_binary_websocket() {
        let root = temp_dir("wanix-cli-p9-ws");
        fs::write(root.join("hello.txt"), b"hello ws").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = P9WsCommand {
            root_path: root,
            addr: addr.to_string(),
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_p9_ws_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let mut socket = connect(format!("ws://{addr}/")).unwrap().0;
        let requests = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tgetattr(4, 2, u64::MAX),
            p9_tlopen(5, 2, 0),
            p9_tread(6, 2, 0, 8),
        ]);
        socket.send(Message::binary(requests)).unwrap();

        let frames = read_binary_frames(&mut socket, 6);
        socket.close(None).unwrap();
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(
            stderr.contains("wanix-rust p9-ws: listening on ws://127.0.0.1:"),
            "{stderr}"
        );
        assert_eq!(
            frame_types(&frames),
            [
                P9_RVERSION,
                P9_RATTACH,
                P9_RWALK,
                P9_RGETATTR,
                P9_RLOPEN,
                P9_RREAD
            ]
        );
        let attr = p9_decode_rgetattr(&frames[3]).unwrap();
        assert_eq!(attr.size, 8);
        assert_eq!(attr.mode & 0o170000, 0o100000);
        assert_eq!(p9_decode_rread(&frames[5]).unwrap(), b"hello ws");
    }

    fn read_binary_frames<S: Read + Write>(
        socket: &mut WebSocket<S>,
        count: usize,
    ) -> Vec<P9Frame> {
        let mut frames = Vec::new();
        while frames.len() < count {
            if let Message::Binary(bytes) = socket.read().unwrap() {
                frames.push(P9Frame::decode(&bytes).unwrap());
            }
        }
        frames
    }

    fn request_stream<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
        let mut stream = Vec::new();
        for frame in frames {
            stream.extend_from_slice(&frame.encode().unwrap());
        }
        stream
    }

    fn frame_types(frames: &[P9Frame]) -> Vec<u8> {
        frames.iter().map(P9Frame::message_type).collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
