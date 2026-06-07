//! `wanix-rust mesh-serve`: export a host directory as 9P over QUIC.
//!
//! Binds an iroh endpoint from the persisted node identity (`~/.wanix/node.key`
//! by default), serves `--root` over ALPN `wanix/9p/1`, prints the node id and a
//! dialable ticket, and blocks serving connections. With `--peer`/`--grant` it
//! installs a default-deny grant table keyed by the verified peer identity.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{FileSystem, LocalFs};
use wanix_id::{GrantTable, GrantTablePolicy, NodeIdentity};
use wanix_mesh::{MeshNode, ServeConfig};

use super::grant::{GrantSpec, build_grant_table, parse_peer_hex};
use crate::serve::services_namespace_for_root;
use crate::{CliError, write_process_output};

/// How long to wait for public-network connectivity before printing a ticket.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// A parsed `mesh-serve` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeshServeCommand {
    root_path: PathBuf,
    key_path: Option<PathBuf>,
    local_addr: Option<SocketAddr>,
    peer_hex: Option<String>,
    grants: Vec<GrantSpec>,
    /// Explicit opt-in to export the whole root read-write to *any* peer over
    /// the PUBLIC endpoint with no grant gate. Required because that inverts
    /// default-deny on a global transport (anyone with the ticket gets the
    /// directory). Ungranted public serving is refused without it.
    insecure_open: bool,
    /// Export a full Wanix services namespace (host root plus `#term`/`#pipe`/
    /// `#kv`/`#agent`/`#task`) rather than a bare host directory, so a remote
    /// node can operate node A's service devices as files over the mesh. This is
    /// the Slice 4 demo path: `/n/A/#kv/<key>` imports for free because `#kv` is a
    /// `FileSystem` bound into the served namespace.
    wanix_services: bool,
}

/// Parses `mesh-serve --root DIR [--key FILE] [--addr IP:PORT] [--peer HEX]
/// [--grant ANAME:PREFIX:RIGHTS]... [--insecure-open] [--wanix-services]`.
///
/// # Errors
///
/// Returns a usage error when `--root` is missing, an option lacks its value, a
/// grant/peer/address token is malformed, or the public endpoint would be
/// exported with no grants and no explicit `--insecure-open` opt-in.
pub(crate) fn parse_mesh_serve_command(
    args: &[std::ffi::OsString],
) -> Result<MeshServeCommand, CliError> {
    let mut root_path = None;
    let mut key_path = None;
    let mut local_addr = None;
    let mut peer_hex = None;
    let mut grants = Vec::new();
    let mut insecure_open = false;
    let mut wanix_services = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        let value = || {
            args.get(index + 1)
                .map(|value| value.to_string_lossy().into_owned())
                .ok_or_else(|| CliError::usage(format!("mesh-serve {flag} expects a value")))
        };
        match flag.as_str() {
            "--insecure-open" => {
                insecure_open = true;
                index += 1;
                continue;
            }
            "--wanix-services" => {
                wanix_services = true;
                index += 1;
                continue;
            }
            "--root" => root_path = Some(PathBuf::from(value()?)),
            "--key" => key_path = Some(PathBuf::from(value()?)),
            "--addr" => {
                local_addr = Some(value()?.parse::<SocketAddr>().map_err(|error| {
                    CliError::usage(format!("mesh-serve --addr must be IP:PORT: {error}"))
                })?);
            }
            "--peer" => peer_hex = Some(value()?),
            "--grant" => grants.push(GrantSpec::parse(&value()?)?),
            other => {
                return Err(CliError::usage(format!(
                    "unexpected mesh-serve argument: {other}"
                )));
            }
        }
        index += 2;
    }
    let root_path = root_path.ok_or_else(|| CliError::usage("mesh-serve requires --root DIR"))?;
    if !grants.is_empty() && peer_hex.is_none() {
        return Err(CliError::usage(
            "mesh-serve --grant requires --peer HEX to name the authorized peer",
        ));
    }
    // Default-deny on the global transport: serving the public endpoint with no
    // grant gate exports the whole root read-write to anyone holding the ticket.
    // Refuse it unless the operator explicitly opts in, or pins to a local
    // direct-address-only endpoint (--addr) where peers exchange tickets out of
    // band rather than discovering it via relays/DNS.
    let public = local_addr.is_none();
    if public && peer_hex.is_none() && !insecure_open {
        return Err(CliError::usage(
            "mesh-serve on the public endpoint with no --peer/--grant exports the entire \
             root read-write to anyone with the ticket; pass --peer HEX with --grant to \
             gate access, --addr IP:PORT to serve a local direct-address-only endpoint, or \
             --insecure-open to deliberately export it to the open internet",
        ));
    }
    // `--wanix-services` binds the exec devices `#task`/`#agent` (remote code
    // execution) into the served namespace, backed by the real QuickJs/Wasm task
    // drivers. The blueprint pins exec-device export to local-trust only: we do
    // NOT hand arbitrary NodeIDs code execution on the serving host until public
    // auth lands. The only local-trust endpoint here is `--addr IP:PORT` (a
    // direct-address-only socket whose ticket is exchanged out of band, not via
    // relays/DNS discovery). A public endpoint is reachable by any NodeID with
    // the ticket, so refuse services there regardless of --peer/--grant or
    // --insecure-open: a grant's backing is the same services namespace, so even
    // a grant-gated public serve would expose #task to the granted peer.
    if wanix_services && public {
        return Err(CliError::usage(
            "mesh-serve --wanix-services binds the #task/#agent exec devices (remote code \
             execution) into the served namespace; the blueprint keeps exec-device export \
             local-trust only, so it is refused on the public endpoint (reachable by any \
             NodeID with the ticket) even with --peer/--grant or --insecure-open. Pass \
             --addr IP:PORT to serve a local direct-address-only endpoint, or drop \
             --wanix-services to export only the host directory",
        ));
    }
    Ok(MeshServeCommand {
        root_path,
        key_path,
        local_addr,
        peer_hex,
        grants,
        insecure_open,
        wanix_services,
    })
}

/// Binds the node, prints its id/ticket, and serves until the process is killed.
///
/// # Errors
///
/// Returns a CLI error when the identity or root cannot be loaded, the endpoint
/// cannot bind, or a grant spec is invalid.
pub(crate) fn run_mesh_serve_streaming(
    command: MeshServeCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let identity = load_identity(&command)?;
    let root = load_root(&command)?;
    let mut node = bind_node(&command, &identity)?;
    let config = build_config(&command, &root)?;
    node.serve(config);
    announce(&command, &node, process_stderr)?;
    // Serving runs on the node's owned runtime; park the foreground thread so the
    // node (endpoint + router) stays alive until the process is terminated.
    loop {
        std::thread::park();
    }
}

fn load_identity(command: &MeshServeCommand) -> Result<NodeIdentity, CliError> {
    let path = match &command.key_path {
        Some(path) => path.clone(),
        None => {
            NodeIdentity::default_key_path().map_err(|error| CliError::new(error.to_string(), 1))?
        }
    };
    NodeIdentity::load_or_create(&path).map_err(|error| {
        CliError::new(
            format!("failed to load node identity at {path:?}: {error}"),
            1,
        )
    })
}

fn load_root(command: &MeshServeCommand) -> Result<Arc<dyn FileSystem>, CliError> {
    if command.wanix_services {
        // Export the full services namespace so `#kv`/`#term`/`#task` (and the
        // rest) cross the mesh: a remote node imports `/n/A/#kv/<key>` as files.
        return services_namespace_for_root(&command.root_path);
    }
    let root = LocalFs::new(&command.root_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open mesh-serve root {}: {error}",
                command.root_path.display()
            ),
            1,
        )
    })?;
    Ok(Arc::new(root))
}

fn bind_node(command: &MeshServeCommand, identity: &NodeIdentity) -> Result<MeshNode, CliError> {
    match command.local_addr {
        Some(addr) => MeshNode::bind_local(identity, addr),
        None => MeshNode::bind(identity),
    }
    .map_err(|error| CliError::new(format!("failed to bind mesh endpoint: {error}"), 1))
}

fn build_config(
    command: &MeshServeCommand,
    root: &Arc<dyn FileSystem>,
) -> Result<ServeConfig, CliError> {
    let Some(peer_hex) = &command.peer_hex else {
        return Ok(ServeConfig::open(Arc::clone(root)));
    };
    let peer = parse_peer_hex(peer_hex)?;
    let table = GrantTable::new();
    build_grant_table(&table, peer, &command.grants, Arc::clone(root));
    let policy = Arc::new(GrantTablePolicy::new(table));
    Ok(ServeConfig::guarded(Arc::clone(root), policy))
}

/// Prints the node id and a dialable `iroh://` ticket to stderr.
fn announce(
    command: &MeshServeCommand,
    node: &MeshNode,
    process_stderr: &mut dyn Write,
) -> Result<(), CliError> {
    if command.local_addr.is_none() {
        // Public mode: wait (bounded) for connectivity so the ticket is dialable.
        node.wait_online(ONLINE_TIMEOUT);
    }
    let peer = node.peer_id();
    let ticket = node.ticket();
    let direct: Vec<String> = ticket
        .ip_addrs()
        .map(|addr| format!("addr={addr}"))
        .collect();
    let query = if direct.is_empty() {
        String::new()
    } else {
        format!("?{}", direct.join("&"))
    };
    let mut message = format!(
        "wanix-rust mesh-serve: node {peer}\nwanix-rust mesh-serve: mount iroh://{peer}{query}\n"
    );
    if command.insecure_open && command.local_addr.is_none() {
        // Loud about the default-deny inversion the operator opted into. The
        // parser refuses --wanix-services on this public endpoint, so this
        // exports the host directory read-write only — it does NOT bind the
        // #task/#agent exec devices. Say so explicitly so the operator knows the
        // hazard is file read-write, not remote code execution.
        message.push_str(
            "wanix-rust mesh-serve: WARNING --insecure-open exports the entire root \
             read-write (file contents, not the #task/#agent exec devices) to anyone \
             with the ticket\n",
        );
    }
    write_process_output(process_stderr, "stderr", message.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parse_requires_root() {
        let error = parse_mesh_serve_command(&args(&["--addr", "127.0.0.1:0"])).unwrap_err();
        assert!(error.to_string().contains("mesh-serve requires --root DIR"));
    }

    #[test]
    fn parse_reads_root_addr_and_grants() {
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--addr",
            "127.0.0.1:7000",
            "--peer",
            &"ab".repeat(32),
            "--grant",
            "projects/foo:projects/foo:rw",
        ]))
        .unwrap();
        assert_eq!(command.root_path, PathBuf::from("/tmp/x"));
        assert_eq!(command.local_addr, Some("127.0.0.1:7000".parse().unwrap()));
        assert!(command.peer_hex.is_some());
        assert_eq!(command.grants.len(), 1);
    }

    #[test]
    fn parse_rejects_grant_without_peer() {
        let error =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--grant", "docs:docs:ro"]))
                .unwrap_err();
        assert!(error.to_string().contains("--grant requires --peer"));
    }

    #[test]
    fn parse_reports_missing_value() {
        let error = parse_mesh_serve_command(&args(&["--root"])).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("mesh-serve --root expects a value")
        );
    }

    #[test]
    fn parse_rejects_ungranted_public_serve() {
        // No --addr (public endpoint), no --peer/--grant, no --insecure-open:
        // this would export the whole root read-write to anyone on the internet.
        let error = parse_mesh_serve_command(&args(&["--root", "/tmp/x"])).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("exports the entire") && message.contains("--insecure-open"),
            "ungranted public serve must be refused with guidance, got {message:?}"
        );
    }

    #[test]
    fn parse_allows_local_open_serve() {
        // A local direct-address-only endpoint is fine without grants: peers
        // exchange the ticket out of band, not via relays/DNS discovery.
        let command =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--addr", "127.0.0.1:0"]))
                .unwrap();
        assert!(command.peer_hex.is_none());
        assert!(!command.insecure_open);
        assert_eq!(command.local_addr, Some("127.0.0.1:0".parse().unwrap()));
    }

    #[test]
    fn parse_allows_explicit_insecure_public_serve() {
        // The operator opts into the default-deny inversion explicitly.
        let command =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--insecure-open"])).unwrap();
        assert!(command.insecure_open);
        assert!(command.local_addr.is_none());
        assert!(command.peer_hex.is_none());
    }

    #[test]
    fn parse_reads_wanix_services_flag() {
        // The Slice 4 demo path: export the services namespace (host + #kv/#term/
        // #task) over the mesh so a remote node operates `#kv` as files.
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--addr",
            "127.0.0.1:0",
            "--wanix-services",
        ]))
        .unwrap();
        assert!(command.wanix_services);
        assert_eq!(command.local_addr, Some("127.0.0.1:0".parse().unwrap()));
    }

    #[test]
    fn parse_defaults_wanix_services_off() {
        let command =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--addr", "127.0.0.1:0"]))
                .unwrap();
        assert!(!command.wanix_services);
    }

    #[test]
    fn parse_refuses_wanix_services_on_public_endpoint() {
        // SECURITY: --wanix-services binds the #task/#agent exec devices (remote
        // code execution) into the served namespace. The blueprint keeps
        // exec-device export local-trust only. A public endpoint (no --addr) is
        // reachable by any NodeID with the ticket, so services there is refused.
        let error =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--wanix-services"])).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("#task") && message.contains("--addr"),
            "public --wanix-services must be refused naming the exec-device hazard, got {message:?}"
        );
    }

    #[test]
    fn parse_refuses_wanix_services_with_insecure_open() {
        // --insecure-open inverts default-deny for the host directory, but it must
        // NOT be a backdoor to exec-device export: remote code execution stays
        // local-trust only regardless of the file-sharing opt-in.
        let error = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--wanix-services",
            "--insecure-open",
        ]))
        .unwrap_err();
        assert!(
            error.to_string().contains("#task"),
            "--wanix-services --insecure-open must be refused naming the exec-device hazard"
        );
    }

    #[test]
    fn parse_refuses_wanix_services_with_public_grant() {
        // Even a grant-gated public serve exposes #task to the granted peer
        // (the grant's backing is the same services namespace), so exec-device
        // export over the public endpoint is refused with --peer/--grant too.
        let error = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--peer",
            &"ab".repeat(32),
            "--grant",
            "svc:.:rw",
            "--wanix-services",
        ]))
        .unwrap_err();
        assert!(
            error.to_string().contains("#task"),
            "grant-gated public --wanix-services must still be refused: it exposes #task"
        );
    }

    #[test]
    fn parse_allows_wanix_services_on_local_addr_endpoint() {
        // The one local-trust path: --addr IP:PORT is a direct-address-only socket
        // whose ticket is exchanged out of band, so exec-device export is allowed.
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--addr",
            "127.0.0.1:0",
            "--wanix-services",
        ]))
        .unwrap();
        assert!(command.wanix_services);
        assert_eq!(command.local_addr, Some("127.0.0.1:0".parse().unwrap()));
    }

    #[test]
    fn parse_allows_granted_public_serve() {
        // A grant-gated public serve is the intended secure default path.
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--peer",
            &"ab".repeat(32),
            "--grant",
            "docs:docs:ro",
        ]))
        .unwrap();
        assert!(command.local_addr.is_none());
        assert!(command.peer_hex.is_some());
        assert_eq!(command.grants.len(), 1);
    }
}
