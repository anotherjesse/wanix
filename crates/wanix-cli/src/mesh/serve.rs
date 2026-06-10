//! `wanix mesh-serve`: export a host directory over the native mesh wire.
//!
//! Binds an iroh endpoint from the persisted node identity (`~/.wanix/node.key`
//! by default), serves `--root` over the native `wanix-mesh-wire` plane (ALPN
//! [`wanix_mesh::WANIX_FS_ALPN`]) — the Wanix↔Wanix path the `mount iroh://`
//! client dials — prints the node id and a dialable ticket, and blocks serving
//! connections. With `--peer`/`--grant` it installs a default-deny grant table
//! keyed by the verified peer identity. (9P stays at the foreign edge: the
//! `tcp://` mount path and `serve --p9`.)

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{FileSystem, LocalFs};
use wanix_id::{GrantTable, GrantTablePolicy, NodeIdentity};
use wanix_mesh::{MeshNode, NativeServeConfig};

use super::grant::{GrantSpec, build_grant_table, parse_peer_hex};
use crate::serve::services_namespace_for_root;
use crate::{CliError, write_process_output};

/// How long to wait for public-network connectivity before printing a ticket.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// What `mesh-serve` exports: a host directory (`--root`) or a named persistent
/// volume (`--volume`, resolved under `~/.wanix/volumes`). The two are mutually
/// exclusive. Per ADR 0007 a volume serve is the single-volume shorthand — one
/// endpoint, one resource root — not an aggregate `/vol/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ServeRoot {
    Dir(PathBuf),
    Volume(String),
}

/// A parsed `mesh-serve` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MeshServeCommand {
    root: ServeRoot,
    key_path: Option<PathBuf>,
    local_addr: Option<SocketAddr>,
    peer_hex: Option<String>,
    grants: Vec<GrantSpec>,
    /// Explicit opt-in to export the whole root read-write to *any* peer over
    /// a non-loopback binding (the public endpoint, or a LAN `--addr`) with no
    /// grant gate. Required because that inverts default-deny on an open
    /// network (anyone who can reach the socket gets the directory). Ungranted
    /// non-loopback serving is refused without it.
    insecure_open: bool,
    /// Export a full Wanix services namespace (host root plus `#term`/`#pipe`/
    /// `#kv`/`#agent`/`#task`) rather than a bare host directory, so a remote
    /// node can operate node A's service devices as files over the mesh. This is
    /// the Slice 4 demo path: `/n/A/#kv/<key>` imports for free because `#kv` is a
    /// `FileSystem` bound into the served namespace.
    wanix_services: bool,
    /// Serve the `#cpu` exec plane (ALPN `wanix/cpu/1`) beside the namespace:
    /// an admitted peer runs a `qjs`/`wasm` task ON THIS HOST against its own
    /// reverse-exported namespace. Remote code execution, so it follows the
    /// exec-device rule: refused on any non-loopback endpoint entirely, and
    /// scoped to the `--peer` identity when one is named.
    cpu: bool,
}

/// Parses `mesh-serve (--root DIR | --volume NAME) [--key FILE] [--addr IP:PORT]
/// [--peer HEX] [--grant ANAME:PREFIX:RIGHTS]... [--insecure-open]
/// [--wanix-services] [--cpu]`.
///
/// # Errors
///
/// Returns a usage error when neither (or both) of `--root`/`--volume` is given,
/// an option lacks its value, a grant/peer/address token is malformed, a
/// non-loopback endpoint would be exported with no grants and no explicit
/// `--insecure-open` opt-in, or `--wanix-services`/`--cpu` (exec export) is
/// requested off loopback.
pub(crate) fn parse_mesh_serve_command(
    args: &[std::ffi::OsString],
) -> Result<MeshServeCommand, CliError> {
    let mut root_path = None;
    let mut volume_name = None;
    let mut key_path = None;
    let mut local_addr = None;
    let mut peer_hex = None;
    let mut grants = Vec::new();
    let mut insecure_open = false;
    let mut wanix_services = false;
    let mut cpu = false;
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
            "--cpu" => {
                cpu = true;
                index += 1;
                continue;
            }
            "--root" => root_path = Some(PathBuf::from(value()?)),
            "--volume" => volume_name = Some(value()?),
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
    let root = match (root_path, volume_name) {
        (Some(_), Some(_)) => {
            return Err(CliError::usage(
                "mesh-serve --root and --volume are mutually exclusive",
            ));
        }
        (Some(path), None) => ServeRoot::Dir(path),
        (None, Some(name)) => ServeRoot::Volume(name),
        (None, None) => {
            return Err(CliError::usage(
                "mesh-serve requires --root DIR or --volume NAME",
            ));
        }
    };
    if !grants.is_empty() && peer_hex.is_none() {
        return Err(CliError::usage(
            "mesh-serve --grant requires --peer HEX to name the authorized peer",
        ));
    }
    // Trust tiers. The public endpoint (no --addr) is reachable by any NodeID
    // holding the ticket. A NON-loopback --addr (0.0.0.0 or a LAN IP) is not
    // local trust either: `MeshNode::bind_local` keeps mDNS advertise on, so
    // the stable NodeID is discoverable by every LAN host and the "ticket
    // exchanged out of band" assumption fails there. Only a loopback --addr —
    // a socket other hosts cannot reach at all — is the local-trust tier,
    // mirroring the ADR 0006 serve rule (`is_loopback_addr`) that refuses
    // exec devices on any non-loopback door.
    let loopback = local_addr.is_some_and(|addr| addr.ip().is_loopback());
    // `--wanix-services` binds the exec devices `#task`/`#agent` (remote code
    // execution) into the served namespace, backed by the real QuickJs/Wasm task
    // drivers. The blueprint pins exec-device export to local-trust only: we do
    // NOT hand arbitrary NodeIDs code execution on the serving host until public
    // auth lands, so refuse services off loopback regardless of --peer/--grant
    // or --insecure-open: a grant's backing is the same services namespace, so
    // even a grant-gated open serve would expose #task to the granted peer. This
    // sharper refusal is checked before the generic open-serve one so the
    // operator hears about the exec-device hazard, not just the file export.
    if wanix_services && !loopback {
        return Err(CliError::usage(
            "mesh-serve --wanix-services binds the #task/#agent exec devices (remote code \
             execution) into the served namespace; the blueprint keeps exec-device export \
             local-trust only, so it is refused on any non-loopback endpoint (the public \
             endpoint is reachable by any NodeID with the ticket; a LAN --addr is \
             mDNS-discoverable by any LAN host) even with --peer/--grant or \
             --insecure-open. Pass --addr 127.0.0.1:PORT to serve a loopback-only \
             endpoint, or drop --wanix-services to export only the host directory",
        ));
    }
    // `--cpu` binds the cpu exec plane (ALPN `wanix/cpu/1`): an admitted peer
    // runs arbitrary task code ON THIS HOST. That is the sharpest capability on
    // the mesh — sharper than `--wanix-services`, which at least confines the
    // peer to this node's service files — so it follows the same exec-device
    // rule: local-trust (loopback) only, regardless of --peer/--grant or
    // --insecure-open.
    if cpu && !loopback {
        return Err(CliError::usage(
            "mesh-serve --cpu binds the #cpu exec plane (remote code execution on this \
             host); the blueprint keeps exec export local-trust only, so it is refused on \
             any non-loopback endpoint (the public endpoint is reachable by any NodeID \
             with the ticket; a LAN --addr is mDNS-discoverable by any LAN host) even \
             with --peer/--grant or --insecure-open. Pass --addr 127.0.0.1:PORT to serve \
             a loopback-only endpoint (add --peer HEX to admit only that identity), or \
             drop --cpu",
        ));
    }
    // Default-deny on an open-network binding: serving the public endpoint — or
    // a non-loopback --addr, whose NodeID mDNS advertises to the LAN — with no
    // grant gate exports the whole root read-write to anyone who can reach it.
    // Refuse it unless the operator explicitly opts in, or pins to a loopback
    // --addr where the socket itself is unreachable from other hosts.
    if !loopback && peer_hex.is_none() && !insecure_open {
        return Err(CliError::usage(
            "mesh-serve on a non-loopback endpoint (public, or a LAN --addr whose node id \
             mDNS advertises) with no --peer/--grant exports the entire root read-write \
             to anyone who can reach it; pass --peer HEX with --grant to gate access, \
             --addr 127.0.0.1:PORT to serve a loopback-only endpoint, or --insecure-open \
             to deliberately export it open",
        ));
    }
    Ok(MeshServeCommand {
        root,
        key_path,
        local_addr,
        peer_hex,
        grants,
        insecure_open,
        wanix_services,
        cpu,
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
    let node = build_and_serve(&command, &identity, &root)?;
    announce(&command, &node, process_stderr)?;
    // Serving runs on the node's owned runtime; park the foreground thread so the
    // node (endpoint + router) stays alive until the process is terminated.
    loop {
        std::thread::park();
    }
}

/// Binds the node and starts serving `root` over the **native** `wanix-mesh-wire`
/// plane ([`wanix_mesh::WANIX_FS_ALPN`]).
///
/// Factored out of [`run_mesh_serve_streaming`] (which then only announces and
/// parks) so a test can drive the real serve-config selection against the real
/// [`crate::mesh::dial_iroh_remote`] client without the park-forever loop. The
/// `node.serve_native` call is type-locked to [`build_config`]'s
/// [`NativeServeConfig`]: reverting to the 9P `node.serve` would not compile.
///
/// With `--cpu` the exec acceptor registers beside the namespace plane on one
/// router (`serve_native_with_cpu`); the parser has already refused the public
/// endpoint, and the acceptor's allowlist is the `--peer` identity when named.
fn build_and_serve(
    command: &MeshServeCommand,
    identity: &NodeIdentity,
    root: &Arc<dyn FileSystem>,
) -> Result<MeshNode, CliError> {
    let mut node = bind_node(command, identity)?;
    let config = build_config(command, root)?;
    if command.cpu {
        let peer = command
            .peer_hex
            .as_deref()
            .map(parse_peer_hex)
            .transpose()?;
        let acceptor = super::serve_cpu::cpu_acceptor_for(&node, peer)?;
        node.serve_native_with_cpu(config, acceptor);
    } else {
        node.serve_native(config);
    }
    Ok(node)
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
    let root_dir = resolve_serve_root(command)?;
    if command.wanix_services {
        // Export the full services namespace so `#kv`/`#term`/`#task` (and the
        // rest) cross the mesh: a remote node imports `/n/A/#kv/<key>` as files.
        return services_namespace_for_root(&root_dir);
    }
    let root = LocalFs::new(&root_dir).map_err(|error| {
        CliError::new(
            format!(
                "failed to open mesh-serve root {}: {error}",
                root_dir.display()
            ),
            1,
        )
    })?;
    Ok(Arc::new(root))
}

/// Resolves the served root to a host directory: `--root` is used verbatim;
/// `--volume NAME` resolves under `~/.wanix/volumes` and must already exist.
fn resolve_serve_root(command: &MeshServeCommand) -> Result<PathBuf, CliError> {
    match &command.root {
        ServeRoot::Dir(path) => Ok(path.clone()),
        ServeRoot::Volume(name) => {
            crate::volume::resolve_existing_volume(&crate::volume::volumes_root()?, name)
        }
    }
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
) -> Result<NativeServeConfig, CliError> {
    let Some(peer_hex) = &command.peer_hex else {
        return Ok(NativeServeConfig::open(Arc::clone(root)));
    };
    let peer = parse_peer_hex(peer_hex)?;
    let table = GrantTable::new();
    build_grant_table(&table, peer, &command.grants, Arc::clone(root));
    let policy = Arc::new(GrantTablePolicy::new(table));
    Ok(NativeServeConfig::guarded(Arc::clone(root), policy))
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
    let insecure_public = command.insecure_open && command.local_addr.is_none();
    let message = announce_message(&peer.to_hex(), &query, insecure_public, command.cpu);
    write_process_output(process_stderr, "stderr", message.as_bytes())
}

/// Builds the announce text: the node id, the dialable ticket, and a directly
/// copy-pasteable client command (whatever a server prints should paste into
/// the matching mount command, not just be a bare ticket).
fn announce_message(peer_hex: &str, query: &str, insecure_public: bool, cpu: bool) -> String {
    let ticket_url = format!("iroh://{peer_hex}{query}");
    let mut message = format!(
        "wanix mesh-serve: node {peer_hex}\n\
         wanix mesh-serve: ticket {ticket_url}\n\
         wanix mesh-serve: mount with: wanix mount-ls '{ticket_url}'\n"
    );
    if cpu {
        // Name the hazard and the matching client command: serving #cpu means an
        // admitted peer runs task code on this host against its own reverse
        // export.
        message.push_str(&format!(
            "wanix mesh-serve: serving the #cpu exec plane (remote code execution \
             for admitted peers); run a job with: wanix cpu --node '{ticket_url}' \
             -- qjs PROGRAM\n"
        ));
    }
    if insecure_public {
        // Loud about the default-deny inversion the operator opted into. The
        // parser refuses --wanix-services on this public endpoint, so this
        // exports the host directory read-write only — it does NOT bind the
        // #task/#agent exec devices. Say so explicitly so the operator knows the
        // hazard is file read-write, not remote code execution.
        message.push_str(
            "wanix mesh-serve: WARNING --insecure-open exports the entire root \
             read-write (file contents, not the #task/#agent exec devices) to anyone \
             with the ticket\n",
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    /// End-to-end regression guard: the production serve path and the production
    /// `mount iroh://` client must speak the SAME wire.
    ///
    /// The bug this catches: `mesh-serve` served the 9P plane (`node.serve`)
    /// while `dial_iroh_remote` dialed the native plane (`dial_native`), so every
    /// CLI dial failed with "peer doesn't support any known protocol". Each half
    /// HAD a test — but each tested against a hand-rolled counterpart that matched
    /// its own ALPN (the integration test in `tests/mesh_iroh.rs` hand-builds the
    /// server with `serve_native` and *mirrors* `dial_iroh_remote`), so the drift
    /// between the real server and the real client was invisible. This drives
    /// `build_and_serve` (the exact serve call the foreground makes) against the
    /// real [`crate::mesh::dial_iroh_remote`] over loopback QUIC and round-trips a
    /// file. Reverting the server to 9P fails to compile (type-locked); diverging
    /// the dialer's ALPN fails this at runtime.
    #[test]
    fn mesh_serve_and_mount_iroh_speak_the_same_wire() {
        use wanix_fs::{MemFs, NormalizedPath, OpenOptions};
        use wanix_vfs::{BindOptions, Namespace};

        use crate::mesh::IROH_SCHEME;

        // A real parsed command on a local direct-address endpoint, then the real
        // serve call — serving an in-memory root so the test needs no temp dir.
        let command =
            parse_mesh_serve_command(&args(&["--root", "/unused", "--addr", "127.0.0.1:0"]))
                .unwrap();
        let identity = NodeIdentity::from_secret_bytes([42u8; 32]);
        let host = Arc::new(MemFs::new());
        let root: Arc<dyn FileSystem> = host.clone();
        let server = build_and_serve(&command, &identity, &root).unwrap();

        // Build the dialable ticket string exactly as `announce` prints it for the
        // operator, then dial it through the REAL CLI client.
        let peer = server.peer_id();
        let addrs: Vec<String> = server
            .ticket()
            .ip_addrs()
            .map(|addr| format!("addr={addr}"))
            .collect();
        assert!(
            !addrs.is_empty(),
            "loopback ticket must carry a direct addr"
        );
        let url = format!("{IROH_SCHEME}{peer}?{}", addrs.join("&"));

        let mount = crate::mesh::dial_iroh_remote(&url, "").unwrap();
        let mut namespace = Namespace::new();
        namespace
            .bind(
                mount.remote.clone(),
                ".",
                "n/remote",
                BindOptions::default(),
            )
            .unwrap();

        // Write across the native wire (synchronous request/reply), confirm it
        // landed on the served root, then read it back through the mount.
        let payload = b"step 0 across the native wire";
        let p = NormalizedPath::new("n/remote/hello.txt").unwrap();
        let mut file = namespace
            .open(
                &p,
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        assert_eq!(file.write(payload).unwrap(), payload.len());
        drop(file);

        assert_eq!(host.read_file("hello.txt").unwrap(), payload);

        let mut got = Vec::new();
        let mut reader = namespace.open(&p, OpenOptions::read()).unwrap();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = reader.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            got.extend_from_slice(&chunk[..read]);
            assert!(got.len() < (1 << 20), "stream grew unbounded");
        }
        assert_eq!(got, payload);

        // The dialer node (inside `mount`) and the server own the runtimes their
        // ops ride on; hold both until every assertion above has run.
        drop(mount);
        drop(server);
    }

    /// Address-model guarantee (better-iroh-discovery.md): `addr=` is only a
    /// route HINT, never identity. Dialing one peer's id at a different peer's
    /// socket must FAIL the cryptographic identity check, never silently mount the
    /// peer that happens to live at that address. The bad outcome is "route
    /// failed", never "mounted the wrong resource".
    #[test]
    fn wrong_direct_route_fails_identity_and_never_mounts_another_peer() {
        use wanix_fs::{MemFs, NormalizedPath, OpenOptions};
        use wanix_vfs::{BindOptions, Namespace};

        use crate::mesh::IROH_SCHEME;

        // One real server B, with a recognizable marker file.
        let host_b = Arc::new(MemFs::new());
        host_b.write_file("who.txt", b"server-B").unwrap();
        let identity_b = NodeIdentity::from_secret_bytes([21u8; 32]);
        let server_b = build_and_serve(
            &parse_mesh_serve_command(&args(&["--root", "/unused", "--addr", "127.0.0.1:0"]))
                .unwrap(),
            &identity_b,
            &(host_b.clone() as Arc<dyn FileSystem>),
        )
        .unwrap();
        let b_query: String = server_b
            .ticket()
            .ip_addrs()
            .map(|addr| format!("addr={addr}"))
            .collect::<Vec<_>>()
            .join("&");
        assert!(
            !b_query.is_empty(),
            "loopback ticket must carry a direct addr"
        );

        // A DIFFERENT peer id pointed at B's socket. `addr=` is a hint for that id;
        // since B cannot authenticate as this id, the dial must error — not mount B.
        let other_peer = NodeIdentity::from_secret_bytes([22u8; 32]).peer_id();
        let wrong = format!("{IROH_SCHEME}{other_peer}?{b_query}");
        assert!(
            crate::mesh::dial_iroh_remote(&wrong, "").is_err(),
            "a wrong-identity direct route must fail, never mount the peer at that address"
        );

        // Positive control: the CORRECT id at the same hint dials and mounts B.
        // Proves the failure above was identity verification, not a dead socket,
        // and that `addr=` is a usable route hint for the right peer.
        let right = format!("{IROH_SCHEME}{}?{b_query}", server_b.peer_id());
        let mount = crate::mesh::dial_iroh_remote(&right, "").unwrap();
        let mut namespace = Namespace::new();
        namespace
            .bind(mount.remote.clone(), ".", "n", BindOptions::default())
            .unwrap();
        let mut file = namespace
            .open(
                &NormalizedPath::new("n/who.txt").unwrap(),
                OpenOptions::read(),
            )
            .unwrap();
        let mut got = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = file.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            got.extend_from_slice(&chunk[..read]);
        }
        assert_eq!(got, b"server-B");

        drop(mount);
        drop(server_b);
    }

    #[test]
    fn announce_prints_a_copy_pasteable_mount_command() {
        let message = announce_message("ab12", "?addr=127.0.0.1:5610", false, false);
        assert!(message.contains("node ab12\n"), "{message}");
        assert!(
            message.contains("ticket iroh://ab12?addr=127.0.0.1:5610\n"),
            "{message}"
        );
        // Whatever the server prints must paste into the matching client command.
        assert!(
            message.contains("wanix mount-ls 'iroh://ab12?addr=127.0.0.1:5610'"),
            "{message}"
        );
        assert!(!message.contains("WARNING"), "{message}");
        assert!(!message.contains("#cpu"), "{message}");
    }

    #[test]
    fn announce_warns_about_insecure_public_export() {
        let message = announce_message("ab12", "", true, false);
        assert!(message.contains("WARNING --insecure-open"), "{message}");
    }

    #[test]
    fn announce_names_the_cpu_exec_plane_and_its_client_command() {
        // Serving #cpu is remote code execution; the operator must hear it, and
        // the printed line must paste into the matching dial verb.
        let message = announce_message("ab12", "?addr=127.0.0.1:5610", false, true);
        assert!(message.contains("remote code execution"), "{message}");
        assert!(
            message.contains("wanix cpu --node 'iroh://ab12?addr=127.0.0.1:5610'"),
            "{message}"
        );
    }

    #[test]
    fn parse_requires_root_or_volume() {
        let error = parse_mesh_serve_command(&args(&["--addr", "127.0.0.1:0"])).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("mesh-serve requires --root DIR or --volume NAME"),
            "got {error}"
        );
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
        assert_eq!(command.root, ServeRoot::Dir(PathBuf::from("/tmp/x")));
        assert_eq!(command.local_addr, Some("127.0.0.1:7000".parse().unwrap()));
        assert!(command.peer_hex.is_some());
        assert_eq!(command.grants.len(), 1);
    }

    #[test]
    fn parse_accepts_volume_as_root_source() {
        let command =
            parse_mesh_serve_command(&args(&["--volume", "notes", "--addr", "127.0.0.1:0"]))
                .unwrap();
        assert_eq!(command.root, ServeRoot::Volume("notes".to_owned()));
    }

    #[test]
    fn parse_rejects_root_and_volume_together() {
        let error = parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--volume", "notes"]))
            .unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(
            error.to_string().contains("mutually exclusive"),
            "got {error}"
        );
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
        // A loopback endpoint is fine without grants: the socket is
        // unreachable from any other host.
        let command =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--addr", "127.0.0.1:0"]))
                .unwrap();
        assert!(command.peer_hex.is_none());
        assert!(!command.insecure_open);
        assert_eq!(command.local_addr, Some("127.0.0.1:0".parse().unwrap()));
    }

    #[test]
    fn parse_rejects_ungranted_non_loopback_addr_serve() {
        // SECURITY: a non-loopback --addr is NOT the local-trust tier —
        // `bind_local` keeps mDNS advertise on, so the stable NodeID is
        // discoverable by any LAN host and an ungranted serve is the whole
        // root read-write to the LAN. Same default-deny as the public door.
        for addr in ["0.0.0.0:5000", "10.0.0.5:7000"] {
            let error =
                parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--addr", addr])).unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains("exports the entire") && message.contains("--insecure-open"),
                "ungranted non-loopback --addr {addr} must be refused, got {message:?}"
            );
        }
    }

    #[test]
    fn parse_allows_granted_non_loopback_addr_serve() {
        // The gated file plane stays available on a LAN --addr: --peer/--grant
        // scopes it to one verified identity (no exec flags involved).
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--addr",
            "0.0.0.0:0",
            "--peer",
            &"ab".repeat(32),
            "--grant",
            "docs:docs:ro",
        ]))
        .unwrap();
        assert_eq!(command.local_addr, Some("0.0.0.0:0".parse().unwrap()));
        assert!(command.peer_hex.is_some());
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
        // The one local-trust path: a LOOPBACK --addr, a socket no other host
        // can reach, so exec-device export is allowed.
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
    fn parse_refuses_wanix_services_on_non_loopback_addr() {
        // SECURITY: `--addr 0.0.0.0:PORT` (or a LAN IP) is reachable from the
        // LAN and mDNS advertises the NodeID, so exec devices there would be
        // unauthenticated LAN remote code execution. Refused even with --peer,
        // mirroring the ADR 0006 non-loopback exec-door rule.
        for addr in ["0.0.0.0:5000", "10.0.0.5:7000"] {
            let error = parse_mesh_serve_command(&args(&[
                "--root",
                "/tmp/x",
                "--addr",
                addr,
                "--wanix-services",
            ]))
            .unwrap_err();
            assert!(
                error.to_string().contains("#task"),
                "non-loopback --addr {addr} must refuse exec devices, got {error}"
            );
        }
    }

    /// The serve half the docs called "dial-only" until now: `mesh-serve --cpu`
    /// binds the [`wanix_mesh::CpuAcceptor`] beside the namespace plane on one
    /// router, and the REAL CLI dial verb (`wanix cpu`) runs a qjs job on
    /// this node against the caller's reverse-exported cwd — output and exit
    /// observable on the caller's process streams, with the namespace plane
    /// still mountable on the same ticket.
    #[test]
    fn mesh_serve_cpu_runs_a_dialed_job_against_the_callers_reverse_export() {
        use wanix_fs::{MemFs, NormalizedPath};

        use crate::mesh::IROH_SCHEME;

        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/unused",
            "--addr",
            "127.0.0.1:0",
            "--cpu",
        ]))
        .unwrap();
        let identity = NodeIdentity::from_secret_bytes([61u8; 32]);
        let host = Arc::new(MemFs::new());
        host.write_file("hello.txt", b"fs plane lives").unwrap();
        let root: Arc<dyn FileSystem> = host.clone();
        let server = build_and_serve(&command, &identity, &root).unwrap();

        // The caller's working directory holds the program; the job reads it
        // through the caller's reverse export, never the served root.
        let cwd = std::env::temp_dir().join(format!("wanix-cpu-serve-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(
            cwd.join("build.js"),
            "import * as std from 'qjs:std';\nstd.out.puts('ran over mesh-serve --cpu\\n');\n",
        )
        .unwrap();

        let addrs: Vec<String> = server
            .ticket()
            .ip_addrs()
            .map(|addr| format!("addr={addr}"))
            .collect();
        let url = format!("{IROH_SCHEME}{}?{}", server.peer_id(), addrs.join("&"));

        // The real dial verb, parsed by the real grammar.
        let cpu_command = crate::cpu::parse_cpu_command(&args(&[
            "--node",
            &url,
            "--cwd",
            cwd.to_str().unwrap(),
            "--",
            "qjs",
            "build.js",
        ]))
        .unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let exit = crate::cpu::run_cpu_streaming(cpu_command, &mut out, &mut err).unwrap();
        assert_eq!(exit, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert_eq!(
            String::from_utf8_lossy(&out).trim(),
            "ran over mesh-serve --cpu"
        );

        // One router, two planes: the namespace plane still answers on the same
        // ticket beside the exec plane.
        let mount = crate::mesh::dial_iroh_remote(&url, "").unwrap();
        assert!(
            mount
                .remote
                .metadata(&NormalizedPath::new("hello.txt").unwrap())
                .is_ok(),
            "the FS plane must keep serving beside #cpu"
        );

        drop(mount);
        drop(server);
        std::fs::remove_dir_all(&cwd).ok();
    }

    /// `--cpu --peer HEX` scopes exec to that one verified identity: a dialer
    /// with any other key is closed without running code (default-deny exec).
    #[test]
    fn mesh_serve_cpu_with_peer_refuses_other_identities() {
        use wanix_fs::MemFs;

        use crate::mesh::IROH_SCHEME;

        let granted = NodeIdentity::from_secret_bytes([62u8; 32]);
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/unused",
            "--addr",
            "127.0.0.1:0",
            "--cpu",
            "--peer",
            &granted.peer_id().to_hex(),
            "--grant",
            "wanix:.:ro",
        ]))
        .unwrap();
        let identity = NodeIdentity::from_secret_bytes([63u8; 32]);
        let root: Arc<dyn FileSystem> = Arc::new(MemFs::new());
        let server = build_and_serve(&command, &identity, &root).unwrap();

        let cwd = std::env::temp_dir().join(format!("wanix-cpu-deny-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("build.js"), "std.out.puts('must not run');").unwrap();

        let addrs: Vec<String> = server
            .ticket()
            .ip_addrs()
            .map(|addr| format!("addr={addr}"))
            .collect();
        let url = format!("{IROH_SCHEME}{}?{}", server.peer_id(), addrs.join("&"));
        let cpu_command = crate::cpu::parse_cpu_command(&args(&[
            "--node",
            &url,
            "--cwd",
            cwd.to_str().unwrap(),
            "--",
            "qjs",
            "build.js",
        ]))
        .unwrap();
        // The CLI dial verb binds an ephemeral identity, which is not the
        // granted peer, so the acceptor must close without running the job.
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = crate::cpu::run_cpu_streaming(cpu_command, &mut out, &mut err);
        assert!(
            result.is_err(),
            "an unallowlisted identity must not run code, got stdout {:?}",
            String::from_utf8_lossy(&out)
        );

        drop(server);
        std::fs::remove_dir_all(&cwd).ok();
    }

    #[test]
    fn parse_reads_cpu_flag_on_local_endpoint() {
        let command = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--addr",
            "127.0.0.1:0",
            "--cpu",
        ]))
        .unwrap();
        assert!(command.cpu);
    }

    #[test]
    fn parse_defaults_cpu_off() {
        let command =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--addr", "127.0.0.1:0"]))
                .unwrap();
        assert!(!command.cpu);
    }

    #[test]
    fn parse_refuses_cpu_on_public_endpoint() {
        // SECURITY: --cpu binds the exec plane — an admitted peer runs arbitrary
        // task code on this host. Like --wanix-services it is refused on the
        // public endpoint entirely, with the hazard named.
        let error = parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--cpu"])).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("remote code execution") && message.contains("--addr"),
            "public --cpu must be refused naming the exec hazard, got {message:?}"
        );
    }

    #[test]
    fn parse_refuses_cpu_with_insecure_open() {
        // --insecure-open opts into open FILE export; it must not be a backdoor
        // to exec export.
        let error =
            parse_mesh_serve_command(&args(&["--root", "/tmp/x", "--cpu", "--insecure-open"]))
                .unwrap_err();
        assert!(
            error.to_string().contains("remote code execution"),
            "--cpu --insecure-open must be refused naming the exec hazard"
        );
    }

    #[test]
    fn parse_refuses_cpu_on_non_loopback_addr() {
        // SECURITY: with no --peer the cpu acceptor admits any dialer, and a
        // non-loopback --addr is mDNS-discoverable on the LAN — so --cpu there
        // is unauthenticated LAN RCE. Refused even with --peer/--grant, like
        // the public endpoint.
        for extra in [&[][..], &["--peer", "abababab"][..]] {
            let mut argv = vec!["--root", "/tmp/x", "--addr", "192.168.1.9:7000", "--cpu"];
            argv.extend_from_slice(extra);
            if !extra.is_empty() {
                argv.extend_from_slice(&["--grant", "docs:docs:ro"]);
            }
            let error = parse_mesh_serve_command(&args(&argv)).unwrap_err();
            assert!(
                error.to_string().contains("remote code execution"),
                "non-loopback --cpu must be refused naming the exec hazard, got {error}"
            );
        }
    }

    #[test]
    fn parse_refuses_cpu_with_public_grant() {
        // Even a grant-gated public serve keeps exec off the public endpoint,
        // mirroring the --wanix-services rule.
        let error = parse_mesh_serve_command(&args(&[
            "--root",
            "/tmp/x",
            "--peer",
            &"ab".repeat(32),
            "--grant",
            "docs:docs:ro",
            "--cpu",
        ]))
        .unwrap_err();
        assert!(
            error.to_string().contains("remote code execution"),
            "grant-gated public --cpu must still be refused"
        );
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
