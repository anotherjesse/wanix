//! End-to-end Plan 9 cpu over a real QUIC connection: send the task to the data.
//!
//! Two [`MeshNode`]s are bound on loopback (relay/DNS disabled). Node A (the data
//! node) serves the cpu exec plane with node B allowlisted; node B (the caller)
//! dials A's cpu ALPN, exports a scoped, read-only reverse namespace holding a
//! program, and runs a job. The task runs *on node A* against node B's exported
//! world over the reverse 9P session, and its captured output and exit status
//! return on the control stream as a `CpuEvent` batch. The synchronous cpu core
//! (`run_job`, `serve_export`, `drive_control`) runs unchanged over QUIC.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use wanix_cpu::{CpuJobSpec, ExportScope};
use wanix_fs::{FileSystem, MemFs, OpenOptions};
use wanix_id::NodeIdentity;
use wanix_mesh::MeshNode;
use wanix_task::{Fd, Task, TaskDriver, TaskTable};

/// A test driver that echoes its world program file to stdout, like a real run.
///
/// It reads the program path through the task namespace — which on the acceptor
/// is node B's reverse-exported world — proving the task ran against the remote
/// caller's files, then writes the bytes to fd 1 and records a zero exit.
struct EchoWorldDriver;

impl TaskDriver for EchoWorldDriver {
    fn start(&self, task: &Task) -> wanix_fs::FsResult<()> {
        let program = task.spec().program;
        let world = task.namespace();
        let mut file = world.open(&program, OpenOptions::read())?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = file.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        task.write_fd(Fd::STDOUT, &bytes)?;
        task.set_exit("0")?;
        Ok(())
    }
}

fn loopback() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

/// Builds a fresh task table with the `echo` test driver registered.
fn echo_table_factory() -> wanix_mesh::TaskTableFactory {
    Arc::new(|| {
        let table = TaskTable::new();
        table
            .register_driver("echo", Arc::new(EchoWorldDriver))
            .unwrap();
        table
    })
}

/// Builds node B's scoped reverse export: a read-only `work` subtree with a
/// program file and a sibling `secret.txt` the export must not reach.
fn caller_export_root() -> Arc<dyn FileSystem> {
    let host = Arc::new(MemFs::new());
    host.create_dir_all("work").unwrap();
    host.write_file("work/build.js", b"console.log('cpu ran on the data node')")
        .unwrap();
    host.write_file("secret.txt", b"do not export me").unwrap();
    ExportScope::new(host, "work").into_root().unwrap()
}

#[test]
fn cpu_job_runs_on_the_remote_node_over_quic() {
    let identity_a = NodeIdentity::from_secret_bytes([101u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([102u8; 32]);
    let peer_b = identity_b.peer_id();

    // Node A serves the cpu exec plane with node B explicitly allowlisted.
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    let acceptor = node_a.cpu_acceptor(echo_table_factory(), Arc::new(move |peer| peer == peer_b));
    node_a.serve_cpu(acceptor);
    let ticket = node_a.ticket();

    // Node B dials the cpu plane and runs a job, exporting its scoped world.
    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    let dialer = node_b.dial_cpu(ticket).unwrap();
    let spec = CpuJobSpec::new("echo", "build.js").unwrap();
    let report = dialer.run(&spec, caller_export_root()).unwrap();

    // The job read node B's file through the reverse export and echoed it back.
    assert_eq!(
        report.output.stdout,
        b"console.log('cpu ran on the data node')"
    );
    assert_eq!(report.exit_code, 0);
}

#[test]
fn an_unallowlisted_peer_cannot_run_a_cpu_job() {
    let identity_a = NodeIdentity::from_secret_bytes([111u8; 32]);
    let identity_b = NodeIdentity::from_secret_bytes([112u8; 32]);

    // Node A serves cpu but allowlists NO ONE (default-deny exec).
    let mut node_a = MeshNode::bind_local(&identity_a, loopback()).unwrap();
    let acceptor = node_a.cpu_acceptor(echo_table_factory(), Arc::new(|_peer| false));
    node_a.serve_cpu(acceptor);
    let ticket = node_a.ticket();

    let node_b = MeshNode::bind_local(&identity_b, loopback()).unwrap();
    // The connect itself may succeed, but the acceptor closes the connection
    // without accepting streams, so running the job fails rather than executing.
    let result = node_b.dial_cpu(ticket).and_then(|dialer| {
        let spec = CpuJobSpec::new("echo", "build.js").unwrap();
        dialer
            .run(&spec, caller_export_root())
            .map_err(wanix_mesh::MeshError::Session)
    });
    assert!(
        result.is_err(),
        "an unallowlisted peer must not run remote code"
    );
}
