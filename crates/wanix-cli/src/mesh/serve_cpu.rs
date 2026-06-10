//! `mesh-serve --cpu`: bind the `#cpu` exec acceptor beside the served namespace.
//!
//! Serving the cpu plane is REMOTE CODE EXECUTION on this host: an admitted
//! peer reverse-exports a namespace and this node runs a `qjs`/`wasm` task
//! against it. The trust posture mirrors the `--wanix-services` exec-device
//! rule (ADR 0006 / the mesh blueprint): the parser refuses `--cpu` on the
//! public endpoint entirely, and on the local direct-address-only endpoint the
//! acceptor admits either exactly the `--peer` identity or — with no `--peer`
//! — anyone holding the out-of-band ticket, exactly like the rest of a local
//! open serve.
//!
//! Each admitted job gets a fresh, fully driver-registered [`TaskTable`] (the
//! per-job isolation boundary); the QuickJS runner is built once at serve time
//! and shared across jobs, since the runner is the engine, not task state.

use std::sync::Arc;

use wanix_id::PeerId;
use wanix_mesh::{CpuAcceptor, MeshNode, TaskTableFactory};
use wanix_qjs::QuickJsTaskDriver;
use wanix_task::TaskTable;
use wanix_wasm::WasmTaskDriver;

use crate::{CliError, quickjs_runner};

/// Builds the `--cpu` acceptor for `node`: per-job driver-registered task
/// tables behind the exec allowlist derived from `--peer`.
///
/// # Errors
///
/// Returns a CLI error when the QuickJS runner cannot be built.
pub(super) fn cpu_acceptor_for(
    node: &MeshNode,
    peer: Option<PeerId>,
) -> Result<CpuAcceptor, CliError> {
    let runner = quickjs_runner()?;
    let factory: TaskTableFactory = Arc::new(move || cpu_task_table(&runner));
    Ok(node.cpu_acceptor(factory, cpu_allowlist(peer)))
}

/// The exec allowlist: with `--peer HEX` only that verified identity may run
/// code; without it (local direct-address-only endpoint, ticket exchanged out
/// of band) any dialing peer is admitted, matching the local open-serve
/// posture for `--wanix-services`.
fn cpu_allowlist(peer: Option<PeerId>) -> Arc<dyn Fn(PeerId) -> bool + Send + Sync> {
    match peer {
        Some(granted) => Arc::new(move |dialer| dialer == granted),
        None => Arc::new(|_| true),
    }
}

/// One fresh task table per job, with the same driver set as `serve
/// --wanix-services` (`noop`, `qjs`, `wasm`) so a cpu job command mirrors the
/// local `#task` launch surface.
fn cpu_task_table(runner: &Arc<wanix_qjs::QuickJsRunner>) -> TaskTable {
    let table = TaskTable::new();
    table
        .register_noop_driver("noop")
        .expect("register noop driver on a fresh table");
    table
        .register_driver("qjs", Arc::new(QuickJsTaskDriver::new(Arc::clone(runner))))
        .expect("register qjs driver on a fresh table");
    table
        .register_driver("wasm", Arc::new(WasmTaskDriver::new()))
        .expect("register wasm driver on a fresh table");
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_with_peer_admits_only_that_peer() {
        let granted = PeerId::from_bytes([7u8; 32]);
        let other = PeerId::from_bytes([8u8; 32]);
        let allowlist = cpu_allowlist(Some(granted));
        assert!(allowlist(granted));
        assert!(!allowlist(other), "exec must stay peer-scoped under --peer");
    }

    #[test]
    fn allowlist_without_peer_admits_ticket_holders() {
        // The local direct-address-only endpoint: the ticket is the capability,
        // exchanged out of band, same as an open local --wanix-services serve.
        let allowlist = cpu_allowlist(None);
        assert!(allowlist(PeerId::from_bytes([9u8; 32])));
    }

    #[test]
    fn cpu_task_table_registers_the_serve_driver_set() {
        let runner = quickjs_runner().expect("build qjs runner");
        let kinds = cpu_task_table(&runner).driver_kinds();
        for kind in ["auto", "noop", "qjs", "wasm"] {
            assert!(kinds.contains(&kind.to_owned()), "kinds: {kinds:?}");
        }
    }
}
