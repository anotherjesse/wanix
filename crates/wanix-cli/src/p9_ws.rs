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
    let mut parts = P9WsCommandParts::default();
    let mut i = 0;
    while i < args.len() {
        let Some(option) = P9WsOption::from_arg(&args[i]) else {
            return Err(CliError::usage(format!(
                "unexpected p9-ws argument: {}",
                args[i].to_string_lossy()
            )));
        };
        match option.value_name() {
            Some(value_name) => {
                i += 1;
                let value = args.get(i).ok_or_else(|| {
                    CliError::usage(format!("{} expects {value_name}", option.label()))
                })?;
                parts.apply_value(option, value)?;
            }
            None => parts.apply_flag(option)?,
        }
        i += 1;
    }

    parts.finish()
}

#[derive(Default)]
struct P9WsCommandParts {
    root_path: Option<PathBuf>,
    addr: Option<String>,
    once: bool,
}

impl P9WsCommandParts {
    fn apply_value(&mut self, option: P9WsOption, value: &OsString) -> Result<(), CliError> {
        match option {
            P9WsOption::Root => {
                set_single_path(&mut self.root_path, value, "p9-ws accepts only one --root")
            }
            P9WsOption::Addr => {
                if self.addr.is_some() {
                    return Err(CliError::usage("p9-ws accepts only one --addr"));
                }
                self.addr = Some(value.to_string_lossy().into_owned());
                Ok(())
            }
            P9WsOption::Once => Ok(()),
        }
    }

    fn apply_flag(&mut self, option: P9WsOption) -> Result<(), CliError> {
        match option {
            P9WsOption::Once => {
                if self.once {
                    return Err(CliError::usage("p9-ws accepts only one --once"));
                }
                self.once = true;
            }
            P9WsOption::Root | P9WsOption::Addr => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<P9WsCommand, CliError> {
        let root_path = self
            .root_path
            .ok_or_else(|| CliError::usage("p9-ws requires --root DIR"))?;
        let addr = self
            .addr
            .ok_or_else(|| CliError::usage("p9-ws requires --addr HOST:PORT"))?;
        Ok(P9WsCommand {
            root_path,
            addr,
            once: self.once,
        })
    }
}

#[derive(Clone, Copy)]
enum P9WsOption {
    Root,
    Addr,
    Once,
}

impl P9WsOption {
    fn from_arg(arg: &OsString) -> Option<Self> {
        match arg.to_str()? {
            "--root" => Some(Self::Root),
            "--addr" => Some(Self::Addr),
            "--once" => Some(Self::Once),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Root => "p9-ws --root",
            Self::Addr => "p9-ws --addr",
            Self::Once => "p9-ws --once",
        }
    }

    fn value_name(self) -> Option<&'static str> {
        match self {
            Self::Root => Some("DIR"),
            Self::Addr => Some("HOST:PORT"),
            Self::Once => None,
        }
    }
}

fn set_single_path(
    target: &mut Option<PathBuf>,
    value: &OsString,
    duplicate_message: &str,
) -> Result<(), CliError> {
    if target.is_some() {
        return Err(CliError::usage(duplicate_message));
    }
    *target = Some(PathBuf::from(value));
    Ok(())
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
        let Some(message) = read_websocket_message(&mut socket, frames.buffered_len())? else {
            return Ok(());
        };
        if !handle_websocket_message(&mut server, &mut frames, &mut socket, message)? {
            return Ok(());
        }
    }
}

fn read_websocket_message(
    socket: &mut WebSocket<TcpStream>,
    buffered_len: usize,
) -> Result<Option<Message>, P9WsConnectionError> {
    match socket.read() {
        Ok(message) => Ok(Some(message)),
        Err(WsError::ConnectionClosed) if buffered_len == 0 => Ok(None),
        Err(WsError::ConnectionClosed) => Err(P9WsConnectionError::TruncatedFrame { buffered_len }),
        Err(error) => Err(P9WsConnectionError::WebSocket(error)),
    }
}

fn handle_websocket_message(
    server: &mut P9Server,
    frames: &mut P9FrameBuffer,
    socket: &mut WebSocket<TcpStream>,
    message: Message,
) -> Result<bool, P9WsConnectionError> {
    match message {
        Message::Binary(bytes) => handle_binary_websocket_message(server, frames, socket, &bytes),
        Message::Close(_) => close_websocket_connection(frames),
        Message::Ping(bytes) => {
            socket.send(Message::Pong(bytes))?;
            Ok(true)
        }
        Message::Text(_) | Message::Pong(_) | Message::Frame(_) => Ok(true),
    }
}

fn handle_binary_websocket_message(
    server: &mut P9Server,
    frames: &mut P9FrameBuffer,
    socket: &mut WebSocket<TcpStream>,
    bytes: &[u8],
) -> Result<bool, P9WsConnectionError> {
    for request in frames.push(bytes)? {
        let response = server.handle_frame(&request)?;
        let response_bytes = response.encode()?;
        socket.send(Message::binary(response_bytes))?;
    }
    Ok(true)
}

fn close_websocket_connection(frames: &P9FrameBuffer) -> Result<bool, P9WsConnectionError> {
    if frames.buffered_len() != 0 {
        return Err(P9WsConnectionError::TruncatedFrame {
            buffered_len: frames.buffered_len(),
        });
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::io::{self, Read, Write};
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
    fn p9_ws_connection_errors_have_stable_display_text() {
        assert_eq!(
            P9WsConnectionError::Handshake("bad key".to_owned()).to_string(),
            "websocket handshake failed: bad key"
        );
        assert!(
            P9WsConnectionError::WebSocket(WsError::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pipe closed"
            )))
            .to_string()
            .starts_with("websocket I/O failed:"),
        );
        assert_eq!(
            P9WsConnectionError::Protocol(P9Error::InvalidUtf8).to_string(),
            "9P protocol error: 9P string is not valid UTF-8"
        );
        assert_eq!(
            P9WsConnectionError::Server(wanix_9p::Wanix9pError::InvalidPath("../x".to_owned()))
                .to_string(),
            "9P server error: invalid 9P walk path: ../x"
        );
        assert_eq!(
            P9WsConnectionError::TruncatedFrame { buffered_len: 7 }.to_string(),
            "websocket closed with 7 buffered 9P bytes"
        );
    }

    #[test]
    fn p9_ws_connection_errors_report_sources_for_wrapped_errors() {
        assert!(Error::source(&P9WsConnectionError::Handshake("bad key".to_owned())).is_none());
        assert!(
            Error::source(&P9WsConnectionError::WebSocket(WsError::Io(
                io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed")
            )))
            .is_some()
        );
        assert!(Error::source(&P9WsConnectionError::Protocol(P9Error::InvalidUtf8)).is_some());
        assert!(
            Error::source(&P9WsConnectionError::Server(
                wanix_9p::Wanix9pError::InvalidPath("../x".to_owned())
            ))
            .is_some()
        );
        assert!(Error::source(&P9WsConnectionError::TruncatedFrame { buffered_len: 7 }).is_none());
    }

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
    fn parse_p9_ws_preserves_option_errors() {
        let cases = [
            (vec![OsString::from("--root")], "p9-ws --root expects DIR"),
            (
                vec![OsString::from("--addr")],
                "p9-ws --addr expects HOST:PORT",
            ),
            (
                vec![
                    OsString::from("--root"),
                    OsString::from("."),
                    OsString::from("--root"),
                    OsString::from("."),
                ],
                "p9-ws accepts only one --root",
            ),
            (
                vec![
                    OsString::from("--addr"),
                    OsString::from("127.0.0.1:0"),
                    OsString::from("--addr"),
                    OsString::from("127.0.0.1:1"),
                ],
                "p9-ws accepts only one --addr",
            ),
            (
                vec![OsString::from("--once"), OsString::from("--once")],
                "p9-ws accepts only one --once",
            ),
            (
                vec![OsString::from("--bad")],
                "unexpected p9-ws argument: --bad",
            ),
        ];

        for (args, expected) in cases {
            let error = parse_p9_ws_command(&args).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{error} did not contain {expected}"
            );
        }
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
