//! One-resource-per-endpoint serve machinery (ADR 0007 §Resource Model).
//!
//! `volume serve` and `tool serve` share this shape: each served resource gets
//! its own native mesh endpoint with a distinct, stable identity, and the
//! process prints one parse-stable ticket record per resource. The caller owns
//! what each endpoint serves (a [`NativeServeConfig`]); this module owns the
//! binding loop, the per-resource identity files, and the announce-line format.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig};

use crate::CliError;

/// How long to wait for public-network connectivity before printing a ticket.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// A live served resource endpoint: its name, the held [`MeshNode`] (dropping
/// it shuts the endpoint down), and the dialable ticket.
pub(crate) struct ServedEndpoint {
    pub(crate) name: String,
    /// Held purely as a liveness keepalive: the endpoint serves as long as the
    /// node lives, so it is never read in production (only in tests, via
    /// `peer_id()`), but dropping it shuts the endpoint down.
    #[allow(dead_code)]
    pub(crate) node: MeshNode,
    pub(crate) ticket_url: String,
}

/// Binds one native mesh endpoint per `(name, identity, config)` — one resource
/// per endpoint, never an aggregate root. The caller must hold the returned
/// [`ServedEndpoint`]s (their nodes) for as long as the endpoints serve.
///
/// # Errors
///
/// Returns a CLI error when an endpoint cannot be bound.
pub(crate) fn bind_endpoints(
    noun: &str,
    resources: Vec<(String, NodeIdentity, NativeServeConfig)>,
    local_addr: Option<SocketAddr>,
) -> Result<Vec<ServedEndpoint>, CliError> {
    let public = local_addr.is_none();
    let mut served = Vec::with_capacity(resources.len());
    for (name, identity, config) in resources {
        let mut node = match local_addr {
            Some(addr) => MeshNode::bind_local(&identity, addr),
            None => MeshNode::bind(&identity),
        }
        .map_err(|error| {
            CliError::new(
                format!("failed to bind mesh endpoint for {noun} {name:?}: {error}"),
                1,
            )
        })?;
        node.serve_native(config);
        if public {
            node.wait_online(ONLINE_TIMEOUT);
        }
        let ticket_url = ticket_url(&node);
        served.push(ServedEndpoint {
            name,
            node,
            ticket_url,
        });
    }
    Ok(served)
}

/// Multiple resources cannot share one fixed port; require port 0 (a unique
/// ephemeral port per endpoint) when serving more than one.
pub(crate) fn reject_fixed_port_multi(
    command: &str,
    noun: &str,
    local_addr: Option<SocketAddr>,
    count: usize,
) -> Result<(), CliError> {
    if count > 1 && local_addr.is_some_and(|addr| addr.port() != 0) {
        return Err(CliError::usage(format!(
            "{command}: serving {count} {noun}s needs a unique port per endpoint; pass --addr \
             with port 0 (e.g. 127.0.0.1:0) instead of a fixed port"
        )));
    }
    Ok(())
}

/// One announce line for a served resource: `NAME\tTICKET_URL\n`. Tab-separated
/// and newline-terminated so a later catalog-register step can parse it stably.
pub(crate) fn serve_record_line(name: &str, ticket_url: &str) -> String {
    format!("{name}\t{ticket_url}\n")
}

/// A copy-pasteable client command for a served resource, printed beside the
/// stable tab record. Starts with `# ` so record parsers skip it as a comment.
pub(crate) fn serve_record_example_line(ticket_url: &str) -> String {
    format!("# mount with: wanix-rust mount-ls '{ticket_url}'\n")
}

/// Builds the dialable `iroh://<peer>?addr=...` ticket for a bound node — the
/// inverse of [`crate::mesh::MeshTicket::parse`].
fn ticket_url(node: &MeshNode) -> String {
    let peer = node.peer_id();
    let addrs: Vec<String> = node
        .ticket()
        .ip_addrs()
        .map(|addr| format!("addr={addr}"))
        .collect();
    if addrs.is_empty() {
        format!("{}{peer}", crate::mesh::IROH_SCHEME)
    } else {
        format!("{}{peer}?{}", crate::mesh::IROH_SCHEME, addrs.join("&"))
    }
}

/// Loads or creates an owner-private per-resource endpoint identity at `path`,
/// creating the parent directory first (`load_or_create` does not).
///
/// Identity key files live under `~/.wanix/<kind>-identities/<name>.key`,
/// deliberately OUTSIDE any served root so a resource's own endpoint secret key
/// is never exported to peers that mount it.
pub(crate) fn load_identity_at(path: &Path) -> Result<NodeIdentity, CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CliError::new(
                format!(
                    "failed to create identity dir {}: {error}",
                    parent.display()
                ),
                1,
            )
        })?;
    }
    NodeIdentity::load_or_create(path).map_err(|error| {
        CliError::new(
            format!("failed to load identity {}: {error}", path.display()),
            1,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{reject_fixed_port_multi, serve_record_example_line, serve_record_line};
    use std::net::SocketAddr;

    #[test]
    fn serve_record_line_is_a_stable_tab_record() {
        // Pinned so a later catalog-register step can parse it: NAME\tTICKET\n.
        assert_eq!(
            serve_record_line("notes", "iroh://abc?addr=127.0.0.1:5610"),
            "notes\tiroh://abc?addr=127.0.0.1:5610\n"
        );
    }

    #[test]
    fn serve_record_example_line_is_a_copy_pasteable_comment() {
        // The human-facing example is a `# ` comment (record parsers skip it)
        // and pastes directly into the matching client command.
        let line = serve_record_example_line("iroh://abc?addr=127.0.0.1:5610");
        assert!(line.starts_with("# "), "{line}");
        assert!(!line.contains('\t'), "{line}");
        assert!(
            line.contains("wanix-rust mount-ls 'iroh://abc?addr=127.0.0.1:5610'"),
            "{line}"
        );
    }

    #[test]
    fn fixed_port_rule_rejects_multiple_resources() {
        let fixed: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        // A fixed nonzero port cannot serve more than one resource.
        let error = reject_fixed_port_multi("tool serve", "tool", Some(fixed), 2).unwrap_err();
        assert!(error.to_string().contains("tool serve: serving 2 tools"));
        // One resource on a fixed port, port 0 for many, or a public endpoint
        // are fine.
        assert!(reject_fixed_port_multi("tool serve", "tool", Some(fixed), 1).is_ok());
        assert!(
            reject_fixed_port_multi("tool serve", "tool", "127.0.0.1:0".parse().ok(), 2).is_ok()
        );
        assert!(reject_fixed_port_multi("tool serve", "tool", None, 2).is_ok());
    }
}
