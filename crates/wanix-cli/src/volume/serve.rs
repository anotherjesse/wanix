//! `wanix volume serve`: one process, many volume resources.
//!
//! Per ADR 0007 the v0 volume server is "one process, many resource endpoints":
//! each served volume gets its own native mesh endpoint with a distinct, stable
//! identity and prints one ticket. There is **no aggregate `/vol/*` root** — the
//! client composes the per-volume tickets it was handed with repeated
//! `--mount-mesh`. Per-resource selectors / `aname` / catalog / auth are out of
//! scope here (later slices).

use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{FileSystem, LocalFs};
use wanix_id::NodeIdentity;
use wanix_mesh::{MeshNode, NativeServeConfig};

use super::{
    defined_volume_names, load_volume_identity, resolve_existing_volume, validate_volume_name,
    volumes_root,
};
use crate::{CliError, write_process_output};

/// How long to wait for public-network connectivity before printing a ticket.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which volumes a `volume serve` invocation should export.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VolumeSelection {
    /// An explicit, de-duplicated `--volume NAME` list.
    Explicit(Vec<String>),
    /// `--all`: serve every defined volume, each as its own endpoint/ticket.
    All,
}

/// A parsed `volume serve` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeServeCommand {
    selection: VolumeSelection,
    local_addr: Option<SocketAddr>,
    insecure_open: bool,
}

/// Parses `volume serve (--volume NAME ... | --all) [--addr IP:PORT]
/// [--insecure-open]`.
///
/// # Errors
///
/// Returns a usage error when no selection (or both `--all` and `--volume`) is
/// given, a name is invalid or duplicated, an option lacks its value, or an
/// explicit multi-volume serve is pinned to a fixed nonzero port.
pub(crate) fn parse_volume_serve_command(
    args: &[OsString],
) -> Result<VolumeServeCommand, CliError> {
    let mut names = Vec::new();
    let mut all = false;
    let mut local_addr = None;
    let mut insecure_open = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| CliError::usage("volume serve: arguments must be valid UTF-8"))?;
        match flag {
            "--all" => {
                all = true;
                index += 1;
            }
            "--insecure-open" => {
                insecure_open = true;
                index += 1;
            }
            "--volume" => {
                let name = value(args, index, "--volume")?;
                validate_volume_name(&name)?;
                if names.contains(&name) {
                    return Err(CliError::usage(format!(
                        "volume serve: --volume {name} given more than once"
                    )));
                }
                names.push(name);
                index += 2;
            }
            "--addr" => {
                let raw = value(args, index, "--addr")?;
                local_addr = Some(raw.parse::<SocketAddr>().map_err(|error| {
                    CliError::usage(format!("volume serve --addr must be IP:PORT: {error}"))
                })?);
                index += 2;
            }
            other => {
                return Err(CliError::usage(format!(
                    "volume serve: unexpected argument {other}"
                )));
            }
        }
    }
    let selection = match (all, names.is_empty()) {
        (true, false) => {
            return Err(CliError::usage(
                "volume serve: --all cannot be combined with --volume",
            ));
        }
        (true, true) => VolumeSelection::All,
        (false, false) => VolumeSelection::Explicit(names),
        (false, true) => {
            return Err(CliError::usage(
                "volume serve: specify --all or one or more --volume NAME",
            ));
        }
    };
    if let VolumeSelection::Explicit(list) = &selection {
        reject_fixed_port_multi(local_addr, list.len())?;
    }
    // Default-deny on the public endpoint (matches mesh-serve): serving with no
    // --addr exports each volume read-write to anyone with its ticket, so require
    // an explicit --insecure-open. Enforced at parse time so EVERY path — the
    // streaming runtime and the collected/library path — refuses it, never just
    // the runtime.
    if local_addr.is_none() && !insecure_open {
        return Err(CliError::usage(
            "volume serve on the public endpoint exports each volume read-write to anyone with \
             its ticket; pass --addr IP:PORT (use port 0 to serve multiple volumes) or \
             --insecure-open to deliberately export to the open internet",
        ));
    }
    Ok(VolumeServeCommand {
        selection,
        local_addr,
        insecure_open,
    })
}

fn value(args: &[OsString], index: usize, flag: &str) -> Result<String, CliError> {
    args.get(index + 1)
        .ok_or_else(|| CliError::usage(format!("volume serve {flag} expects a value")))?
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::usage(format!("volume serve {flag} value must be valid UTF-8")))
}

/// Multiple volumes cannot share one fixed port; require port 0 (a unique
/// ephemeral port per endpoint) when serving more than one.
fn reject_fixed_port_multi(local_addr: Option<SocketAddr>, count: usize) -> Result<(), CliError> {
    if count > 1 && local_addr.is_some_and(|addr| addr.port() != 0) {
        return Err(CliError::usage(format!(
            "volume serve: serving {count} volumes needs a unique port per endpoint; pass --addr \
             with port 0 (e.g. 127.0.0.1:0) instead of a fixed port"
        )));
    }
    Ok(())
}

/// Binds one native mesh endpoint per selected volume, prints one ticket per
/// volume to stderr, and parks the process so the endpoints stay alive.
///
/// # Errors
///
/// Returns a CLI error when no volume is selected, a fixed port is used for
/// multiple volumes, or a volume/identity/endpoint cannot be resolved or bound.
/// (The public-endpoint posture is enforced in [`parse_volume_serve_command`].)
pub(crate) fn run_volume_serve_streaming(
    command: VolumeServeCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let names = resolve_selection(&command)?;
    // The public-endpoint posture is enforced at parse time. The fixed-port rule
    // is re-checked here because `--all`'s volume count is only known now (parse
    // already covered an explicit `--volume` list).
    reject_fixed_port_multi(command.local_addr, names.len())?;

    let mut volumes = Vec::with_capacity(names.len());
    for name in names {
        let root_dir = resolve_existing_volume(&volumes_root()?, &name)?;
        let identity = load_volume_identity(&name)?;
        volumes.push((name, root_dir, identity));
    }
    let served = bind_volume_endpoints(volumes, command.local_addr)?;
    for volume in &served {
        write_process_output(
            process_stderr,
            "stderr",
            volume_serve_line(&volume.name, &volume.ticket_url).as_bytes(),
        )?;
        write_process_output(
            process_stderr,
            "stderr",
            volume_serve_example_line(&volume.ticket_url).as_bytes(),
        )?;
    }
    // Serving runs on each node's owned runtime; park so they stay alive until the
    // process is terminated.
    loop {
        std::thread::park();
    }
}

/// One announce line for a served volume: `NAME\tTICKET_URL\n`. Tab-separated and
/// newline-terminated so a later catalog-register step can parse it stably.
fn volume_serve_line(name: &str, ticket_url: &str) -> String {
    format!("{name}\t{ticket_url}\n")
}

/// A copy-pasteable client command for a served volume, printed beside the
/// stable tab record. Starts with `# ` so record parsers skip it as a comment.
fn volume_serve_example_line(ticket_url: &str) -> String {
    format!("# mount with: wanix-rust mount-ls '{ticket_url}'\n")
}

fn resolve_selection(command: &VolumeServeCommand) -> Result<Vec<String>, CliError> {
    let names = match &command.selection {
        VolumeSelection::All => defined_volume_names(&volumes_root()?)?,
        VolumeSelection::Explicit(names) => names.clone(),
    };
    if names.is_empty() {
        return Err(CliError::new(
            "volume serve: no volumes to serve (none defined; create one with `wanix volume \
             create NAME`)",
            1,
        ));
    }
    Ok(names)
}

/// A live served volume endpoint: its name, the held [`MeshNode`] (dropping it
/// shuts the endpoint down), and the dialable ticket.
pub(crate) struct ServedVolume {
    pub(crate) name: String,
    /// Held purely as a liveness keepalive: the endpoint serves as long as the
    /// node lives, so it is never read in production (only in tests, via
    /// `peer_id()`), but dropping it shuts the endpoint down.
    #[allow(dead_code)]
    pub(crate) node: MeshNode,
    pub(crate) ticket_url: String,
}

/// Binds one native mesh endpoint per `(name, root_dir, identity)`, each serving
/// exactly that volume's `LocalFs` root over [`NativeServeConfig::open`] — one
/// resource per endpoint, never an aggregate root. The caller must hold the
/// returned [`ServedVolume`]s (their nodes) for as long as the endpoints serve.
pub(crate) fn bind_volume_endpoints(
    volumes: Vec<(String, PathBuf, NodeIdentity)>,
    local_addr: Option<SocketAddr>,
) -> Result<Vec<ServedVolume>, CliError> {
    let public = local_addr.is_none();
    let mut served = Vec::with_capacity(volumes.len());
    for (name, root_dir, identity) in volumes {
        let mut node = match local_addr {
            Some(addr) => MeshNode::bind_local(&identity, addr),
            None => MeshNode::bind(&identity),
        }
        .map_err(|error| {
            CliError::new(
                format!("failed to bind mesh endpoint for volume {name:?}: {error}"),
                1,
            )
        })?;
        let root = LocalFs::new(&root_dir).map_err(|error| {
            CliError::new(
                format!(
                    "failed to open volume {name:?} root {}: {error}",
                    root_dir.display()
                ),
                1,
            )
        })?;
        let root: Arc<dyn FileSystem> = Arc::new(root);
        node.serve_native(NativeServeConfig::open(root));
        if public {
            node.wait_online(ONLINE_TIMEOUT);
        }
        let ticket_url = ticket_url(&node);
        served.push(ServedVolume {
            name,
            node,
            ticket_url,
        });
    }
    Ok(served)
}

/// Builds the dialable `iroh://<peer>?addr=...` ticket for a bound node.
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

#[cfg(test)]
mod tests;
