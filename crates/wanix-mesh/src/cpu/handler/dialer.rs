//! The caller's side of a dialed cpu job over QUIC.
//!
//! [`run_dialed_job`] opens the two role-sorted streams, sends the spec on
//! control, serves the scoped reverse export on a blocking thread, drains the
//! [`wanix_cpu::CpuEvent`] batch, and — on the terminal exit — closes the QUIC
//! connection to stop the reverse export. Closing the connection is the caller's
//! protocol-level "job done" signal: it half-closes the export stream so the
//! synchronous `serve_export` reader returns, exactly as a `TcpStream::shutdown`
//! does in the transport-agnostic tests.

use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use tokio::runtime::Handle;
use wanix_cpu::{CollectedOutput, CpuJobSpec, drive_control, serve_export, write_spec};
use wanix_fs::FileSystem;

use super::CpuJobReport;
use super::role_streams::open_sorted_streams;

/// Runs one dialed cpu job to its terminal exit and returns its report.
///
/// `deadline` bounds every per-op read/write on the job's streams.
pub(super) fn run_dialed_job(
    connection: Connection,
    handle: Handle,
    deadline: Duration,
    spec: &CpuJobSpec,
    export_root: Arc<dyn FileSystem>,
) -> Result<CpuJobReport, String> {
    let (mut control, export) = open_sorted_streams(&connection, handle.clone(), deadline)?;

    // Serve the scoped reverse export on the blocking pool. It runs until the
    // connection is closed below, which half-closes the export stream.
    let export_task = handle.spawn_blocking(move || {
        let _ = serve_export(export_root, export);
    });

    // The acceptor expects the spec first on the control stream.
    write_spec(&mut control, spec).map_err(|err| err.to_string())?;

    // Drain the result batch until the terminal exit event.
    let mut output = CollectedOutput::default();
    let exit_code = drive_control(&mut control, &mut output).map_err(|err| err.to_string())?;

    // Job done: closing the connection stops the reverse export so its server
    // thread can finish. Then await the export task so no thread is left dangling.
    connection.close(0u32.into(), b"cpu: job complete");
    let _ = handle.block_on(export_task);

    Ok(CpuJobReport { output, exit_code })
}
