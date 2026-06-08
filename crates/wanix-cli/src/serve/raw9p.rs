use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_9p::{AttachPolicy, P9Server, P9TransportError, P9TransportStats, PeerId};
use wanix_fs::FileSystem;

use crate::{CliError, write_process_output};

pub(in crate::serve) mod grant;
use grant::{GrantSpec, build_policy};

/// The verified peer and shared attach policy a policy-gated raw-9P server
/// consults at every `Tattach`.
///
/// The policy is shared, so a revoke applied between connections is observed by
/// the next connection's freshly built [`P9Server`].
#[derive(Clone)]
pub(in crate::serve) struct ServePolicy {
    peer: PeerId,
    policy: Arc<dyn AttachPolicy>,
}

/// Builds the shared attach policy when `--peer` is supplied; otherwise returns
/// `None` so the raw-9P door keeps the unauthenticated byte-for-byte behavior.
pub(in crate::serve) fn build_serve_policy(
    peer: Option<PeerId>,
    grants: Vec<GrantSpec>,
    root: &Arc<dyn FileSystem>,
) -> Option<ServePolicy> {
    let peer = peer?;
    let (_, policy) = build_policy(peer, grants, Arc::clone(root));
    Some(ServePolicy {
        peer,
        policy: Arc::new(policy),
    })
}

/// Binds the raw-9P TCP door on `addr`, spawns a dedicated blocking accept loop
/// thread serving `root` (capability-gated by `policy`), and returns the bound
/// address so discovery/status can advertise it.
///
/// The loop runs on its own thread because the concurrent HTTP poller has no
/// shutdown signal (see the design plan); the raw-9P door reuses the blocking
/// per-connection thread model the standalone `p9-listen` used. It always loops
/// (it is a long-lived service door for many `mount-*` clients); the thread is
/// detached and reaped on process exit. The thread logs to the live process
/// `stderr` directly, since the loop only runs in the live `wanix-rust` binary,
/// never in collected/captured mode.
///
/// # Errors
///
/// Returns an error when the address cannot be bound or inspected.
pub(in crate::serve) fn start_raw9p_door(
    addr: &str,
    root: Arc<dyn FileSystem>,
    policy: Option<ServePolicy>,
) -> Result<SocketAddr, CliError> {
    let listener = TcpListener::bind(addr).map_err(|error| {
        CliError::new(
            format!("failed to bind serve --p9 address {addr}: {error}"),
            1,
        )
    })?;
    let bound = listener.local_addr().map_err(|error| {
        CliError::new(format!("failed to inspect serve --p9 address: {error}"), 1)
    })?;
    thread::spawn(move || {
        let mut stderr = std::io::stderr();
        let _ = serve_raw9p_listener(false, &listener, root, policy.as_ref(), &mut stderr);
    });
    Ok(bound)
}

/// Accept/serve/error loop for the raw-9P door. Shared, transport-agnostic over
/// `root` + optional `policy`. With `once`, serves a single connection (used by
/// tests); otherwise loops, logging and continuing past per-connection errors.
///
/// # Errors
///
/// Returns an error when an `accept` or a diagnostic write fails.
pub(in crate::serve) fn serve_raw9p_listener(
    once: bool,
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    policy: Option<&ServePolicy>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    if once {
        return serve_one_connection(listener, root, policy, process_stderr);
    }
    loop {
        let exit_code = serve_one_connection(listener, Arc::clone(&root), policy, process_stderr)?;
        write_continue_after_error(exit_code, process_stderr)?;
    }
}

fn write_continue_after_error(
    exit_code: i32,
    process_stderr: &mut dyn Write,
) -> Result<(), CliError> {
    if exit_code == 0 {
        return Ok(());
    }
    write_process_output(
        process_stderr,
        "stderr",
        b"wanix-rust serve --p9: continuing after connection error\n",
    )
}

fn serve_one_connection(
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    policy: Option<&ServePolicy>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let (stream, peer_addr) = listener
        .accept()
        .map_err(|error| CliError::new(format!("serve --p9 accept failed: {error}"), 1))?;
    match serve_raw9p_connection(root, policy, stream) {
        Ok(_) => Ok(0),
        Err(error) => {
            write_process_output(
                process_stderr,
                "stderr",
                format!("wanix-rust serve --p9: connection {peer_addr} failed: {error}\n")
                    .as_bytes(),
            )?;
            Ok(1)
        }
    }
}

fn serve_raw9p_connection(
    root: Arc<dyn FileSystem>,
    policy: Option<&ServePolicy>,
    stream: TcpStream,
) -> Result<P9TransportStats, P9TransportError> {
    let reader = stream.try_clone().map_err(P9TransportError::Io)?;
    let mut server = match policy {
        Some(serve_policy) => {
            P9Server::with_policy(root, serve_policy.peer, Arc::clone(&serve_policy.policy))
        }
        None => P9Server::new(root),
    };
    server.serve_stream(reader, stream)
}

pub(in crate::serve) fn raw9p_startup_message(local_addr: SocketAddr) -> String {
    format!("wanix-rust serve: raw 9P listening on tcp://{local_addr}/\n")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use wanix_9p::{Grant, GrantTable, GrantTablePolicy};
    use wanix_fs::LocalFs;
    use wanix_protocol::{
        P9_RATTACH, P9_RLERROR, P9_RWRITE, P9_VERSION_9P2000_L, P9Frame, P9FrameBuffer,
        p9_decode_rlerror, p9_decode_rread, p9_tattach, p9_tlopen, p9_tread, p9_tversion, p9_twalk,
        p9_twrite,
    };
    use wanix_vfs::Rights;

    use super::*;

    const EACCES: u32 = 13;
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        dir.push(format!(
            "wanix-serve-p9-policy-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn capability_root() -> (std::path::PathBuf, Arc<dyn FileSystem>) {
        let root = temp_root();
        std::fs::create_dir_all(root.join("projects/foo")).unwrap();
        std::fs::write(root.join("projects/foo/file.txt"), b"granted").unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/readme.txt"), b"read me").unwrap();
        let fs: Arc<dyn FileSystem> = Arc::new(LocalFs::new(&root).unwrap());
        (root, fs)
    }

    /// Serves one connection over a real loopback TCP stream under `policy` and
    /// returns the decoded response frames for the driven request frames.
    fn serve_once_with_policy(
        root: Arc<dyn FileSystem>,
        policy: Option<ServePolicy>,
        request: Vec<u8>,
    ) -> Vec<P9Frame> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = serve_raw9p_connection(root, policy.as_ref(), stream);
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(&request).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        handle.join().unwrap();

        let mut buffer = P9FrameBuffer::new();
        buffer.push(&output).unwrap()
    }

    fn encode_frames<const N: usize>(frames: [P9Frame; N]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for frame in frames {
            bytes.extend_from_slice(&frame.encode().unwrap());
        }
        bytes
    }

    fn serve_policy(peer: PeerId, table: &GrantTable) -> ServePolicy {
        ServePolicy {
            peer,
            policy: Arc::new(GrantTablePolicy::new(table.clone())),
        }
    }

    #[test]
    fn continue_message_is_only_written_after_connection_errors() {
        let mut stderr = Vec::new();

        write_continue_after_error(0, &mut stderr).unwrap();
        assert!(stderr.is_empty());

        write_continue_after_error(1, &mut stderr).unwrap();
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "wanix-rust serve --p9: continuing after connection error\n"
        );
    }

    #[test]
    fn raw9p_startup_message_reports_bound_address() {
        let addr = "127.0.0.1:4712".parse().unwrap();
        assert_eq!(
            raw9p_startup_message(addr),
            "wanix-rust serve: raw 9P listening on tcp://127.0.0.1:4712/\n"
        );
    }

    #[test]
    fn raw9p_listener_serves_a_host_file_over_tcp() {
        let root = temp_root();
        std::fs::write(root.join("hello.txt"), b"hello tcp").unwrap();
        let fs: Arc<dyn FileSystem> = Arc::new(LocalFs::new(&root).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            serve_raw9p_listener(true, &listener, fs, None, &mut stderr).unwrap()
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        let request = encode_frames([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap(),
            p9_twalk(3, 1, 2, &["hello.txt"]).unwrap(),
            p9_tlopen(4, 2, 0),
            p9_tread(5, 2, 0, 9),
        ]);
        stream.write_all(&request).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        let exit_code = handle.join().unwrap();

        assert_eq!(exit_code, 0);
        let mut buffer = P9FrameBuffer::new();
        let frames = buffer.push(&output).unwrap();
        assert_eq!(frames[1].message_type(), P9_RATTACH);
        assert_eq!(
            p9_decode_rread(frames.last().unwrap()).unwrap(),
            b"hello tcp"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn authorized_peer_attaches_scoped_subtree_and_reads_it() {
        let (root, fs) = capability_root();
        let peer = PeerId::from_bytes([7u8; 32]);
        let table = GrantTable::new();
        table.grant(Grant::new(
            peer,
            "projects/foo",
            Arc::clone(&fs),
            "projects/foo",
            Rights::read_write(),
        ));

        let request = encode_frames([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "k", "projects/foo", 0).unwrap(),
            p9_twalk(3, 1, 2, &["file.txt"]).unwrap(),
            p9_tlopen(5, 2, 0),
            p9_tread(6, 2, 0, 7),
        ]);
        let frames = serve_once_with_policy(fs, Some(serve_policy(peer, &table)), request);

        assert_eq!(frames[1].message_type(), P9_RATTACH, "attach must succeed");
        assert_eq!(
            p9_decode_rread(frames.last().unwrap()).unwrap(),
            b"granted",
            "the scoped root must expose the granted file's bytes"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn read_only_grant_denies_writes_with_eacces() {
        let (root, fs) = capability_root();
        let peer = PeerId::from_bytes([9u8; 32]);
        let table = GrantTable::new();
        table.grant(Grant::new(
            peer,
            "docs",
            Arc::clone(&fs),
            "docs",
            Rights::read_only(),
        ));

        let request = encode_frames([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "k", "docs", 0).unwrap(),
            p9_twalk(3, 1, 2, &["readme.txt"]).unwrap(),
            p9_tlopen(5, 2, 1),
            p9_twrite(6, 2, 0, b"overwrite").unwrap(),
        ]);
        let frames = serve_once_with_policy(fs, Some(serve_policy(peer, &table)), request);

        assert_eq!(frames[1].message_type(), P9_RATTACH, "attach must succeed");
        let open_response = &frames[3];
        assert_eq!(open_response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(open_response).unwrap().ecode, EACCES);
        assert_ne!(
            frames.last().unwrap().message_type(),
            P9_RWRITE,
            "a write to a read-only grant must not succeed"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn revoking_a_grant_denies_the_next_attach() {
        let (root, fs) = capability_root();
        let peer = PeerId::from_bytes([3u8; 32]);
        let table = GrantTable::new();
        table.grant(Grant::new(
            peer,
            "projects/foo",
            Arc::clone(&fs),
            "projects/foo",
            Rights::read_write(),
        ));

        let attach = encode_frames([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "k", "projects/foo", 0).unwrap(),
        ]);

        let before = serve_once_with_policy(
            Arc::clone(&fs),
            Some(serve_policy(peer, &table)),
            attach.clone(),
        );
        assert_eq!(
            before[1].message_type(),
            P9_RATTACH,
            "the granted peer attaches before revoke"
        );

        assert_eq!(table.revoke(peer, "projects/foo"), 1);

        let after = serve_once_with_policy(fs, Some(serve_policy(peer, &table)), attach);
        assert_eq!(after[1].message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&after[1]).unwrap().ecode, EACCES);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn build_serve_policy_is_none_without_peer() {
        let (root, fs) = capability_root();
        assert!(build_serve_policy(None, Vec::new(), &fs).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
