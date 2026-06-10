//! `wanix tool serve`: ToolFS devices over the native mesh wire.
//!
//! Per ADR 0007 (and `docs/toolfs.md` §"Resource Model") one ticket names one
//! resource: each served tool gets its own native mesh endpoint with a
//! distinct, stable identity and prints one `NAME\tTICKET` record — never an
//! aggregate root of sibling tools. The trust-boundary heart is
//! [`ToolAttachPolicy`]: every connection is served
//! `ToolService::open_view(ToolPrincipal::node(<verified remote_id>))`, so two
//! peers get disjoint `jobs/` views, derived from the QUIC handshake identity
//! and never from anything the client sent.

use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use wanix_id::{AttachPolicy, Authorization, NodeIdentity, PeerId};
use wanix_mesh::NativeServeConfig;
use wanix_tool::{ToolPrincipal, ToolService};
use wanix_vfs::Rights;

use super::config::{ToolConfigFile, load_tool_config};
use super::{BUILTIN_TOOL_NAMES, build_named_tool_service, load_tool_identity};
use crate::mesh::resource::{
    ServedEndpoint, bind_endpoints, reject_fixed_port_multi, serve_record_example_line,
    serve_record_line,
};
use crate::{CliError, write_process_output};

/// A parsed `tool serve` invocation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolServeCommand {
    tools: Vec<String>,
    config: Option<ToolConfigFile>,
    local_addr: Option<SocketAddr>,
    insecure_open: bool,
}

/// Parses `tool serve [--config TOOLS.toml] [--tool NAME ...]
/// [--listen IP:PORT] [--insecure-open]` (`--addr` stays a parsing synonym
/// for `--listen`, ADR 0006).
///
/// `--config` defines host-program tools (shadowing same-named built-ins);
/// `--tool` selects from the merged registry, defaulting to every configured
/// tool when a config is given.
///
/// # Errors
///
/// Returns a usage error when no tool is selected, a name is neither
/// configured nor built-in or is duplicated, the config is invalid, an option
/// lacks its value, a multi-tool serve is pinned to a fixed nonzero port, or
/// the public endpoint is not explicitly opted into.
pub(crate) fn parse_tool_serve_command(args: &[OsString]) -> Result<ToolServeCommand, CliError> {
    let mut tools: Vec<String> = Vec::new();
    let mut config: Option<ToolConfigFile> = None;
    let mut local_addr = None;
    let mut insecure_open = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| CliError::usage("tool serve: arguments must be valid UTF-8"))?;
        match flag {
            "--insecure-open" => {
                insecure_open = true;
                index += 1;
            }
            "--config" => {
                if config.is_some() {
                    return Err(CliError::usage("tool serve: --config given more than once"));
                }
                config = Some(load_tool_config(value(args, index, "--config")?.as_ref())?);
                index += 2;
            }
            "--tool" => {
                let name = value(args, index, "--tool")?;
                if tools.contains(&name) {
                    return Err(CliError::usage(format!(
                        "tool serve: --tool {name} given more than once"
                    )));
                }
                tools.push(name);
                index += 2;
            }
            "--listen" | "--addr" => {
                let raw = value(args, index, flag)?;
                local_addr = Some(raw.parse::<SocketAddr>().map_err(|error| {
                    CliError::usage(format!("tool serve --listen must be IP:PORT: {error}"))
                })?);
                index += 2;
            }
            other => {
                return Err(CliError::usage(format!(
                    "tool serve: unexpected argument {other}"
                )));
            }
        }
    }
    // --tool names resolve against the merged registry (config first, then
    // built-ins), wherever --config appeared on the command line.
    for name in &tools {
        let known = config.as_ref().is_some_and(|file| file.get(name).is_some())
            || BUILTIN_TOOL_NAMES.contains(&name.as_str());
        if !known {
            let mut available: Vec<String> = config
                .as_ref()
                .map(ToolConfigFile::names)
                .unwrap_or_default();
            let unshadowed: Vec<String> = BUILTIN_TOOL_NAMES
                .iter()
                .filter(|builtin| !available.iter().any(|name| name == *builtin))
                .map(ToString::to_string)
                .collect();
            available.extend(unshadowed);
            return Err(CliError::usage(format!(
                "tool serve: unknown tool {name:?} (available: {})",
                available.join(", ")
            )));
        }
    }
    if tools.is_empty() {
        match &config {
            // A config with no selection serves every configured tool.
            Some(file) => tools = file.names(),
            None => {
                return Err(CliError::usage(
                    "tool serve: specify one or more --tool NAME (or --config TOOLS.toml)",
                ));
            }
        }
    }
    reject_fixed_port_multi("tool serve", "tool", local_addr, tools.len())?;
    // Default-deny on the public endpoint (matches volume serve / mesh-serve):
    // serving with no --listen hands a runnable tool to anyone with its
    // ticket, so require an explicit --insecure-open. Note what the flag does
    // NOT skip: the verified peer identity still binds every connection to its
    // own private job view — open mode has no allow-list, not no identity.
    if local_addr.is_none() && !insecure_open {
        return Err(CliError::usage(
            "tool serve on the public endpoint exposes each tool to anyone with its ticket; \
             pass --listen IP:PORT (use port 0 to serve multiple tools) or --insecure-open to \
             deliberately export to the open internet",
        ));
    }
    Ok(ToolServeCommand {
        tools,
        config,
        local_addr,
        insecure_open,
    })
}

fn value(args: &[OsString], index: usize, flag: &str) -> Result<String, CliError> {
    args.get(index + 1)
        .ok_or_else(|| CliError::usage(format!("tool serve {flag} expects a value")))?
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::usage(format!("tool serve {flag} value must be valid UTF-8")))
}

/// Binds one native mesh endpoint per selected tool, prints one ticket per tool
/// to stderr, and parks the process so the endpoints stay alive.
///
/// # Errors
///
/// Returns a CLI error when a tool identity or endpoint cannot be resolved or
/// bound. (The public-endpoint posture and the built-in tool names are
/// enforced in [`parse_tool_serve_command`].)
pub(crate) fn run_tool_serve_streaming(
    command: ToolServeCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let mut tools = Vec::with_capacity(command.tools.len());
    for name in &command.tools {
        tools.push((name.clone(), load_tool_identity(name)?));
    }
    let served = bind_tool_endpoints(tools, command.config.as_ref(), command.local_addr)?;
    for tool in &served {
        write_process_output(
            process_stderr,
            "stderr",
            serve_record_line(&tool.name, &tool.ticket_url).as_bytes(),
        )?;
        write_process_output(
            process_stderr,
            "stderr",
            serve_record_example_line(&tool.ticket_url).as_bytes(),
        )?;
    }
    // Serving runs on each node's owned runtime; park so they stay alive until
    // the process is terminated.
    loop {
        std::thread::park();
    }
}

/// Binds one native mesh endpoint per `(name, identity)`, each serving exactly
/// that tool's [`ToolService`] through a per-connection principal-scoped view —
/// one resource per endpoint, never an aggregate root. The caller must hold the
/// returned [`ServedEndpoint`]s (their nodes) for as long as the endpoints
/// serve.
pub(crate) fn bind_tool_endpoints(
    tools: Vec<(String, NodeIdentity)>,
    config: Option<&ToolConfigFile>,
    local_addr: Option<SocketAddr>,
) -> Result<Vec<ServedEndpoint>, CliError> {
    let mut resources = Vec::with_capacity(tools.len());
    for (name, identity) in tools {
        let service = build_named_tool_service(&name, config)?;
        let policy = Arc::new(ToolAttachPolicy { service });
        resources.push((name, identity, NativeServeConfig::per_peer(policy)));
    }
    bind_endpoints("tool", resources, local_addr)
}

/// The ToolFS attach seam: binds each connection's verified peer id to a
/// principal-scoped [`wanix_tool::ToolFs`] view.
///
/// The `peer` argument is the cryptographically verified `remote_id()` of the
/// QUIC connection (`wanix-mesh` reads it from the handshake before serving a
/// single frame) — never a client-claimed name or payload field. Every ticket
/// holder is admitted (there is no allow-list yet; `--insecure-open` governs
/// only the public-endpoint posture), but each peer sees only its own `jobs/`:
/// ToolFS job privacy crossing the mesh.
struct ToolAttachPolicy {
    service: ToolService,
}

impl AttachPolicy for ToolAttachPolicy {
    fn evaluate(&self, peer: PeerId, _aname: &str) -> Option<Authorization> {
        let view = self.service.open_view(ToolPrincipal::node(peer.to_hex()));
        Some(Authorization::new(Arc::new(view), Rights::read_write()))
    }
}

#[cfg(test)]
mod proc_tests;
#[cfg(test)]
mod tests;
