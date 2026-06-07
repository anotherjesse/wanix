//! The `iroh://` ticket scheme: parse a peer address and dial it over QUIC.
//!
//! A ticket is `iroh://<64-hex-peer-id>` for relay/DNS-discovered dialing on the
//! public network, optionally with `?addr=IP:PORT` query parameters carrying
//! direct addresses for LAN or offline first contact. The blueprint prefers a
//! ticket with direct addresses for first contact because `online()` does not
//! guarantee dialability immediately.

use std::net::SocketAddr;
use std::sync::Arc;

use wanix_id::NodeIdentity;
use wanix_mesh::{EndpointAddr, EndpointId, MeshNode};

use crate::CliError;

/// URL scheme that selects a QUIC mesh dial instead of a raw TCP 9P dial.
pub(crate) const IROH_SCHEME: &str = "iroh://";

/// A parsed `iroh://` dial target: a verified peer id plus optional direct
/// addresses for first contact.
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
    fn endpoint_addr(&self) -> EndpointAddr {
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
/// [`RemoteFs`] drives every operation through. Its [`crate::mesh::BlockingDuplex`]
/// holds only a non-owning runtime [`tokio::runtime::Handle`], so dropping the
/// node shuts the runtime down and the next namespace op would panic ("a Tokio
/// 1.x context was found, but it is being shutdown"). The node is therefore
/// returned alongside the `RemoteFs` and **must be kept alive for the mount's
/// whole lifetime** — every read/write/walk through the mount runs on it.
#[must_use]
pub(crate) struct IrohMount {
    /// The imported remote filesystem, bound into a namespace by the caller.
    pub(crate) remote: Arc<wanix_9p_client::RemoteFs>,
    /// The live dialer node; kept alive for the mount's lifetime. Held only to
    /// keep the runtime the `remote` drives its operations on from shutting down.
    _node: MeshNode,
}

/// Dials an `iroh://` ticket and returns a connected mount over QUIC.
///
/// A fresh dialer-only [`MeshNode`] is bound from an ephemeral identity (the
/// importer's stable identity is not required to dial out), then the ticket is
/// dialed. `aname` selects the named subtree to attach, empty for the root.
///
/// The bound node is returned inside the [`IrohMount`] and **must outlive every
/// operation on the returned filesystem**: it owns the runtime the `RemoteFs`
/// drives its QUIC traffic on, so dropping it early shuts that runtime down and
/// panics the first namespace op.
///
/// # Errors
///
/// Returns a CLI error when the node cannot bind, the ticket cannot be parsed,
/// or the QUIC dial/9P negotiation fails.
pub(crate) fn dial_iroh_remote(addr: &str, aname: &str) -> Result<IrohMount, CliError> {
    let ticket = MeshTicket::parse(addr)?;
    // Dialing out only needs an endpoint; a fresh identity is fine.
    let identity = NodeIdentity::generate().map_err(|error| CliError::new(error.to_string(), 1))?;
    let node = MeshNode::bind(&identity)
        .map_err(|error| CliError::new(format!("failed to bind mesh endpoint: {error}"), 1))?;
    let remote = node
        .dialer()
        .dial_attach(ticket.endpoint_addr(), aname)
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
