//! `wanix app serve`: one guest-defined AppResource over the native mesh.
//!
//! Follows the `tool serve`/`volume serve` resource shape (ADR 0007: one
//! ticket names one resource): a persisted per-app endpoint identity, one
//! parse-stable `NAME\tTICKET` record plus a copy-pasteable mount comment,
//! then park forever. The trust-boundary heart is [`AppAttachPolicy`]: every
//! connection is served `AppFsService::open_view(<verified remote_id hex>)`,
//! so the principal stamped into every guest event — message attribution,
//! `who` presence — is derived from the QUIC handshake identity and never
//! from anything the client sent.

use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use wanix_appfs::AppFsService;
use wanix_id::{AttachPolicy, Authorization, NodeIdentity, PeerId};
use wanix_mesh::NativeServeConfig;
use wanix_vfs::Rights;

use super::guest::start_app_guest;
use super::{app_identity_path, load_app_manifest};
use crate::mesh::resource::{
    ServedEndpoint, bind_endpoints, load_identity_at, serve_record_example_line, serve_record_line,
};
use crate::{CliError, write_process_output};

/// A parsed `app serve` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppServeCommand {
    app_dir: PathBuf,
    state_dir: PathBuf,
    name: Option<String>,
    local_addr: Option<SocketAddr>,
    insecure_open: bool,
}

/// Parses `app serve --app DIR --state DIR [--name NAME] [--addr IP:PORT]
/// [--insecure-open]`.
///
/// # Errors
///
/// Returns a usage error when `--app` or `--state` is missing, an option
/// lacks its value, or the public endpoint is not explicitly opted into.
pub(crate) fn parse_app_serve_command(args: &[OsString]) -> Result<AppServeCommand, CliError> {
    let mut app_dir = None;
    let mut state_dir = None;
    let mut name = None;
    let mut local_addr = None;
    let mut insecure_open = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| CliError::usage("app serve: arguments must be valid UTF-8"))?;
        match flag {
            "--insecure-open" => {
                insecure_open = true;
                index += 1;
            }
            "--app" => {
                app_dir = Some(PathBuf::from(value(args, index, "--app")?));
                index += 2;
            }
            "--state" => {
                state_dir = Some(PathBuf::from(value(args, index, "--state")?));
                index += 2;
            }
            "--name" => {
                name = Some(value(args, index, "--name")?);
                index += 2;
            }
            "--addr" => {
                let raw = value(args, index, "--addr")?;
                local_addr = Some(raw.parse::<SocketAddr>().map_err(|error| {
                    CliError::usage(format!("app serve --addr must be IP:PORT: {error}"))
                })?);
                index += 2;
            }
            other => {
                return Err(CliError::usage(format!(
                    "app serve: unexpected argument {other}"
                )));
            }
        }
    }
    let app_dir = app_dir.ok_or_else(|| CliError::usage("app serve: --app DIR is required"))?;
    let state_dir = state_dir.ok_or_else(|| {
        CliError::usage("app serve: --state DIR is required (the app's durable state mount)")
    })?;
    // Default-deny on the public endpoint (matches tool serve / volume serve):
    // serving with no --addr hands the app to anyone with its ticket. Open
    // mode has no allow-list, not no identity: every connection is still
    // bound to its verified peer id, so attribution stays unforgeable.
    if local_addr.is_none() && !insecure_open {
        return Err(CliError::usage(
            "app serve on the public endpoint exposes the app to anyone with its ticket; pass \
             --addr IP:PORT or --insecure-open to deliberately export to the open internet",
        ));
    }
    Ok(AppServeCommand {
        app_dir,
        state_dir,
        name,
        local_addr,
        insecure_open,
    })
}

fn value(args: &[OsString], index: usize, flag: &str) -> Result<String, CliError> {
    args.get(index + 1)
        .ok_or_else(|| CliError::usage(format!("app serve {flag} expects a value")))?
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::usage(format!("app serve {flag} value must be valid UTF-8")))
}

/// Starts the guest, binds the app's mesh endpoint, prints its ticket record,
/// and parks the process so the endpoint and guest stay alive.
///
/// # Errors
///
/// Returns a CLI error when the manifest, state dir, guest task, identity, or
/// endpoint cannot be set up. (The public-endpoint posture is enforced in
/// [`parse_app_serve_command`].)
pub(crate) fn run_app_serve_streaming(
    command: AppServeCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let manifest = load_app_manifest(&command.app_dir)?;
    let name = command
        .name
        .clone()
        .unwrap_or_else(|| manifest.name.clone());
    std::fs::create_dir_all(&command.state_dir).map_err(|error| {
        CliError::new(
            format!(
                "app serve: cannot create --state {}: {error}",
                command.state_dir.display()
            ),
            1,
        )
    })?;
    let (guest, service) = start_app_guest(&command.app_dir, &command.state_dir, &manifest)?;
    let identity = load_identity_at(&app_identity_path(&name)?)?;
    let served = bind_app_endpoint(name, identity, &service, command.local_addr)?;
    for endpoint in &served {
        write_process_output(
            process_stderr,
            "stderr",
            serve_record_line(&endpoint.name, &endpoint.ticket_url).as_bytes(),
        )?;
        write_process_output(
            process_stderr,
            "stderr",
            serve_record_example_line(&endpoint.ticket_url).as_bytes(),
        )?;
    }
    // Park: the endpoint serves and the guest task runs until the process is
    // terminated (the guest then sees stdin EOF and exits; no auto-restart).
    let _guest = guest;
    loop {
        std::thread::park();
    }
}

/// Binds one native mesh endpoint serving the app's [`AppFsService`] through
/// a per-connection principal-scoped view.
pub(crate) fn bind_app_endpoint(
    name: String,
    identity: NodeIdentity,
    service: &AppFsService,
    local_addr: Option<SocketAddr>,
) -> Result<Vec<ServedEndpoint>, CliError> {
    let policy = Arc::new(AppAttachPolicy {
        service: service.clone(),
    });
    let config = NativeServeConfig::per_peer(policy);
    bind_endpoints("app", vec![(name, identity, config)], local_addr)
}

/// The AppFS attach seam: binds each connection's verified peer id to a
/// principal-scoped [`wanix_appfs::AppFs`] view.
///
/// The `peer` argument is the cryptographically verified `remote_id()` of the
/// QUIC connection — never a client-claimed name or payload field. Every
/// ticket holder is admitted (an open room; allow-list rooms are the ADR 0007
/// Layer 2 follow-up), but every guest event this connection causes carries
/// the verified principal, which is what makes chat attribution and `who`
/// presence unforgeable.
struct AppAttachPolicy {
    service: AppFsService,
}

impl AttachPolicy for AppAttachPolicy {
    fn evaluate(&self, peer: PeerId, _aname: &str) -> Option<Authorization> {
        let view = self.service.open_view(peer.to_hex());
        Some(Authorization::new(Arc::new(view), Rights::read_write()))
    }
}

#[cfg(test)]
mod tests;
