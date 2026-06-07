//! `wanix capsule` — freeze a Wanix world (a directory the agent built) onto the
//! content-addressed plane (venti) and restore it deterministically.
//!
//! Reframed onto CAS per the mesh blueprint: each world file is one blob
//! (identical files dedup automatically), a deterministic sorted
//! [`WorldManifest`] is itself a blob, and *that manifest blob's hash is the
//! capsule id* — the share token. `save` prints the id; `load <id>` fetches the
//! manifest blob, verifies every referenced blob, and materializes the world
//! with the path-safety + size/fan-out caps [`wanix_cas`] enforces.
//!
//! Blobs live in a [`LocalCasStore`] (the audited owner-private/atomic-write
//! boundary). The store directory is `$WANIX_CAS_DIR` or the per-user default;
//! `--store DIR` overrides it. The id is the local-store form of the mesh's
//! `BlobTicket`: shipping a capsule peer-to-peer over iroh is the `wanix-mesh`
//! data plane, which loads the same capsule id from an [`IrohCasStore`].

use std::ffi::OsString;
use std::path::PathBuf;

use wanix_cas::{Capsule, ContentStore, LocalCasStore};
use wanix_fs::ContentHash;

use crate::{CliError, CliOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapsuleAction {
    Save,
    Load,
}

/// A parsed `capsule` command.
#[derive(Debug)]
pub(super) struct CapsuleCommand {
    action: CapsuleAction,
    /// World directory: the tree to freeze (`save`) or to materialize into
    /// (`load`).
    dir: PathBuf,
    /// Capsule id (manifest blob hash) for `load`; `None` for `save`.
    id: Option<ContentHash>,
    /// Optional explicit CAS store directory (`--store DIR`).
    store_dir: Option<PathBuf>,
}

/// Parses `capsule save <DIR> [--store DIR]` and
/// `capsule load <CAPSULE_ID> <DIR> [--store DIR]`.
///
/// # Errors
///
/// Returns a usage error when the action, paths, or capsule id are missing or
/// malformed.
pub(super) fn parse_capsule_command(args: &[OsString]) -> Result<CapsuleCommand, CliError> {
    let (positional, store_dir) = split_store_flag(args)?;
    let action = match positional.first().and_then(|a| a.to_str()) {
        Some("save") => CapsuleAction::Save,
        Some("load") => CapsuleAction::Load,
        _ => return Err(CliError::usage("capsule: expected `save` or `load`")),
    };
    match action {
        CapsuleAction::Save => {
            let dir = positional
                .get(1)
                .ok_or_else(|| CliError::usage("capsule save: missing DIR"))?;
            if positional.len() > 2 {
                return Err(CliError::usage("capsule save: too many arguments"));
            }
            Ok(CapsuleCommand {
                action,
                dir: PathBuf::from(dir),
                id: None,
                store_dir,
            })
        }
        CapsuleAction::Load => {
            let id_arg = positional
                .get(1)
                .and_then(|a| a.to_str())
                .ok_or_else(|| CliError::usage("capsule load: missing CAPSULE_ID"))?;
            let id = ContentHash::from_hex(id_arg)
                .map_err(|_| CliError::usage("capsule load: CAPSULE_ID must be a 64-hex hash"))?;
            let dir = positional
                .get(2)
                .ok_or_else(|| CliError::usage("capsule load: missing DIR"))?;
            if positional.len() > 3 {
                return Err(CliError::usage("capsule load: too many arguments"));
            }
            Ok(CapsuleCommand {
                action,
                dir: PathBuf::from(dir),
                id: Some(id),
                store_dir,
            })
        }
    }
}

/// Splits out an optional trailing/leading `--store DIR` flag from the
/// positional arguments.
fn split_store_flag(args: &[OsString]) -> Result<(Vec<OsString>, Option<PathBuf>), CliError> {
    let mut positional = Vec::new();
    let mut store_dir = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--store" {
            let value = iter
                .next()
                .ok_or_else(|| CliError::usage("capsule: --store requires a DIR"))?;
            store_dir = Some(PathBuf::from(value));
        } else {
            positional.push(arg.clone());
        }
    }
    Ok((positional, store_dir))
}

/// Runs the capsule command.
///
/// # Errors
///
/// Returns an error when the directory cannot be read or written, the store
/// cannot freeze/fetch a blob, or a load id is unknown.
pub(super) fn run_capsule_command(command: CapsuleCommand) -> Result<CliOutput, CliError> {
    let store = open_store(command.store_dir.as_deref());
    let stdout = match command.action {
        CapsuleAction::Save => save(&store, &command.dir)?,
        CapsuleAction::Load => load(&store, command.id, &command.dir)?,
    };
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

/// Opens the CAS store, honoring an explicit `--store DIR` over the default.
fn open_store(store_dir: Option<&std::path::Path>) -> LocalCasStore {
    match store_dir {
        Some(dir) => LocalCasStore::open(dir.to_path_buf()),
        None => LocalCasStore::open_default(),
    }
}

fn save(store: &dyn ContentStore, dir: &std::path::Path) -> Result<String, CliError> {
    let capsule = Capsule::freeze(store, dir)
        .map_err(|error| CliError::new(format!("capsule save: {error}"), 1))?;
    let id = capsule.id().to_hex();
    let count = capsule.manifest().len();
    // The id is the share token: `wanix capsule load <id> <DIR>` on any node with
    // access to the same (or a peer-fed) store reconstructs the world.
    Ok(format!(
        "capsule {id} saved ({count} files) from {}\nload with: wanix capsule load {id} <DIR>\n",
        dir.display()
    ))
}

fn load(
    store: &dyn ContentStore,
    id: Option<ContentHash>,
    dir: &std::path::Path,
) -> Result<String, CliError> {
    let id = id.ok_or_else(|| CliError::usage("capsule load: missing CAPSULE_ID"))?;
    let capsule = Capsule::load(store, id)
        .map_err(|error| CliError::new(format!("capsule load: {error}"), 1))?;
    let stats = capsule
        .materialize(store, dir)
        .map_err(|error| CliError::new(format!("capsule load: {error}"), 1))?;
    Ok(format!(
        "capsule {} restored ({} files, {} bytes) to {}\n",
        id.to_hex(),
        stats.files,
        stats.bytes,
        dir.display()
    ))
}

#[cfg(test)]
mod tests;
