//! The `iroh://` ticket scheme: parse a peer address and dial it over QUIC.
//!
//! The peer id IS the resource identity; `?addr=IP:PORT` is only a direct-route
//! HINT, never identity (see `docs/better-iroh-discovery.md`). So
//! `iroh://<64-hex-peer-id>` is the canonical resource address: every mesh
//! endpoint runs always-on mDNS local discovery, so a bare `iroh://PEER` dials on
//! the LAN or same machine with no `addr=` — and a peer that restarts on a new
//! port is still found by its stable id. On the public network it also resolves
//! via relay/DNS. `?addr=IP:PORT` query parameters are an optional direct-route
//! shortcut/fallback.
//!
//! Crucially, a stale or wrong `addr=` cannot mount the wrong resource: every
//! route still authenticates as the requested peer id, so a bad hint fails the
//! dial rather than connecting to whoever happens to answer at that socket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use wanix_id::NodeIdentity;
use wanix_mesh::{EndpointAddr, EndpointId, IrohStreamFactory, MeshNode, NativeFs};

use crate::CliError;

/// The imported remote filesystem type for an `iroh://` mesh mount: the native
/// `wanix-mesh-wire` client over a held QUIC connection.
///
/// Wanix↔Wanix mesh imports ride the native wire (typed `FsError`s, one bidi
/// stream per op / per open file), not 9P. 9P stays at the foreign edge (the
/// `tcp://` mount path, Linux/v86/QEMU, external tools, the cockpit) per
/// ADR 0004 and the native-mesh-wire plan §9.
pub(crate) type MeshFs = NativeFs<IrohStreamFactory>;

/// URL scheme that selects a QUIC mesh dial instead of a raw TCP 9P dial.
pub(crate) const IROH_SCHEME: &str = "iroh://";

/// Foreground CLI mesh mounts should feel like local interactive tools: bounded
/// enough for an agent or shell to recover, without making transient Wi-Fi or
/// mDNS hiccups look instant-fatal.
const CLI_MESH_MOUNT_DEADLINE: Duration = Duration::from_secs(5);

/// A parsed `iroh://` dial target: a verified peer id (the resource identity)
/// plus optional direct-route hints for first contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeshTicket {
    peer: EndpointId,
    addrs: Vec<SocketAddr>,
}

impl MeshTicket {
    /// Parses an `iroh://<peer-hex>[?addr=IP:PORT&...]` ticket.
    ///
    /// # Errors
    ///
    /// Returns a usage error when the scheme is wrong, the peer id is not 64 hex
    /// digits, or a direct address does not parse as `IP:PORT`.
    pub(crate) fn parse(value: &str) -> Result<Self, CliError> {
        let rest = value.strip_prefix(IROH_SCHEME).ok_or_else(|| {
            CliError::usage(format!(
                "mesh address must start with {IROH_SCHEME}: {value}"
            ))
        })?;
        let (id_part, query) = match rest.split_once('?') {
            Some((id, query)) => (id, Some(query)),
            None => (rest, None),
        };
        let peer = parse_peer_id(id_part)?;
        let addrs = match query {
            Some(query) => parse_addr_query(query)?,
            None => Vec::new(),
        };
        Ok(Self { peer, addrs })
    }

    /// Builds the iroh [`EndpointAddr`] this ticket dials.
    pub(crate) fn endpoint_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.peer);
        for socket in &self.addrs {
            addr = addr.with_ip_addr(*socket);
        }
        addr
    }
}

/// Parses a 64-character lowercase-hex ed25519 public key into an [`EndpointId`].
fn parse_peer_id(value: &str) -> Result<EndpointId, CliError> {
    if value.len() != 64 {
        return Err(CliError::usage(format!(
            "iroh ticket peer id must be 64 hex digits, got {} characters",
            value.len()
        )));
    }
    value
        .parse::<EndpointId>()
        .map_err(|error| CliError::usage(format!("invalid iroh peer id: {error}")))
}

/// Parses `addr=IP:PORT[&addr=IP:PORT...]` query parameters into socket addrs.
fn parse_addr_query(query: &str) -> Result<Vec<SocketAddr>, CliError> {
    let mut addrs = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let value = pair.strip_prefix("addr=").ok_or_else(|| {
            CliError::usage(format!(
                "iroh ticket query expects addr=IP:PORT, got {pair:?}"
            ))
        })?;
        let socket = value.parse::<SocketAddr>().map_err(|error| {
            CliError::usage(format!(
                "iroh ticket addr {value:?} is not IP:PORT: {error}"
            ))
        })?;
        addrs.push(socket);
    }
    Ok(addrs)
}

/// A dialed `iroh://` mount: the remote filesystem and the live node that backs
/// it.
///
/// The [`MeshNode`] owns the tokio runtime and iroh endpoint the returned
/// [`MeshFs`] drives every operation through. The native wire's
/// [`IrohStreamFactory`] holds only a non-owning runtime
/// [`tokio::runtime::Handle`] (each op opens a fresh bidi stream wrapped in a
/// `BlockingDuplex`), so dropping the node shuts the runtime down and the next
/// namespace op would panic ("a Tokio 1.x context was found, but it is being
/// shutdown"). The node is therefore returned alongside the [`MeshFs`] and
/// **must be kept alive for the mount's whole lifetime** — every
/// read/write/walk through the mount runs on it.
#[must_use]
pub(crate) struct IrohMount {
    /// The imported remote filesystem (native wire), bound into a namespace by
    /// the caller.
    pub(crate) remote: Arc<MeshFs>,
    /// The live dialer node; kept alive for the mount's lifetime. Held only to
    /// keep the runtime the `remote` drives its operations on from shutting down.
    _node: MeshNode,
}

/// Dials an `iroh://` ticket and returns a connected mount over the native wire.
///
/// A fresh dialer-only [`MeshNode`] is bound from an ephemeral identity (the
/// importer's stable identity is not required to dial out), then the ticket is
/// dialed over [`wanix_mesh::WANIX_FS_ALPN`] — the Wanix↔Wanix mesh path, not
/// 9P. `aname` is threaded for symmetry with a future scoped-attach path; the
/// v1 native wire resolves the connection root from the verified `remote_id()`
/// alone and does not yet carry the attach name on the wire (so a default-deny
/// rejection surfaces lazily as a per-op transport fault, not at dial time).
///
/// The bound node is returned inside the [`IrohMount`] and **must outlive every
/// operation on the returned filesystem**: it owns the runtime the [`MeshFs`]
/// drives its QUIC traffic on, so dropping it early shuts that runtime down and
/// panics the first namespace op.
///
/// # Errors
///
/// Returns a CLI error when the node cannot bind, the ticket cannot be parsed,
/// or the QUIC dial fails.
pub(crate) fn dial_iroh_remote(addr: &str, aname: &str) -> Result<IrohMount, CliError> {
    let ticket = MeshTicket::parse(addr)?;
    // Dialing out only needs an endpoint; a fresh identity is fine.
    let identity = NodeIdentity::generate().map_err(|error| CliError::new(error.to_string(), 1))?;
    let node = MeshNode::bind(&identity)
        .map_err(|error| CliError::new(format!("failed to bind mesh endpoint: {error}"), 1))?;
    let node = node.with_deadline(CLI_MESH_MOUNT_DEADLINE);
    let remote = node
        .dialer()
        .dial_native_attach(ticket.endpoint_addr(), aname)
        .map_err(|error| CliError::new(format!("failed to dial iroh peer: {error}"), 1))?;
    Ok(IrohMount {
        remote,
        _node: node,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real, on-curve ed25519 public key hex (iroh validates the curve point,
    /// so an arbitrary 64-hex string is rejected by design).
    fn valid_peer_hex() -> String {
        NodeIdentity::from_secret_bytes([5u8; 32])
            .peer_id()
            .to_hex()
    }

    #[test]
    fn parses_a_bare_peer_ticket() {
        let hex = valid_peer_hex();
        let ticket = MeshTicket::parse(&format!("{IROH_SCHEME}{hex}")).unwrap();
        assert!(ticket.addrs.is_empty());
    }

    #[test]
    fn parses_direct_addresses_from_the_query() {
        let hex = valid_peer_hex();
        let ticket = MeshTicket::parse(&format!(
            "{IROH_SCHEME}{hex}?addr=127.0.0.1:5000&addr=10.0.0.2:6000"
        ))
        .unwrap();
        assert_eq!(ticket.addrs.len(), 2);
        assert_eq!(ticket.addrs[0], "127.0.0.1:5000".parse().unwrap());
    }

    #[test]
    fn rejects_a_wrong_scheme() {
        assert!(MeshTicket::parse("tcp://127.0.0.1:5640").is_err());
    }

    #[test]
    fn rejects_a_short_peer_id() {
        assert!(MeshTicket::parse(&format!("{IROH_SCHEME}deadbeef")).is_err());
    }

    #[test]
    fn rejects_an_off_curve_peer_id() {
        // 64 valid hex digits that are not a valid ed25519 public key.
        let hex = "ab".repeat(32);
        assert!(MeshTicket::parse(&format!("{IROH_SCHEME}{hex}")).is_err());
    }

    #[test]
    fn rejects_a_malformed_address_query() {
        let hex = valid_peer_hex();
        assert!(MeshTicket::parse(&format!("{IROH_SCHEME}{hex}?addr=not-an-addr")).is_err());
        assert!(MeshTicket::parse(&format!("{IROH_SCHEME}{hex}?host=127.0.0.1:1")).is_err());
    }
}
