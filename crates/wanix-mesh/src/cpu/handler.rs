//! The cpu acceptor (node Y) and dialer (caller) over QUIC.
//!
//! [`CpuAcceptor`] is the iroh [`ProtocolHandler`] for [`super::WANIX_CPU_ALPN`]:
//! it accepts a job's two bidi streams, role-sorts them, reads the spec, and runs
//! [`wanix_cpu::run_job`] inside `spawn_blocking` against a fresh task table. It
//! is grant-allowlisted — only peers an explicit allowlist admits run code.
//!
//! [`CpuDialer`] is the caller: it opens the control and export streams, writes
//! the role bytes, sends the spec, serves a scoped reverse export, and drains the
//! [`wanix_cpu::CpuEvent`] result batch — all over the held runtime [`Handle`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use wanix_cpu::{CollectedOutput, CpuJobSpec, read_spec, run_job};
use wanix_fs::FileSystem;
use wanix_id::PeerId;
use wanix_task::TaskTable;

use crate::duplex::BlockingDuplex;
use crate::identity::peer_id_for;
use crate::node::MAX_CONCURRENT_CPU_JOBS;

mod dialer;
mod role_streams;

use role_streams::accept_sorted_streams;

/// Builds a fresh, driver-registered [`TaskTable`] for one cpu job.
///
/// `wanix-mesh` must not depend on a concrete task runtime (`qjs`/`wasm`), so the
/// caller of [`CpuAcceptor::new`] injects this factory; the CLI registers the
/// real drivers. A fresh table per job is also the isolation boundary: a job's
/// task state never leaks into another peer's.
pub type TaskTableFactory = Arc<dyn Fn() -> TaskTable + Send + Sync>;

/// The cpu acceptor: runs grant-allowlisted remote jobs against exported worlds.
#[derive(Clone)]
pub struct CpuAcceptor {
    table_factory: TaskTableFactory,
    allowlist: Arc<dyn Fn(PeerId) -> bool + Send + Sync>,
    handle: Handle,
    deadline: Duration,
    /// Caps concurrently running cpu jobs; one permit is held per in-flight job
    /// for its lifetime, so a flood of stalled jobs from an allowlisted-but-
    /// hostile peer cannot exhaust the blocking pool shared with the 9P plane.
    jobs: Arc<Semaphore>,
}

impl CpuAcceptor {
    /// Builds an acceptor admitting peers for which `allowlist` returns `true`.
    ///
    /// `table_factory` produces a fresh driver-registered [`TaskTable`] per job.
    /// `allowlist` is the exec trust gate: remote code execution is the sharpest
    /// capability, so the default posture is deny and only admitted peers run.
    /// `deadline` bounds every per-op read/write on the job's streams, so a
    /// hostile caller cannot park an acceptor thread forever. Concurrent jobs are
    /// capped at [`MAX_CONCURRENT_CPU_JOBS`].
    #[must_use]
    pub fn new(
        table_factory: TaskTableFactory,
        allowlist: Arc<dyn Fn(PeerId) -> bool + Send + Sync>,
        handle: Handle,
        deadline: Duration,
    ) -> Self {
        Self {
            table_factory,
            allowlist,
            handle,
            deadline,
            jobs: Arc::new(Semaphore::new(MAX_CONCURRENT_CPU_JOBS)),
        }
    }
}

impl fmt::Debug for CpuAcceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CpuAcceptor").finish()
    }
}

impl ProtocolHandler for CpuAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        // Identity is the verified QUIC handshake key (0-RTT is never used).
        let peer = peer_id_for(connection.remote_id());
        if !(self.allowlist)(peer) {
            // Denied peers get nothing: close without accepting a stream. The cpu
            // ALPN is exec, so default-deny is enforced before any work.
            connection.close(0u32.into(), b"cpu: peer not allowlisted");
            return Ok(());
        }
        // Admit at most MAX_CONCURRENT_CPU_JOBS jobs. The owned permit is held for
        // the job's whole lifetime (stream accept, spec read, and run), so a
        // flood of stalled jobs cannot exhaust the blocking pool. The semaphore is
        // never closed, so acquire only fails on shutdown.
        let Ok(permit) = Arc::clone(&self.jobs).acquire_owned().await else {
            return Ok(());
        };
        // Accept and role-sort the control and export bidi streams, deadline-bound.
        let Ok((control, export)) =
            accept_sorted_streams(&connection, self.handle.clone(), self.deadline).await
        else {
            return Ok(());
        };
        let table_factory = Arc::clone(&self.table_factory);
        // run_job is synchronous and runs the guest to completion; run it on the
        // blocking pool so it never pins a runtime worker and BlockingDuplex can
        // block_on safely.
        let _ = tokio::task::spawn_blocking(move || {
            serve_one_job(&table_factory, control, export);
            // Permit held until the job ends, then released here.
            drop(permit);
        })
        .await;
        Ok(())
    }
}

/// Runs one cpu job synchronously: read the spec, run it, batch the result.
fn serve_one_job(
    table_factory: &TaskTableFactory,
    mut control: BlockingDuplex,
    export: BlockingDuplex,
) {
    // The caller sends the spec first on the control stream, after its role byte
    // (already consumed by role-sorting).
    let Ok(spec) = read_spec(&mut control) else {
        return;
    };
    let table = table_factory();
    // run_job reads nothing more from control; it only writes the event batch.
    let _ = run_job(table, &spec, Box::new(export), &mut control);
}

/// The outcome of a dialed cpu job: its captured output and exit status.
#[derive(Debug)]
pub struct CpuJobReport {
    /// The job's captured standard output and error, as drained from control.
    pub output: CollectedOutput,
    /// The job's terminal exit status.
    pub exit_code: i32,
}

/// Dials a cpu acceptor and runs one job, exporting a scoped reverse namespace.
///
/// The dialer opens the control and export streams, writes the role bytes, sends
/// `spec`, serves `export_root` (a scoped [`wanix_cpu::ExportScope`] root) as the
/// reverse 9P world, and drains the result batch — returning the job's output and
/// exit code. The export root must be scoped and read-only by default; this is
/// the caller's half of the mutual confinement.
pub struct CpuDialer {
    connection: Connection,
    handle: Handle,
    deadline: Duration,
}

impl CpuDialer {
    /// Wraps an established cpu [`Connection`], driving streams on `handle`.
    ///
    /// `deadline` bounds every per-op read/write on the job's streams, so a
    /// hostile acceptor cannot park the caller's threads forever.
    #[must_use]
    pub fn new(connection: Connection, handle: Handle, deadline: Duration) -> Self {
        Self {
            connection,
            handle,
            deadline,
        }
    }

    /// Runs `spec` against the reverse-exported `export_root`, blocking until the
    /// job's terminal exit, then returns its captured output and exit code.
    ///
    /// # Errors
    ///
    /// Returns a string error when stream setup, the reverse export, or the
    /// control drain fails.
    pub fn run(
        self,
        spec: &CpuJobSpec,
        export_root: Arc<dyn FileSystem>,
    ) -> Result<CpuJobReport, String> {
        dialer::run_dialed_job(
            self.connection,
            self.handle,
            self.deadline,
            spec,
            export_root,
        )
    }
}
