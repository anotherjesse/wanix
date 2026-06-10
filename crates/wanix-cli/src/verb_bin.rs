//! Host-side BinVerbs serving (ADR 0007 §Confinement contract).
//!
//! A served resource ships its vocabulary as plain files in a `bin/`
//! directory beside its tree: `app serve` exposes `<app-dir>/bin/*` when the
//! directory exists, and `tool serve --config` exposes a per-tool `bin` dir.
//! The surface is host-served (never routed through a guest), read-only,
//! flat, and size-capped ([`wanix_fs::VerbBinFs`]); a mounted client invokes
//! the verbs confined through the shell's `NAME:CMD` form, so the program and
//! the authority it gets arrive together.

use std::path::Path;
use std::sync::Arc;

use wanix_fs::{FileSystem, FsResult, LocalFs, VerbBinFs};
use wanix_vfs::{BindOptions, Namespace};

use crate::CliError;

/// Loads a resource's `bin/` directory as its read-only verb surface.
///
/// Returns `Ok(None)` when the directory does not exist — most resources ship
/// no verbs, and the served tree is then exactly what it was before.
///
/// # Errors
///
/// Returns a CLI error when the directory exists but cannot be opened.
pub(crate) fn load_verb_bin(
    bin_dir: &Path,
    context: &str,
) -> Result<Option<Arc<dyn FileSystem>>, CliError> {
    if !bin_dir.is_dir() {
        return Ok(None);
    }
    let local = LocalFs::new(bin_dir).map_err(|error| {
        CliError::new(
            format!("{context}: cannot open {}: {error}", bin_dir.display()),
            1,
        )
    })?;
    Ok(Some(Arc::new(VerbBinFs::new(Arc::new(local)))))
}

/// Composes one connection's resource view with the shipped verb `bin/` (when
/// there is one): the resource stays at the root and `bin` appears beside it,
/// both visible in the root listing.
///
/// # Errors
///
/// Returns a filesystem error when either binding is refused.
pub(crate) fn compose_with_verb_bin(
    view: Arc<dyn FileSystem>,
    bin: Option<&Arc<dyn FileSystem>>,
) -> FsResult<Arc<dyn FileSystem>> {
    let Some(bin) = bin else {
        return Ok(view);
    };
    let mut namespace = Namespace::new();
    namespace.bind(view, ".", ".", BindOptions::default())?;
    namespace.bind(Arc::clone(bin), ".", "bin", BindOptions::default())?;
    Ok(Arc::new(namespace))
}
