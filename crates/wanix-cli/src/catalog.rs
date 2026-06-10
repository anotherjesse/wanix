//! `wanix catalog`: the local address book (ADR 0007 §Suggested build order
//! step 5 — the naming layer only, Layer 1; no ACLs).
//!
//! One JSON file per entry under `~/.wanix/catalog/<name>.json` binds a humane
//! NAME to a dialable address. v0 addresses are live `iroh://` tickets (the
//! peer id IS the resource identity; `?addr=` is only a route hint); `cas:`
//! blobs and `local:` paths are reserved future address kinds. Names use a
//! grammar that can never be confused with a ticket or a path — lowercase
//! `[a-z0-9-]`, no dots, no slashes, no scheme — so every resolution point can
//! tell "a name" from "an address" by spelling alone, forever.
//!
//! Resolution is a launch-time operation: [`resolve_name`] turns a name into
//! the bound address once, when a mount is created, so running tasks hold
//! resolved addresses and a catalog rebind affects the next launch, never a
//! live namespace. Authorization (who may dial what) is the ADR 0007 Layer 2/3
//! work; the catalog only answers "what does this name dial".

mod command;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CliError;

pub(crate) use command::{parse_catalog_command, run_catalog_command};

/// One catalog entry: a humane name bound to a dialable address, with optional
/// human-facing description and query tags.
///
/// The on-disk shape is this struct as JSON, one file per entry at
/// `<catalog dir>/<name>.json`; absent `description`/empty `tags` are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CatalogEntry {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
    pub(crate) address: String,
}

/// The default catalog directory, `~/.wanix/catalog` (beside the volumes root
/// and the identity keys).
///
/// # Errors
///
/// Returns a CLI error when no home directory is known.
pub(crate) fn default_catalog_dir() -> Result<PathBuf, CliError> {
    Ok(crate::volume::wanix_dir()?.join("catalog"))
}

/// Validates a catalog name: non-empty lowercase `[a-z0-9-]` with alphanumeric
/// first and last characters.
///
/// Deliberately stricter than volume names (no `.`, no `_`, no uppercase): a
/// catalog name must stay distinguishable from a ticket (`iroh://...`,
/// `cas:...`) and from a path (anything with `/` or `.`) at every present and
/// future resolution point, by spelling alone.
///
/// # Errors
///
/// Returns a usage error naming the grammar when the name does not fit it.
pub(crate) fn validate_catalog_name(name: &str) -> Result<(), CliError> {
    let bytes = name.as_bytes();
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-';
    if !ok {
        return Err(CliError::usage(format!(
            "invalid catalog name {name:?}: use lowercase letters, digits, and - (no dots, \
             slashes, spaces, or uppercase; must start and end with a letter or digit) so a \
             name can never be mistaken for a ticket or a path"
        )));
    }
    Ok(())
}

/// Validates a catalog address: a parseable `iroh://` ticket in v0.
///
/// `cas:` and `local:` are reserved address kinds (ADR 0007 §1) and named as
/// future work; anything else gets the ticket parser's own diagnosis.
///
/// # Errors
///
/// Returns a usage error for reserved-but-unsupported schemes or an unparseable
/// ticket.
pub(crate) fn validate_catalog_address(address: &str) -> Result<(), CliError> {
    for reserved in ["cas:", "local:"] {
        if address.starts_with(reserved) {
            return Err(CliError::usage(format!(
                "catalog addresses are live iroh:// tickets in v0; {reserved} entries are a \
                 future address kind (got {address:?})"
            )));
        }
    }
    crate::mesh::MeshTicket::parse(address).map(|_| ())
}

/// The entry file for `name`: `<dir>/<name>.json`.
fn entry_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// Writes `entry` into the catalog at `dir`, validating name and address.
/// Refuses to overwrite an existing entry unless `force`. Returns the written
/// path.
///
/// # Errors
///
/// Returns a usage error for an invalid name/address or a refused overwrite,
/// and a CLI error when the file cannot be written.
pub(crate) fn write_entry(
    dir: &Path,
    entry: &CatalogEntry,
    force: bool,
) -> Result<PathBuf, CliError> {
    validate_catalog_name(&entry.name)?;
    validate_catalog_address(&entry.address)?;
    let path = entry_path(dir, &entry.name);
    if !force && path.exists() {
        let existing = read_entry(dir, &entry.name)?;
        return Err(CliError::usage(format!(
            "catalog entry {:?} already exists (address {}); pass --force to overwrite",
            entry.name, existing.address
        )));
    }
    std::fs::create_dir_all(dir).map_err(|error| {
        CliError::new(
            format!("failed to create catalog dir {}: {error}", dir.display()),
            1,
        )
    })?;
    let mut json = serde_json::to_vec_pretty(entry)
        .map_err(|error| CliError::new(format!("failed to encode catalog entry: {error}"), 1))?;
    json.push(b'\n');
    std::fs::write(&path, json).map_err(|error| {
        CliError::new(
            format!("failed to write catalog entry {}: {error}", path.display()),
            1,
        )
    })?;
    Ok(path)
}

/// Reads the entry named `name` from the catalog at `dir`.
///
/// # Errors
///
/// Returns a CLI error when the entry does not exist (pointing at `catalog
/// add`) or its file cannot be read or parsed.
pub(crate) fn read_entry(dir: &Path, name: &str) -> Result<CatalogEntry, CliError> {
    validate_catalog_name(name)?;
    let path = entry_path(dir, name);
    let bytes = std::fs::read(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CliError::new(
                format!(
                    "no catalog entry {name:?}; add one with `wanix-rust catalog add {name} \
                     IROH_URL`"
                ),
                1,
            )
        } else {
            CliError::new(
                format!("failed to read catalog entry {}: {error}", path.display()),
                1,
            )
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::new(
            format!("failed to parse catalog entry {}: {error}", path.display()),
            1,
        )
    })
}

/// Removes the entry named `name` from the catalog at `dir`.
///
/// # Errors
///
/// Returns a CLI error when the entry does not exist or cannot be removed.
pub(crate) fn remove_entry(dir: &Path, name: &str) -> Result<(), CliError> {
    validate_catalog_name(name)?;
    let path = entry_path(dir, name);
    std::fs::remove_file(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CliError::new(format!("no catalog entry {name:?} (nothing to remove)"), 1)
        } else {
            CliError::new(
                format!("failed to remove catalog entry {}: {error}", path.display()),
                1,
            )
        }
    })
}

/// Lists every entry in the catalog at `dir`, sorted by name. A missing
/// catalog directory is an empty catalog, not an error.
///
/// # Errors
///
/// Returns a CLI error when the directory or an entry file cannot be read or
/// parsed.
pub(crate) fn list_entries(dir: &Path) -> Result<Vec<CatalogEntry>, CliError> {
    let mut entries = Vec::new();
    let dir_entries = match std::fs::read_dir(dir) {
        Ok(dir_entries) => dir_entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
        Err(error) => {
            return Err(CliError::new(
                format!("failed to read catalog dir {}: {error}", dir.display()),
                1,
            ));
        }
    };
    for dir_entry in dir_entries {
        let dir_entry = dir_entry
            .map_err(|error| CliError::new(format!("failed to read catalog dir: {error}"), 1))?;
        let path = dir_entry.path();
        let Some(name) = path
            .file_name()
            .and_then(|file| file.to_str())
            .and_then(|file| file.strip_suffix(".json"))
        else {
            continue;
        };
        entries.push(read_entry(dir, name)?);
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Resolves a catalog NAME to the address it is bound to — the single Layer-1
/// naming seam the Names phase consumes (ADR 0007 §2: names resolve at launch
/// time; running tasks hold resolved addresses).
///
/// Nothing routes through this yet by design: the next phase points
/// `--mount-mesh NAME=GUEST` and the `mount-*` verbs here when an argument has
/// no scheme and no slash.
///
/// # Errors
///
/// Returns a usage error when `name` does not fit the name grammar (a ticket
/// or path is never a name) and a CLI error when no entry exists.
#[allow(dead_code)] // the Names-phase seam; consumed by tests only so far
pub(crate) fn resolve_name(name: &str) -> Result<String, CliError> {
    resolve_name_in(&default_catalog_dir()?, name)
}

/// [`resolve_name`] against an explicit catalog directory.
///
/// # Errors
///
/// See [`resolve_name`].
pub(crate) fn resolve_name_in(dir: &Path, name: &str) -> Result<String, CliError> {
    Ok(read_entry(dir, name)?.address)
}

/// Writes/updates catalog entries for served endpoints at announce time
/// (`volume serve`/`tool serve`/`app serve --register NAME`).
///
/// One endpoint registers as `NAME`; several register as `NAME-<resource>`
/// (NAME is a prefix), so one serve invocation never silently collapses two
/// tickets into one name. Registration always overwrites: a re-announced
/// resource has a fresh route hint and the entry must follow it. Returns one
/// `# registered ...` line per entry — `# `-prefixed so announce-record
/// parsers skip it, like [`crate::mesh::resource::serve_record_example_line`].
///
/// # Errors
///
/// Returns a CLI error when a derived name does not fit the catalog grammar or
/// an entry cannot be written.
pub(crate) fn register_served(
    dir: &Path,
    register: &str,
    kind: &str,
    endpoints: &[(String, String)],
) -> Result<Vec<String>, CliError> {
    let mut lines = Vec::with_capacity(endpoints.len());
    for (resource, ticket_url) in endpoints {
        let name = if endpoints.len() == 1 {
            register.to_owned()
        } else {
            format!("{register}-{resource}")
        };
        let entry = CatalogEntry {
            name: name.clone(),
            description: Some(format!("{kind} {resource} (registered at serve time)")),
            tags: vec![kind.to_owned()],
            address: ticket_url.clone(),
        };
        let path = write_entry(dir, &entry, true)?;
        lines.push(format!(
            "# registered catalog entry {name} -> {ticket_url} ({})\n",
            path.display()
        ));
    }
    Ok(lines)
}
