use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

use wanix_9p::{AttachPolicy, P9Server, P9TransportError, PeerId};
use wanix_fs::{FileSystem, LocalFs};

use crate::{CliError, write_process_output};

use super::P9ListenCommand;
use super::grant::build_policy;

/// The verified peer and shared attach policy a policy-gated server consults at
/// every `Tattach`.
///
/// The policy is shared, so a revoke applied between connections (the demo's
/// "remove the grant -> next attach denied" step) is observed by the next
/// connection's freshly built [`P9Server`].
#[derive(Clone)]
struct ServePolicy {
    peer: PeerId,
    policy: Arc<dyn AttachPolicy>,
}

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
    let policy = build_serve_policy(&command, &root);
    write_p9_listen_startup(&listener, process_stderr)?;
    serve_p9_listener(
        command.once,
        &listener,
        root,
        policy.as_ref(),
        process_stderr,
    )
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

/// Builds the shared attach policy when `--peer` is supplied; otherwise returns
/// `None` so the server keeps today's byte-for-byte behavior.
fn build_serve_policy(
    command: &P9ListenCommand,
    root: &Arc<dyn FileSystem>,
) -> Option<ServePolicy> {
    let peer = command.peer?;
    let (_, policy) = build_policy(peer, command.grants.clone(), Arc::clone(root));
    Some(ServePolicy {
        peer,
        policy: Arc::new(policy),
    })
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
    policy: Option<&ServePolicy>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    if once {
        return serve_one_connection(listener, root, policy, process_stderr);
    }

    serve_p9_listener_loop(listener, root, policy, process_stderr)
}

fn serve_p9_listener_loop(
    listener: &TcpListener,
    root: Arc<dyn FileSystem>,
    policy: Option<&ServePolicy>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    loop {
        let exit_code = serve_one_connection(listener, Arc::clone(&root), policy, process_stderr)?;
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
    policy: Option<&ServePolicy>,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let (stream, peer_addr) = listener
        .accept()
        .map_err(|error| CliError::new(format!("p9-listen accept failed: {error}"), 1))?;
    match serve_stream_connection(root, policy, stream) {
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
    policy: Option<&ServePolicy>,
    stream: TcpStream,
) -> Result<wanix_9p::P9TransportStats, P9TransportError> {
    let reader = stream.try_clone().map_err(P9TransportError::Io)?;
    let mut server = match policy {
        Some(serve_policy) => {
            P9Server::with_policy(root, serve_policy.peer, Arc::clone(&serve_policy.policy))
        }
        None => P9Server::new(root),
    };
    server.serve_stream(reader, stream)
}

pub(super) fn p9_listen_startup_message(local_addr: SocketAddr) -> String {
    format!("wanix-rust p9-listen: listening on {local_addr}\n")
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::Shutdown;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use wanix_9p::{Grant, GrantTable, GrantTablePolicy};
    use wanix_protocol::{
        P9_RATTACH, P9_RLERROR, P9_RWRITE, P9_VERSION_9P2000_L, P9Frame, P9FrameBuffer,
        p9_decode_rlerror, p9_tattach, p9_tlopen, p9_tversion, p9_twalk, p9_twrite,
    };
    use wanix_vfs::Rights;

    use super::*;

    const EACCES: u32 = 13;
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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

    fn temp_root() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        dir.push(format!("wanix-p9-policy-{}-{nonce}", std::process::id()));
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
            let _ = serve_stream_connection(root, policy.as_ref(), stream);
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

        // Attach aname=projects/foo, then walk to `file.txt` (which only exists
        // under the granted prefix), open and read it back.
        let request = encode_frames([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "k", "projects/foo", 0).unwrap(),
            p9_twalk(3, 1, 2, &["file.txt"]).unwrap(),
            p9_tlopen(5, 2, 0),
            wanix_protocol::p9_tread(6, 2, 0, 7),
        ]);
        let frames = serve_once_with_policy(fs, Some(serve_policy(peer, &table)), request);

        assert_eq!(frames[1].message_type(), P9_RATTACH, "attach must succeed");
        assert_eq!(
            wanix_protocol::p9_decode_rread(frames.last().unwrap()).unwrap(),
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

        // Attach aname=docs (read-only), then try to open the file for writing
        // (O_WRONLY). The SubtreeFs rights gate rejects the write-mode open as
        // EACCES at the open, so the subsequent write can never land.
        let request = encode_frames([
            p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap(),
            p9_tattach(2, 1, 0xffff_ffff, "k", "docs", 0).unwrap(),
            p9_twalk(3, 1, 2, &["readme.txt"]).unwrap(),
            p9_tlopen(5, 2, 1),
            p9_twrite(6, 2, 0, b"overwrite").unwrap(),
        ]);
        let frames = serve_once_with_policy(fs, Some(serve_policy(peer, &table)), request);

        assert_eq!(frames[1].message_type(), P9_RATTACH, "attach must succeed");
        // The write-mode open is the enforcement point: EACCES, not Rlopen.
        let open_response = &frames[3];
        assert_eq!(open_response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(open_response).unwrap().ecode, EACCES);
        // And the write itself never succeeds (the fid was never opened).
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

        // First attach is authorized.
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

        // Revoke the grant on the shared table; the policy reads the same table.
        assert_eq!(table.revoke(peer, "projects/foo"), 1);

        // The next connection's attach is now default-denied with EACCES.
        let after = serve_once_with_policy(fs, Some(serve_policy(peer, &table)), attach);
        assert_eq!(after[1].message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&after[1]).unwrap().ecode, EACCES);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn build_serve_policy_is_none_without_peer() {
        let (root, fs) = capability_root();
        let command = P9ListenCommand {
            root_path: root.clone(),
            addr: "127.0.0.1:0".to_owned(),
            once: true,
            peer: None,
            grants: Vec::new(),
        };
        assert!(build_serve_policy(&command, &fs).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
