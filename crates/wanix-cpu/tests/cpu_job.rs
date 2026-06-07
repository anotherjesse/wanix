//! End-to-end CPU job over a loopback transport: cpu(1), the mesh way.
//!
//! Two TCP loopback stream pairs stand in for a job's two role-sorted QUIC bidi
//! streams (control + export). The caller serves a scoped, read-only export
//! namespace on the export stream and drains the control batch; the acceptor
//! runs a task whose world *is* that exported namespace, reading the caller's
//! file through the reverse 9P session and writing it back as the job's stdout.
//!
//! This proves the full Slice 6 path with the real synchronous `P9Server` /
//! `RemoteFs` over a real socket, and no async/iroh: compute travels to the data
//! and the result returns on the control stream as a `CpuEvent` batch. The caller
//! owns the export lifetime through the control protocol: once it drains the
//! terminal `Exit`, it shuts down its export stream to stop the reverse server.

use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use wanix_cpu::{
    CollectedOutput, CpuJobSpec, ExportScope, GrantedService, StreamRole, drive_control, read_role,
    run_job, serve_export, write_role,
};
use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
use wanix_task::{Fd, Task, TaskDriver, TaskTable};
use wanix_vfs::Rights;

/// A test task driver that runs a job against its world (the exported namespace).
///
/// It reads the program file from the task's namespace (proving the reverse
/// export is the world), echoes the bytes to fd 1, optionally writes a marker to
/// fd 2, and records an exit status — exactly the observable contract a real
/// driver (`qjs`/`wasm`) honors, without pulling a WASI runtime into this crate.
struct EchoWorldDriver {
    /// Marker written to the job's stderr, proving stderr crosses the batch too.
    stderr_marker: Vec<u8>,
    /// Exit status the driver records.
    exit: String,
}

impl TaskDriver for EchoWorldDriver {
    fn start(&self, task: &Task) -> wanix_fs::FsResult<()> {
        // The program path is the file to echo; resolve it through the world.
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
        if !self.stderr_marker.is_empty() {
            task.write_fd(Fd::STDERR, &self.stderr_marker)?;
        }
        task.set_exit(&self.exit)?;
        Ok(())
    }
}

/// A bidirectional loopback stream pair: a connected client and server `TcpStream`.
fn loopback_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (server, _peer) = listener.accept().unwrap();
    (client, server)
}

/// Builds the caller's scoped export root: a read-only `work` subtree holding
/// `build.js`, plus a granted read-only `#kv` service.
fn caller_export_root() -> Arc<dyn FileSystem> {
    let host = Arc::new(MemFs::new());
    host.create_dir_all("work").unwrap();
    host.write_file("work/build.js", b"console.log('built on the data node')")
        .unwrap();
    // A file outside the job subtree the export must NOT reach.
    host.write_file("secret.txt", b"do not export me").unwrap();

    let kv = Arc::new(MemFs::new());
    kv.write_file("config", b"region=us").unwrap();

    ExportScope::new(host, "work")
        .grant(GrantedService::new("#kv", kv, ".", Rights::read_only()))
        .into_root()
        .unwrap()
}

/// The result of driving one job from the caller side.
struct JobResult {
    output: CollectedOutput,
    code: i32,
}

/// Runs a full job: the caller serves `caller_export_root` and drains control;
/// the acceptor runs `program` under the `echo` driver against the exported world.
///
/// Returns the caller's collected output and exit code. The acceptor's `run_job`
/// is allowed to fail (a job that reads outside the scope fails its open); the
/// terminal control `Exit` still carries the non-zero status, which is what the
/// caller observes.
fn run_one_job(driver: EchoWorldDriver, program: &'static str) -> JobResult {
    let (caller_control, acceptor_control) = loopback_pair();
    let (caller_export, acceptor_export) = loopback_pair();

    let caller = thread::spawn(move || {
        let mut caller_control = caller_control;
        let mut caller_export = caller_export;
        // The caller writes the 1-byte role discriminator first on each stream,
        // immediately after "opening" it — control=0, export=1.
        write_role(&mut caller_control, StreamRole::Control).unwrap();
        write_role(&mut caller_export, StreamRole::Export).unwrap();
        // A shutdown handle so the caller can stop serving once the job is done.
        let shutdown = caller_export.try_clone().unwrap();

        let export_root = caller_export_root();
        let export_thread =
            thread::spawn(move || serve_export(export_root, caller_export).unwrap());

        let mut output = CollectedOutput::default();
        let code = drive_control(&mut caller_control, &mut output).unwrap();
        // The terminal Exit arrived: tear down the reverse export to stop serving.
        let _ = shutdown.shutdown(Shutdown::Both);
        export_thread.join().unwrap();
        JobResult { output, code }
    });

    let acceptor = thread::spawn(move || {
        let mut acceptor_control = acceptor_control;
        let mut acceptor_export = acceptor_export;
        // Reading the leading role byte sorts the two streams regardless of the
        // order in which they became visible to us.
        assert_eq!(
            read_role(&mut acceptor_control).unwrap(),
            StreamRole::Control
        );
        assert_eq!(read_role(&mut acceptor_export).unwrap(), StreamRole::Export);

        let table = TaskTable::new();
        table.register_driver("echo", Arc::new(driver)).unwrap();
        // The program path is relative to the world root, which is the caller's
        // `work` subtree, so `build.js` resolves to `work/build.js` on the caller.
        let spec = CpuJobSpec::new("echo", program).unwrap();
        // run_job may fail (e.g. a denied open); the control batch still carries a
        // terminal Exit, which is what the caller asserts on.
        let _ = run_job(
            table,
            &spec,
            Box::new(acceptor_export),
            &mut acceptor_control,
        );
    });

    acceptor.join().unwrap();
    caller.join().unwrap()
}

#[test]
fn cpu_job_runs_against_the_reverse_exported_world() {
    let result = run_one_job(
        EchoWorldDriver {
            stderr_marker: b"warming up\n".to_vec(),
            exit: "0".to_owned(),
        },
        "build.js",
    );
    // The job read the caller's file through the reverse export and echoed it.
    assert_eq!(
        result.output.stdout,
        b"console.log('built on the data node')"
    );
    assert_eq!(result.output.stderr, b"warming up\n");
    assert_eq!(result.code, 0);
}

#[test]
fn the_exported_world_is_scoped_and_cannot_reach_outside_the_subtree() {
    // `secret.txt` lives at the caller's host root, OUTSIDE the `work` job subtree
    // the export scopes to, so it is not present in the world; the driver's open
    // fails and the job reports a non-zero exit through the batch.
    let result = run_one_job(
        EchoWorldDriver {
            stderr_marker: Vec::new(),
            exit: "1".to_owned(),
        },
        "secret.txt",
    );
    assert_ne!(
        result.code, 0,
        "reading outside the scoped subtree must fail the job"
    );
}

#[test]
fn a_granted_service_imports_into_the_world() {
    // The granted `#kv` service imports into the world at its mount: the job reads
    // `#kv/config` through the reverse export exactly like a local file.
    let result = run_one_job(
        EchoWorldDriver {
            stderr_marker: Vec::new(),
            exit: "0".to_owned(),
        },
        "#kv/config",
    );
    assert_eq!(result.output.stdout, b"region=us");
    assert_eq!(result.code, 0);
}

/// A direct check that the scoped export root denies escape, independent of the
/// transport — the security invariant the scope guarantees the acceptor.
#[test]
fn scoped_export_root_denies_paths_outside_the_subtree() {
    let root = caller_export_root();
    assert!(
        root.metadata(&NormalizedPath::new("secret.txt").unwrap())
            .is_err(),
        "a file outside the job subtree must not be reachable in the export"
    );
    assert!(
        root.open(
            &NormalizedPath::new("build.js").unwrap(),
            OpenOptions::read_write()
        )
        .is_err(),
        "the read-only export must deny a write-mode open"
    );
}
