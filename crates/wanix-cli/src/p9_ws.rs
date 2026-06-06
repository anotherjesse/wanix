mod command;
mod connection;
mod runtime;

pub(super) use command::parse_p9_ws_command;
pub(super) use connection::{P9WsConnectionError, serve_websocket_connection};
pub(super) use runtime::run_p9_ws_streaming;

#[cfg(test)]
use command::P9WsCommand;
#[cfg(test)]
use runtime::{p9_ws_listening_message, run_p9_ws_with_listener};

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tungstenite::{Message, WebSocket, connect};
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
    fn p9_ws_streaming_reports_bind_errors() {
        let command = P9WsCommand {
            root_path: PathBuf::from("."),
            addr: "127.0.0.1:bad-port".to_owned(),
            once: true,
        };
        let mut stderr = Vec::new();

        let error = run_p9_ws_streaming(command, &mut stderr).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("failed to bind p9-ws address"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn p9_ws_listening_message_uses_websocket_scheme() {
        let addr = "127.0.0.1:4711".parse().unwrap();

        assert_eq!(
            p9_ws_listening_message(addr),
            "wanix-rust p9-ws: listening on ws://127.0.0.1:4711/\n"
        );
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
