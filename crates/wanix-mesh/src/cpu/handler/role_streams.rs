//! Opening and accepting a cpu job's two role-sorted bidi streams.
//!
//! A cpu job multiplexes a control and an export bidi stream on one QUIC
//! connection. The caller [`open_sorted_streams`] opens both and writes the role
//! byte first on each (control=0, export=1); the acceptor [`accept_sorted_streams`]
//! accepts both, reads each stream's leading role byte, and returns them sorted
//! `(control, export)` regardless of the order they became visible — first-write
//! order, not open order, is what an iroh acceptor sees.
//!
//! # Per-op deadline
//!
//! Both stream constructions are bound by the node's per-op deadline (the cpu
//! plane's slow-peer DoS answer, matching the 9P plane). Unlike the 9P inbound
//! server — which deliberately leaves its *idle read* unbounded so a mounted-but-
//! idle session is not torn down — a cpu job is short-lived request/response
//! work, so v1 applies the deadline uniformly to every read/write on both
//! streams. The tradeoff: a guest that runs longer than the deadline between two
//! reverse-export 9P ops (a long compute phase with no I/O) tears its export
//! down. The default deadline is generous (30s); a longer one is configurable on
//! the node. Decoupling the export idle-read from in-flight work is a named
//! follow-up, not pretended here.

use std::time::Duration;

use iroh::endpoint::Connection;
use tokio::runtime::Handle;
use wanix_cpu::{StreamRole, read_role, write_role};

use crate::duplex::BlockingDuplex;

/// Opens the control and export bidi streams and writes their role bytes.
///
/// Each stream's role byte is written immediately after `open_bi`, which both
/// makes the stream visible to the acceptor's `accept_bi` (an iroh bidi stream is
/// invisible until its opener writes a first byte) and tags its role. Returns
/// `(control, export)`. `deadline` bounds each per-op read/write on the bridged
/// streams so a hostile acceptor cannot park the caller's threads forever.
///
/// # Errors
///
/// Returns a string error when a stream cannot be opened or its role byte cannot
/// be written.
pub(super) fn open_sorted_streams(
    connection: &Connection,
    handle: Handle,
    deadline: Duration,
) -> Result<(BlockingDuplex, BlockingDuplex), String> {
    let mut control = open_bi_duplex(connection, &handle, deadline)?;
    let mut export = open_bi_duplex(connection, &handle, deadline)?;
    write_role(&mut control, StreamRole::Control).map_err(|err| err.to_string())?;
    write_role(&mut export, StreamRole::Export).map_err(|err| err.to_string())?;
    Ok((control, export))
}

/// Accepts the two bidi streams and sorts them by their leading role byte.
///
/// Returns `(control, export)`. A stream whose role byte is missing or unknown,
/// or a pair that does not contain exactly one of each role, is an error.
/// `deadline` bounds each per-op read/write on the bridged streams so a hostile
/// caller cannot park an acceptor blocking-pool thread forever.
///
/// # Errors
///
/// Returns a string error when a stream cannot be accepted, a role byte cannot be
/// read, or the two streams are not one control and one export.
pub(super) async fn accept_sorted_streams(
    connection: &Connection,
    handle: Handle,
    deadline: Duration,
) -> Result<(BlockingDuplex, BlockingDuplex), String> {
    let first = accept_bi_duplex(connection, &handle, deadline).await?;
    let second = accept_bi_duplex(connection, &handle, deadline).await?;
    // Reading the role byte is a blocking op on the bridged stream; run it off the
    // runtime worker so block_on is safe. The sort consumes the role bytes, so the
    // returned streams are ready for the job.
    tokio::task::spawn_blocking(move || sort_by_role(first, second))
        .await
        .map_err(|err| err.to_string())?
}

/// Reads each stream's role byte and returns them ordered `(control, export)`.
fn sort_by_role(
    mut a: BlockingDuplex,
    mut b: BlockingDuplex,
) -> Result<(BlockingDuplex, BlockingDuplex), String> {
    let role_a = read_role(&mut a).map_err(|err| err.to_string())?;
    let role_b = read_role(&mut b).map_err(|err| err.to_string())?;
    match (role_a, role_b) {
        (StreamRole::Control, StreamRole::Export) => Ok((a, b)),
        (StreamRole::Export, StreamRole::Control) => Ok((b, a)),
        _ => Err("cpu job streams must be one control and one export".to_owned()),
    }
}

/// Opens one bidi stream and bridges it to a blocking duplex with `deadline`.
fn open_bi_duplex(
    connection: &Connection,
    handle: &Handle,
    deadline: Duration,
) -> Result<BlockingDuplex, String> {
    let connection = connection.clone();
    let (send, recv) = handle
        .block_on(async move { connection.open_bi().await })
        .map_err(|err| err.to_string())?;
    Ok(BlockingDuplex::new(
        send,
        recv,
        handle.clone(),
        Some(deadline),
    ))
}

/// Accepts one bidi stream and bridges it to a blocking duplex with `deadline`.
async fn accept_bi_duplex(
    connection: &Connection,
    handle: &Handle,
    deadline: Duration,
) -> Result<BlockingDuplex, String> {
    let (send, recv) = connection
        .accept_bi()
        .await
        .map_err(|err| err.to_string())?;
    Ok(BlockingDuplex::new(
        send,
        recv,
        handle.clone(),
        Some(deadline),
    ))
}
