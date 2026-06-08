use std::io::Write;
use std::net::{SocketAddr, TcpListener};

use crate::{CliError, write_process_output};

mod boot;
mod command;
mod concurrent;
mod connection;
mod direct_v86;
mod discovery;
mod html;
mod http;
mod raw9p;
mod roots;
mod terminal_ws;
mod ws_duplex;

#[cfg(test)]
pub(super) use command::DEFAULT_SERVE_ADDR;
pub(super) use command::{ServeCommand, parse_serve_command};
use concurrent::serve_concurrent_connections;
use connection::serve_one_connection;
use discovery::{display_host, is_loopback_addr};
use roots::ServeRoots;
pub(crate) use roots::services_namespace_for_root;

const FS9P_BUNDLE: &str = "fs9p";
const WORKBENCH_FS9P_BUNDLE: &str = "workbench-fs9p";

pub(super) fn run_serve_streaming(
    command: ServeCommand,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    let listener = TcpListener::bind(&command.addr).map_err(|error| {
        CliError::new(
            format!("failed to bind serve address {}: {error}", command.addr),
            1,
        )
    })?;
    run_serve_with_listener(command, listener, process_stderr)
}

fn run_serve_with_listener(
    command: ServeCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
) -> Result<i32, CliError> {
    run_serve_with_listener_inner(command, listener, process_stderr, None)
}

fn run_serve_with_listener_inner(
    command: ServeCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
    concurrent_connection_limit: Option<usize>,
) -> Result<i32, CliError> {
    let (local_addr, roots) = serve_roots_for_listener(&command, &listener)?;
    // The websocket 9P door rides the HTTP listener and is unauthenticated;
    // `--wanix-services` binds the `#task`/`#agent` exec devices (RCE). Refuse
    // it on a non-loopback HTTP door before binding the raw-9P door too.
    enforce_services_trust_boundary(&command, local_addr, None)?;
    let roots = start_raw9p_door_for_serve(&command, &roots, process_stderr)?;
    write_serve_startup_status(
        &roots,
        local_addr,
        command.bundle.as_deref(),
        process_stderr,
    )?;
    serve_listener_connections(
        command,
        listener,
        roots,
        process_stderr,
        concurrent_connection_limit,
    )
}

/// Binds the raw-9P-over-TCP door when `--p9` is set, enforces the
/// `--wanix-services` off-loopback refusal on the bound raw door, and records
/// the bound address on `roots` for discovery/status. Returns `roots` unchanged
/// when `--p9` is absent.
fn start_raw9p_door_for_serve(
    command: &ServeCommand,
    roots: &ServeRoots,
    process_stderr: &mut dyn Write,
) -> Result<ServeRoots, CliError> {
    let Some(p9_addr) = command.p9_addr.as_deref() else {
        return Ok(roots.clone());
    };
    let policy = raw9p::build_serve_policy(command.peer, command.grants.clone(), &roots.p9_root);
    // The raw-9P door is a long-lived service door (many `mount-*` clients), so
    // it always loops on its dedicated thread; `--once` is the HTTP door's
    // single-connection test affordance and does not apply here. The thread is
    // detached and reaped when the process exits.
    let bound = raw9p::start_raw9p_door(p9_addr, std::sync::Arc::clone(&roots.p9_root), policy)?;
    enforce_services_trust_boundary(command, roots.local_addr, Some(bound))?;
    write_process_output(
        process_stderr,
        "stderr",
        raw9p::raw9p_startup_message(bound).as_bytes(),
    )?;
    Ok(roots.clone().with_p9_tcp_addr(Some(bound)))
}

/// Refuses `--wanix-services` when either 9P door (the HTTP/websocket door or
/// the raw-`--p9` door) is bound to a non-loopback address. `--wanix-services`
/// binds the `#task`/`#agent` exec devices = remote code execution; mirroring
/// the mesh rule, that surface must never reach a non-loopback peer.
fn enforce_services_trust_boundary(
    command: &ServeCommand,
    http_addr: SocketAddr,
    p9_addr: Option<SocketAddr>,
) -> Result<(), CliError> {
    if !command.wanix_services {
        return Ok(());
    }
    let non_loopback_door = if !is_loopback_addr(http_addr) {
        Some("the HTTP/websocket 9P door")
    } else if p9_addr.is_some_and(|addr| !is_loopback_addr(addr)) {
        Some("the raw --p9 9P door")
    } else {
        None
    };
    if let Some(door) = non_loopback_door {
        return Err(CliError::usage(format!(
            "serve --wanix-services binds the #task/#agent exec devices (remote code execution) \
             into the served namespace; it is refused because {door} is bound to a non-loopback \
             address. Bind the door(s) to loopback (e.g. 127.0.0.1:PORT) or drop --wanix-services"
        )));
    }
    Ok(())
}

fn serve_roots_for_listener(
    command: &ServeCommand,
    listener: &TcpListener,
) -> Result<(SocketAddr, ServeRoots), CliError> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| CliError::new(format!("failed to inspect serve address: {error}"), 1))?;
    let roots = ServeRoots::new(
        &command.root_path,
        local_addr,
        command.bundle.clone(),
        command.wanix_services,
    )?;
    Ok((local_addr, roots))
}

fn write_serve_startup_status(
    roots: &ServeRoots,
    local_addr: SocketAddr,
    bundle: Option<&str>,
    process_stderr: &mut dyn Write,
) -> Result<(), CliError> {
    write_process_output(
        process_stderr,
        "stderr",
        format!(
            "wanix-rust serve: serving {} files with Wanix overlay\n",
            roots.static_root.display()
        )
        .as_bytes(),
    )?;
    write_process_output(
        process_stderr,
        "stderr",
        serve_url_status(local_addr, bundle).as_bytes(),
    )
}

fn serve_listener_connections(
    command: ServeCommand,
    listener: TcpListener,
    roots: ServeRoots,
    process_stderr: &mut dyn Write,
    concurrent_connection_limit: Option<usize>,
) -> Result<i32, CliError> {
    if command.once {
        return serve_one_connection(&listener, &roots, process_stderr);
    }

    serve_concurrent_connections(listener, roots, process_stderr, concurrent_connection_limit)
}

#[cfg(test)]
fn run_serve_with_listener_for_connections(
    command: ServeCommand,
    listener: TcpListener,
    process_stderr: &mut dyn Write,
    connection_limit: usize,
) -> Result<i32, CliError> {
    run_serve_with_listener_inner(command, listener, process_stderr, Some(connection_limit))
}

fn serve_url_status(local_addr: SocketAddr, bundle: Option<&str>) -> String {
    let host = display_host(local_addr);
    match bundle {
        Some(bundle) => {
            format!("wanix-rust serve: bundle available at http://{host}/?bundle={bundle}\n")
        }
        None => format!("wanix-rust serve: listening on http://{host}/\n"),
    }
}

#[cfg(test)]
mod tests;
