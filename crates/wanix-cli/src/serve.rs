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
mod roots;
mod terminal_ws;

#[cfg(test)]
pub(super) use command::DEFAULT_SERVE_ADDR;
pub(super) use command::{ServeCommand, parse_serve_command};
use concurrent::serve_concurrent_connections;
use connection::serve_one_connection;
use discovery::display_host;
use roots::ServeRoots;

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
