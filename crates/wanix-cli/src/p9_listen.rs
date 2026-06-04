use std::ffi::OsString;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use wanix_9p::{P9Server, P9TransportError};
use wanix_fs::{FileSystem, LocalFs};

use crate::{CliError, write_process_output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P9ListenCommand {
    root_path: PathBuf,
    addr: String,
    once: bool,
}

pub(super) fn parse_p9_listen_command(args: &[OsString]) -> Result<P9ListenCommand, CliError> {
    let mut parts = P9ListenCommandParts::default();
    let mut i = 0;
    while i < args.len() {
        i = parse_p9_listen_arg(args, i, &mut parts)?;
    }

    parts.finish()
}

fn parse_p9_listen_arg(
    args: &[OsString],
    index: usize,
    parts: &mut P9ListenCommandParts,
) -> Result<usize, CliError> {
    match P9ListenArg::from_arg(&args[index]) {
        Some(P9ListenArg::Value(option)) => apply_p9_listen_value_arg(args, index, parts, option),
        Some(P9ListenArg::Flag(option)) => {
            parts.apply_flag(option)?;
            Ok(index + 1)
        }
        None => Err(CliError::usage(format!(
            "unexpected p9-listen argument: {}",
            args[index].to_string_lossy()
        ))),
    }
}

fn apply_p9_listen_value_arg(
    args: &[OsString],
    option_index: usize,
    parts: &mut P9ListenCommandParts,
    option: P9ListenValueOption,
) -> Result<usize, CliError> {
    let value_index = option_index + 1;
    let value = args.get(value_index).ok_or_else(|| {
        CliError::usage(format!(
            "{} expects {}",
            option.label(),
            option.value_name()
        ))
    })?;
    parts.apply_value(option, value)?;
    Ok(value_index + 1)
}

#[derive(Default)]
struct P9ListenCommandParts {
    root_path: Option<PathBuf>,
    addr: Option<String>,
    once: bool,
}

impl P9ListenCommandParts {
    fn apply_value(
        &mut self,
        option: P9ListenValueOption,
        value: &OsString,
    ) -> Result<(), CliError> {
        match option {
            P9ListenValueOption::Root => set_single_path(
                &mut self.root_path,
                value,
                "p9-listen accepts only one --root",
            ),
            P9ListenValueOption::Addr => {
                if self.addr.is_some() {
                    return Err(CliError::usage("p9-listen accepts only one --addr"));
                }
                self.addr = Some(value.to_string_lossy().into_owned());
                Ok(())
            }
        }
    }

    fn apply_flag(&mut self, option: P9ListenFlagOption) -> Result<(), CliError> {
        match option {
            P9ListenFlagOption::Once => {
                if self.once {
                    return Err(CliError::usage("p9-listen accepts only one --once"));
                }
                self.once = true;
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<P9ListenCommand, CliError> {
        let root_path = self
            .root_path
            .ok_or_else(|| CliError::usage("p9-listen requires --root DIR"))?;
        let addr = self
            .addr
            .ok_or_else(|| CliError::usage("p9-listen requires --addr HOST:PORT"))?;
        Ok(P9ListenCommand {
            root_path,
            addr,
            once: self.once,
        })
    }
}

#[derive(Clone, Copy)]
enum P9ListenArg {
    Value(P9ListenValueOption),
    Flag(P9ListenFlagOption),
}

impl P9ListenArg {
    fn from_arg(arg: &OsString) -> Option<Self> {
        let arg = arg.to_str()?;
        P9_LISTEN_VALUE_OPTIONS
            .iter()
            .find_map(|(name, option)| (*name == arg).then_some(Self::Value(*option)))
            .or_else(|| {
                P9_LISTEN_FLAG_OPTIONS
                    .iter()
                    .find_map(|(name, option)| (*name == arg).then_some(Self::Flag(*option)))
            })
    }
}

const P9_LISTEN_VALUE_OPTIONS: &[(&str, P9ListenValueOption)] = &[
    ("--root", P9ListenValueOption::Root),
    ("--addr", P9ListenValueOption::Addr),
];

const P9_LISTEN_FLAG_OPTIONS: &[(&str, P9ListenFlagOption)] =
    &[("--once", P9ListenFlagOption::Once)];

#[derive(Clone, Copy)]
enum P9ListenValueOption {
    Root,
    Addr,
}

impl P9ListenValueOption {
    fn label(self) -> &'static str {
        match self {
            Self::Root => "p9-listen --root",
            Self::Addr => "p9-listen --addr",
        }
    }

    fn value_name(self) -> &'static str {
        match self {
            Self::Root => "DIR",
            Self::Addr => "HOST:PORT",
        }
    }
}

#[derive(Clone, Copy)]
enum P9ListenFlagOption {
    Once,
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

pub(super) fn run_p9_listen_streaming(
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

fn run_p9_listen_with_listener(
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
        format!("wanix-rust p9-listen: listening on {local_addr}\n").as_bytes(),
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
        if exit_code != 0 {
            write_process_output(
                process_stderr,
                "stderr",
                b"wanix-rust p9-listen: continuing after connection error\n",
            )?;
        }
    }
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLOPEN, P9_RREAD, P9_RVERSION, P9_RWALK, P9_VERSION_9P2000_L,
        P9Frame, P9FrameBuffer, p9_decode_rgetattr, p9_decode_rread, p9_tattach, p9_tgetattr,
        p9_tlopen, p9_tread, p9_tversion, p9_twalk,
    };

    use super::*;

    #[test]
    fn parse_p9_listen_requires_root_and_addr() {
        let error =
            parse_p9_listen_command(&[OsString::from("--root"), OsString::from(".")]).unwrap_err();
        assert!(error.to_string().contains("requires --addr HOST:PORT"));

        let command = parse_p9_listen_command(&[
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
    fn parse_p9_listen_reports_missing_option_values() {
        for (option, expected) in [
            ("--root", "p9-listen --root expects DIR"),
            ("--addr", "p9-listen --addr expects HOST:PORT"),
        ] {
            let error = parse_p9_listen_command(&[OsString::from(option)]).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{option} produced {error}"
            );
        }
    }

    #[test]
    fn p9_listen_once_serves_host_file_over_tcp() {
        let root = temp_dir("wanix-cli-p9-listen");
        fs::write(root.join("hello.txt"), b"hello tcp").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let command = P9ListenCommand {
            root_path: root.clone(),
            addr: addr.to_string(),
            once: true,
        };

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            let exit_code = run_p9_listen_with_listener(command, listener, &mut stderr).unwrap();
            (exit_code, stderr)
        });

        let output = request_responses(addr);
        let (exit_code, stderr) = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(
            stderr.contains("wanix-rust p9-listen: listening on 127.0.0.1:"),
            "{stderr}"
        );
        let frames = decode_response_stream(&output);
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
        assert_eq!(attr.size, 9);
        assert_eq!(attr.mode & 0o170000, 0o100000);
        assert_eq!(p9_decode_rread(&frames[5]).unwrap(), b"hello tcp");
    }

    fn request_responses(addr: std::net::SocketAddr) -> Vec<u8> {
        let mut stream = TcpStream::connect(addr).unwrap();
        let input = request_stream([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tgetattr(4, 2, u64::MAX),
            p9_tlopen(5, 2, 0),
            p9_tread(6, 2, 0, 9),
        ]);
        stream.write_all(&input).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();

        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        output
    }

    fn request_stream<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
        let mut stream = Vec::new();
        for frame in frames {
            stream.extend_from_slice(&frame.encode().unwrap());
        }
        stream
    }

    fn decode_response_stream(bytes: &[u8]) -> Vec<P9Frame> {
        let mut buffer = P9FrameBuffer::new();
        let frames = buffer.push(bytes).unwrap();
        assert_eq!(buffer.buffered_len(), 0);
        frames
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
