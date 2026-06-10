//! `wanix-rust mount` subcommands: dial a TCP 9P server, build a [`RemoteFs`],
//! bind it into a fresh namespace at `/n/remote`, and perform one filesystem
//! operation through the namespace.
//!
//! These are deliberately small, collected (non-streaming) demos: each verb
//! opens one connection, runs one op, prints the result, and returns a
//! [`CliOutput`]. They exist so a blog reader can copy-paste a `serve` and a
//! `mount` command across two processes and watch Plan 9 import work over a real
//! socket.

use std::ffi::OsString;
use std::net::TcpStream;

use wanix_9p_client::RemoteFs;
use wanix_fs::NormalizedPath;
use wanix_vfs::{BindOptions, Namespace};

use crate::{CliError, CliOutput};

mod ops;

#[cfg(test)]
mod tests;

/// Namespace destination (a relative Wanix path) the remote is bound at.
pub(crate) const MOUNT_POINT: &str = "n/remote";

/// Human-facing label for the mount point used in messages.
const MOUNT_LABEL: &str = "/n/remote";

/// One parsed `mount` subcommand: a verb plus its already-validated arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MountCommand {
    /// List a directory at `path` (default the mount root).
    Ls { addr: String, path: String },
    /// Print the bytes of the file at `path`. With `follow`, stream
    /// incrementally on one open handle with no byte cap, exiting only at
    /// end-of-file or on error (Ctrl-C is plain process exit) — the consumer
    /// for never-EOF device streams (`#pipe/<id>/data`, `#task/<id>/wait`).
    Cat {
        addr: String,
        path: String,
        follow: bool,
    },
    /// Write `text` to the file at `path`, creating or truncating it.
    Write {
        addr: String,
        path: String,
        text: String,
    },
}

/// Parses a `mount-ls` / `mount-cat` / `mount-write` invocation.
///
/// The leading `mount-*` command name selects the verb; `rest` carries the
/// `tcp://HOST:PORT` address followed by the verb operands.
///
/// # Errors
///
/// Returns a usage error when the verb is unknown or operands are missing.
pub(super) fn parse_mount_command(verb: &str, rest: &[OsString]) -> Result<MountCommand, CliError> {
    match verb {
        "mount-ls" => parse_ls(rest),
        "mount-cat" => parse_cat(rest),
        "mount-write" => parse_write(rest),
        other => Err(CliError::usage(format!("unknown mount command: {other}"))),
    }
}

fn parse_ls(rest: &[OsString]) -> Result<MountCommand, CliError> {
    let addr = mount_addr(rest.first(), "mount-ls")?;
    let path = optional_path_arg(rest.get(1), "mount-ls")?.unwrap_or_else(|| ".".to_owned());
    Ok(MountCommand::Ls { addr, path })
}

fn parse_cat(rest: &[OsString]) -> Result<MountCommand, CliError> {
    // `--follow` may appear anywhere among the operands.
    let operands: Vec<&OsString> = rest
        .iter()
        .filter(|arg| arg.to_str() != Some("--follow"))
        .collect();
    let follow = operands.len() != rest.len();
    let addr = mount_addr(operands.first().copied(), "mount-cat")?;
    let path = required_path_arg(operands.get(1).copied(), "mount-cat")?;
    Ok(MountCommand::Cat { addr, path, follow })
}

fn parse_write(rest: &[OsString]) -> Result<MountCommand, CliError> {
    let addr = mount_addr(rest.first(), "mount-write")?;
    let path = required_path_arg(rest.get(1), "mount-write")?;
    let text = rest
        .get(2)
        .ok_or_else(|| {
            CliError::usage("mount-write requires (tcp://HOST:PORT | iroh://PEER | NAME) PATH TEXT")
        })?
        .to_str()
        .ok_or_else(|| CliError::usage("mount-write TEXT must be valid UTF-8"))?
        .to_owned();
    Ok(MountCommand::Write { addr, path, text })
}

fn mount_addr(arg: Option<&OsString>, verb: &str) -> Result<String, CliError> {
    arg.ok_or_else(|| {
        CliError::usage(format!(
            "{verb} requires (tcp://HOST:PORT | iroh://PEER | NAME)"
        ))
    })?
    .to_str()
    .map(ToOwned::to_owned)
    .ok_or_else(|| CliError::usage(format!("{verb} address must be valid UTF-8")))
}

fn required_path_arg(arg: Option<&OsString>, verb: &str) -> Result<String, CliError> {
    optional_path_arg(arg, verb)?.ok_or_else(|| {
        CliError::usage(format!(
            "{verb} requires (tcp://HOST:PORT | iroh://PEER | NAME) PATH"
        ))
    })
}

fn optional_path_arg(arg: Option<&OsString>, verb: &str) -> Result<Option<String>, CliError> {
    let Some(arg) = arg else {
        return Ok(None);
    };
    let value = arg
        .to_str()
        .ok_or_else(|| CliError::usage(format!("{verb} PATH must be valid UTF-8")))?;
    Ok(Some(value.to_owned()))
}

/// Runs a parsed `mount` subcommand and returns its captured output.
///
/// # Errors
///
/// Returns a CLI error when the server cannot be dialed, the session cannot be
/// negotiated, or the requested filesystem operation fails.
pub(super) fn run_mount_command(command: MountCommand) -> Result<CliOutput, CliError> {
    match command {
        MountCommand::Ls { addr, path } => {
            // `session` holds the mount's keepalive (the dialer node for an
            // `iroh://` mount owns the runtime the op runs on) for the whole op.
            let session = mount_namespace(&addr)?;
            ops::mount_ls(&session.namespace, &mount_path(&path)?)
        }
        MountCommand::Cat { follow: true, .. } => Err(CliError::usage(
            "mount-cat --follow requires live process IO; use the wanix-rust binary",
        )),
        MountCommand::Cat { addr, path, .. } => {
            let session = mount_namespace(&addr)?;
            ops::mount_cat(&session.namespace, &mount_path(&path)?)
        }
        MountCommand::Write { addr, path, text } => {
            let session = mount_namespace(&addr)?;
            ops::mount_write(&session.namespace, &mount_path(&path)?, text.as_bytes())
        }
    }
}

/// Runs `mount-cat --follow` as a streaming command: chunks flow to the live
/// process stdout as they arrive on one open handle, with no byte cap, until
/// end-of-file or an error. Returns `Ok(None)` when the invocation is not a
/// follow (the collected `mount-cat` path then handles it).
///
/// # Errors
///
/// Returns a CLI error when parsing, dialing, binding, or streaming fails.
pub(super) fn run_mount_cat_follow_streaming(
    rest: &[std::ffi::OsString],
    stdout: &mut dyn std::io::Write,
) -> Result<Option<i32>, CliError> {
    let MountCommand::Cat {
        addr,
        path,
        follow: true,
    } = parse_mount_command("mount-cat", rest)?
    else {
        return Ok(None);
    };
    // The session (and its iroh keepalive, if any) must outlive the stream.
    let session = mount_namespace(&addr)?;
    ops::mount_cat_follow(&session.namespace, &mount_path(&path)?, stdout)?;
    Ok(Some(0))
}

/// A live mount: a namespace with the remote bound at `/n/remote`, plus the
/// transport keepalive that must outlive every operation on it.
///
/// For an `iroh://` mount the keepalive is the dialer [`crate::mesh::IrohMount`],
/// which owns the tokio runtime the native-wire import drives its QUIC traffic
/// on; dropping it before the op runs would shut that runtime down and panic.
/// For a `tcp://` mount there is nothing extra to hold (the `TcpStream` lives
/// inside the `RemoteFs`), so the keepalive is `None`.
struct MountSession {
    namespace: Namespace,
    /// Held only to keep the transport (and its runtime, for iroh) alive for the
    /// mount's lifetime.
    _keepalive: Option<crate::mesh::IrohMount>,
}

/// Dials `addr` and binds the imported remote at `/n/remote`, choosing the
/// transport (and wire) by spelling.
///
/// A bare catalog NAME (no scheme, no slash) resolves through `~/.wanix/catalog`
/// at invocation time — launch-time naming per ADR 0007 §2, so the operation
/// runs against the resolved address, never the name. `iroh://<peer>[?addr=...]`
/// dials the peer over the QUIC mesh and imports it over the **native**
/// `wanix-mesh-wire` plane (typed `FsError`s, one bidi stream per op / per open
/// file); `tcp://HOST:PORT` opens a raw TCP **9P** stream at the foreign edge.
/// Both imports are `FileSystem`s, so the `mount-*` verbs run identically over
/// either, and the bind is type-transparent.
///
/// The `iroh://` arm returns its dialer node (inside [`crate::mesh::IrohMount`])
/// as a keepalive: it owns the runtime the native import runs every op on, so
/// the caller must hold it until the operation completes.
fn mount_namespace(addr: &str) -> Result<MountSession, CliError> {
    dial_mount(&crate::catalog::resolve_mount_target(addr)?)
}

/// [`mount_namespace`] resolving names through an explicit catalog directory,
/// so tests can prove the mount-by-name path without the user's catalog.
#[cfg(test)]
fn mount_namespace_in(catalog: &std::path::Path, addr: &str) -> Result<MountSession, CliError> {
    dial_mount(&crate::catalog::resolve_mount_target_in(catalog, addr)?)
}

/// The transport-dispatch half of [`mount_namespace`]: `addr` is already a
/// resolved address, never a name.
fn dial_mount(addr: &str) -> Result<MountSession, CliError> {
    if addr.starts_with(crate::mesh::IROH_SCHEME) {
        // Wanix↔Wanix mesh import: the native wire over QUIC.
        let mount = crate::mesh::dial_iroh_remote(addr, "")?;
        let remote = std::sync::Arc::clone(&mount.remote);
        return bind_mount(remote, Some(mount));
    }
    // Foreign-edge import: raw TCP 9P, unchanged per ADR 0004.
    let remote = dial_tcp_remote(addr)?;
    bind_mount(remote, None)
}

/// Binds an imported `FileSystem` at `/n/remote` and pairs it with `keepalive`.
fn bind_mount(
    remote: std::sync::Arc<dyn wanix_fs::FileSystem>,
    keepalive: Option<crate::mesh::IrohMount>,
) -> Result<MountSession, CliError> {
    let mut namespace = Namespace::new();
    namespace
        .bind(remote, ".", MOUNT_POINT, BindOptions::default())
        .map_err(|error| {
            CliError::new(
                format!("failed to bind remote at {MOUNT_LABEL}: {error}"),
                1,
            )
        })?;
    Ok(MountSession {
        namespace,
        _keepalive: keepalive,
    })
}

/// Connects a TCP stream to the `tcp://HOST:PORT` address and negotiates 9P.
fn dial_tcp_remote(addr: &str) -> Result<std::sync::Arc<RemoteFs>, CliError> {
    let host_port = addr.strip_prefix("tcp://").ok_or_else(|| {
        CliError::usage(format!(
            "mount address must be tcp://HOST:PORT, iroh://<peer>, or a catalog NAME: {addr}"
        ))
    })?;
    let stream = TcpStream::connect(host_port).map_err(|error| {
        CliError::new(format!("failed to dial 9P server {host_port}: {error}"), 1)
    })?;
    let remote = RemoteFs::connect(Box::new(stream))
        .map_err(|error| CliError::new(format!("failed to negotiate 9P session: {error}"), 1))?;
    Ok(std::sync::Arc::new(remote))
}

/// Joins a user-supplied relative path under the `/n/remote` mount point.
fn mount_path(path: &str) -> Result<NormalizedPath, CliError> {
    let trimmed = path.trim_start_matches('/');
    let joined = if trimmed.is_empty() || trimmed == "." {
        MOUNT_POINT.to_owned()
    } else {
        format!("{MOUNT_POINT}/{trimmed}")
    };
    NormalizedPath::new(&joined)
        .map_err(|error| CliError::new(format!("invalid mount path {path}: {error}"), 1))
}

/// Runs a namespace operation against an already-built namespace.
///
/// Exposed so tests can bind a local `RemoteFs` and reuse the exact verb code
/// the CLI runs.
#[cfg(test)]
pub(crate) fn run_mount_op_for_tests(
    namespace: &Namespace,
    command: &MountCommand,
) -> Result<CliOutput, CliError> {
    match command {
        MountCommand::Ls { path, .. } => ops::mount_ls(namespace, &mount_path(path)?),
        MountCommand::Cat { path, .. } => ops::mount_cat(namespace, &mount_path(path)?),
        MountCommand::Write { path, text, .. } => {
            ops::mount_write(namespace, &mount_path(path)?, text.as_bytes())
        }
    }
}

/// Runs the streaming follow loop against an already-built namespace, so tests
/// exercise the exact loop the CLI runs over a real loopback mount.
#[cfg(test)]
pub(crate) fn run_mount_cat_follow_for_tests(
    namespace: &Namespace,
    path: &str,
    stdout: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    ops::mount_cat_follow(namespace, &mount_path(path)?, stdout)
}
