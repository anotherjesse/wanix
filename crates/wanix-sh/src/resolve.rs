//! Command-name resolution.
//!
//! Turns a bare command name into a program path the executor can launch. The
//! algorithm is intentionally small and pure (it only reads
//! [`NamespaceOps::exists`]) so a future tab-completion module can reuse it to
//! enumerate command candidates.
//!
//! Paths are Wanix-relative (root is `.`, no leading `/`). A name containing `/`
//! is taken literally; otherwise the search directories are tried in order,
//! preferring the `.wasm` form because the wasm task driver only claims programs
//! ending in `.wasm`. If nothing matches, the bare name is returned so the
//! spawn step reports an honest "not found".

use crate::error::ShellResult;
use crate::ns::NamespaceOps;

/// The PATH-like list of directories searched for bare command names.
///
/// Relative to the namespace root; a future cycle can source this from `$PATH`.
const SEARCH_DIRS: &[&str] = &["bin", "usr/bin"];

/// Resolves a command name to a program path.
///
/// # Errors
///
/// Propagates an error from [`NamespaceOps::exists`].
pub fn resolve_command(name: &str, ns: &dyn NamespaceOps) -> ShellResult<String> {
    if name.contains('/') {
        return Ok(name.to_owned());
    }
    for dir in SEARCH_DIRS {
        // Prefer "<dir>/<name>.wasm" so a wasm command is found; fall back to an
        // exact "<dir>/<name>" (e.g. a name that already carries its extension).
        for candidate in [format!("{dir}/{name}.wasm"), format!("{dir}/{name}")] {
            if ns.exists(&candidate)? {
                return Ok(candidate);
            }
        }
    }
    Ok(name.to_owned())
}
