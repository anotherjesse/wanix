mod command;
mod grant;
mod runtime;

pub(super) use command::{P9ListenCommand, parse_p9_listen_command};
pub(super) use runtime::run_p9_listen_streaming;

#[cfg(test)]
use runtime::{p9_listen_startup_message, run_p9_listen_with_listener};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLOPEN, P9_RREAD, P9_RVERSION, P9_RWALK, P9_VERSION_9P2000_L,
        P9Frame, P9FrameBuffer, p9_decode_rgetattr, p9_decode_rread, p9_tattach, p9_tgetattr,
        p9_tlopen, p9_tread, p9_tversion, p9_twalk,
    };

    use super::*;

    #[test]
    fn p9_listen_streaming_reports_bind_errors() {
        let command = P9ListenCommand {
            root_path: PathBuf::from("."),
            addr: "127.0.0.1:bad-port".to_owned(),
            once: true,
            peer: None,
            grants: Vec::new(),
        };
        let mut stderr = Vec::new();

        let error = run_p9_listen_streaming(command, &mut stderr).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(
            error
                .to_string()
                .contains("failed to bind p9-listen address")
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn p9_listen_startup_message_reports_bound_address() {
        let addr = "127.0.0.1:4712".parse().unwrap();

        assert_eq!(
            p9_listen_startup_message(addr),
            "wanix-rust p9-listen: listening on 127.0.0.1:4712\n"
        );
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
            peer: None,
            grants: Vec::new(),
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
