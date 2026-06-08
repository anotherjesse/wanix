//! `wanix volume`: persistent local data volumes under `~/.wanix/volumes/<name>`.
//!
//! A volume is just a host directory you can serve over the native mesh wire
//! (`mesh-serve --volume NAME`) and mount into a shell namespace
//! (`qjs-shell --mount-mesh ...`). This module owns volume naming, the on-disk
//! layout, and the `create`/`ls` verbs. Per ADR 0007 the v0 volume server is
//! "one process, many resource endpoints" — one ticket per volume — so this
//! stays deliberately CLI-local: no remote administration, no aggregate root.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use wanix_id::NodeIdentity;

use crate::{CliError, CliOutput};

mod serve;

#[cfg(test)]
mod tests;

pub(crate) use serve::{parse_volume_serve_command, run_volume_serve_streaming};

/// One parsed `volume` subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumeCommand {
    Create { name: String },
    Ls,
}

/// Parses `volume (create NAME | ls)`.
///
/// # Errors
///
/// Returns a usage error when the subcommand is missing/unknown, `create` lacks
/// a NAME or carries extra arguments, or the name is invalid.
pub(crate) fn parse_volume_command(args: &[OsString]) -> Result<VolumeCommand, CliError> {
    let mut iter = args.iter();
    let verb = iter
        .next()
        .ok_or_else(|| CliError::usage("volume: expected a subcommand (create NAME | ls)"))?
        .to_str()
        .ok_or_else(|| CliError::usage("volume: arguments must be valid UTF-8"))?;
    match verb {
        "create" => {
            let name = iter
                .next()
                .ok_or_else(|| CliError::usage("volume create: expected a NAME"))?
                .to_str()
                .ok_or_else(|| CliError::usage("volume create: NAME must be valid UTF-8"))?
                .to_owned();
            if iter.next().is_some() {
                return Err(CliError::usage("volume create: expected exactly one NAME"));
            }
            validate_volume_name(&name)?;
            Ok(VolumeCommand::Create { name })
        }
        "ls" => {
            if iter.next().is_some() {
                return Err(CliError::usage("volume ls: takes no arguments"));
            }
            Ok(VolumeCommand::Ls)
        }
        other => Err(CliError::usage(format!(
            "volume: unknown subcommand {other:?} (expected create NAME | ls)"
        ))),
    }
}

/// Runs a parsed `volume` subcommand against the default `~/.wanix/volumes` root.
///
/// # Errors
///
/// Returns a CLI error when the volumes root cannot be resolved, the volume
/// already exists (for `create`), or the directory cannot be created/listed.
pub(crate) fn run_volume_command(command: VolumeCommand) -> Result<CliOutput, CliError> {
    let root = volumes_root()?;
    match command {
        VolumeCommand::Create { name } => create_volume_in(&root, &name),
        VolumeCommand::Ls => list_volumes_in(&root),
    }
}

/// Creates `root/<name>`, erroring if it already exists (create is explicit, not
/// idempotent — re-running surfaces a stale-state mistake; pinned by tests).
fn create_volume_in(root: &Path, name: &str) -> Result<CliOutput, CliError> {
    validate_volume_name(name)?;
    let dir = root.join(name);
    if dir.exists() {
        return Err(CliError::new(
            format!("volume {name:?} already exists at {}", dir.display()),
            1,
        ));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|error| CliError::new(format!("failed to create volume {name:?}: {error}"), 1))?;
    let line = format!("created volume {name} at {}\n", dir.display());
    Ok(CliOutput::new(line.into_bytes(), Vec::new(), 0))
}

/// Lists the volume names defined under `root` (one per line, sorted).
fn list_volumes_in(root: &Path) -> Result<CliOutput, CliError> {
    let names = defined_volume_names(root)?;
    let output = if names.is_empty() {
        String::new()
    } else {
        format!("{}\n", names.join("\n"))
    };
    Ok(CliOutput::new(output.into_bytes(), Vec::new(), 0))
}

/// The sorted volume names defined under `root`. A missing volumes root yields an
/// empty list rather than an error. Shared by `volume ls` and `volume serve
/// --all`.
fn defined_volume_names(root: &Path) -> Result<Vec<String>, CliError> {
    let mut names = Vec::new();
    match std::fs::read_dir(root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|error| {
                    CliError::new(format!("failed to read volumes dir: {error}"), 1)
                })?;
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && let Some(name) = entry.file_name().to_str()
                {
                    names.push(name.to_owned());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::new(
                format!("failed to read volumes dir {}: {error}", root.display()),
                1,
            ));
        }
    }
    names.sort();
    Ok(names)
}

/// Resolves an existing volume directory under `root`, validating the name.
///
/// Used by `mesh-serve --volume NAME` to turn a name into a served root.
///
/// # Errors
///
/// Returns a usage error for an invalid name, or a CLI error when the volume
/// does not exist (pointing at `wanix volume create`).
pub(crate) fn resolve_existing_volume(root: &Path, name: &str) -> Result<PathBuf, CliError> {
    validate_volume_name(name)?;
    let dir = root.join(name);
    if !dir.is_dir() {
        return Err(CliError::new(
            format!(
                "volume {name:?} does not exist at {}; create it with `wanix volume create {name}`",
                dir.display()
            ),
            1,
        ));
    }
    Ok(dir)
}

/// The default volumes root, `~/.wanix/volumes` (beside the node identity key).
///
/// # Errors
///
/// Returns a CLI error when no home directory is known.
pub(crate) fn volumes_root() -> Result<PathBuf, CliError> {
    Ok(wanix_dir()?.join("volumes"))
}

/// The `~/.wanix` base directory (parent of the node identity key).
fn wanix_dir() -> Result<PathBuf, CliError> {
    let key = NodeIdentity::default_key_path()
        .map_err(|error| CliError::new(format!("cannot resolve ~/.wanix: {error}"), 1))?;
    key.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| CliError::new("cannot resolve the ~/.wanix directory", 1))
}

/// The per-volume mesh-endpoint identity key path,
/// `~/.wanix/volume-identities/<name>.key`.
///
/// Deliberately OUTSIDE the served volume root (`~/.wanix/volumes/<name>`) so a
/// volume's own endpoint secret key is never exported to peers that mount it.
fn volume_identity_path(name: &str) -> Result<PathBuf, CliError> {
    validate_volume_name(name)?;
    Ok(wanix_dir()?
        .join("volume-identities")
        .join(format!("{name}.key")))
}

/// Loads (or creates) the stable per-volume endpoint identity for `name`. Each
/// volume gets a distinct key, hence a distinct peer id, so its mesh endpoint is
/// an independent resource.
fn load_volume_identity(name: &str) -> Result<NodeIdentity, CliError> {
    load_identity_at(&volume_identity_path(name)?)
}

/// Loads or creates an owner-private identity at `path`, creating the parent
/// directory first (`load_or_create` does not).
fn load_identity_at(path: &Path) -> Result<NodeIdentity, CliError> {
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
            format!("failed to load volume identity {}: {error}", path.display()),
            1,
        )
    })
}

/// Validates a volume name: non-empty, ASCII alphanumeric plus `.` `_` `-`, with
/// alphanumeric first and last characters. This rejects `/`, `\`, `..`, `.`,
/// leading/trailing punctuation, and anything that could escape the volume root.
fn validate_volume_name(name: &str) -> Result<(), CliError> {
    let bytes = name.as_bytes();
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric();
    if !ok {
        return Err(CliError::usage(format!(
            "invalid volume name {name:?}: use ASCII letters/digits, optionally with . _ - in \
             the middle (no /, \\, .., or leading/trailing punctuation)"
        )));
    }
    Ok(())
}
