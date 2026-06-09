//! `wanix volume serve`: one process, many volume resources.
//!
//! Per ADR 0007 the v0 volume server is "one process, many resource endpoints":
//! each served volume gets its own native mesh endpoint with a distinct, stable
//! identity and prints one ticket. There is **no aggregate `/vol/*` root** — the
//! client composes the per-volume tickets it was handed with repeated
//! `--mount-mesh`. Per-resource selectors / `aname` / catalog / auth are out of
//! scope here (later slices). The binding loop and announce-line format are the
//! shared [`crate::mesh::resource`] machinery (also behind `tool serve`).

use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use wanix_fs::{FileSystem, LocalFs};
use wanix_id::NodeIdentity;
use wanix_mesh::NativeServeConfig;

use super::{
    defined_volume_names, load_volume_identity, resolve_existing_volume, validate_volume_name,
    volumes_root,
};
use crate::mesh::resource::{
    ServedEndpoint, bind_endpoints, reject_fixed_port_multi, serve_record_example_line,
    serve_record_line,
};
use crate::{CliError, write_process_output};

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
        reject_fixed_port_multi("volume serve", "volume", local_addr, list.len())?;
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
    reject_fixed_port_multi("volume serve", "volume", command.local_addr, names.len())?;

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
            serve_record_line(&volume.name, &volume.ticket_url).as_bytes(),
        )?;
        write_process_output(
            process_stderr,
            "stderr",
            serve_record_example_line(&volume.ticket_url).as_bytes(),
        )?;
    }
    // Serving runs on each node's owned runtime; park so they stay alive until the
    // process is terminated.
    loop {
        std::thread::park();
    }
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

/// Binds one native mesh endpoint per `(name, root_dir, identity)`, each serving
/// exactly that volume's `LocalFs` root over [`NativeServeConfig::open`] — one
/// resource per endpoint, never an aggregate root. The caller must hold the
/// returned [`ServedEndpoint`]s (their nodes) for as long as the endpoints
/// serve.
pub(crate) fn bind_volume_endpoints(
    volumes: Vec<(String, PathBuf, NodeIdentity)>,
    local_addr: Option<SocketAddr>,
) -> Result<Vec<ServedEndpoint>, CliError> {
    let mut resources = Vec::with_capacity(volumes.len());
    for (name, root_dir, identity) in volumes {
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
        resources.push((name, identity, NativeServeConfig::open(root)));
    }
    bind_endpoints("volume", resources, local_addr)
}

#[cfg(test)]
mod tests;
