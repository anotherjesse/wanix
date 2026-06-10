//! `wanix recipe`: saved mount+run compositions (ADR 0007 recipes).
//!
//! A recipe is one TOML file at `~/.wanix/recipes/<name>.recipe` describing a
//! reproducible working set: a list of binds (catalog NAME or `iroh://`
//! address, each with a guest mount path) and an optional `run` command line.
//! `recipe save` authors one explicitly — there is no magic capture; the binds
//! are intentional decisions — and `recipe run` resolves the binds through the
//! catalog at launch time, composes the mounts into one namespace, and runs
//! the command line through the `sh` path (or an interactive `sh` session when
//! the recipe has no run line).
//!
//! When a bind target is a catalog NAME, the address it resolved to at save
//! time is recorded as a `hint`. At run time the catalog is authoritative —
//! names resolve fresh, per ADR 0007 §2 — and the hint is a drift check: a
//! catalog that now disagrees with the hint warns loudly, and a name that
//! vanished from the catalog falls back to dialing the saved hint (with a
//! warning) instead of stranding the recipe.

mod command;
mod exec;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CliError;
use crate::qjs_args::MeshMountSpec;

pub(crate) use command::{parse_recipe_command, run_recipe_command};
#[cfg(unix)]
pub(crate) use exec::interactive_recipe_session;

/// One recipe: the on-disk shape is this struct as TOML at
/// `<recipes dir>/<name>.recipe` (absent `description`/`run` are omitted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Recipe {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    /// The command line `recipe run` hands to `sh -c`; absent means the recipe
    /// opens an interactive shell over its mounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run: Option<String>,
    #[serde(default)]
    pub(crate) binds: Vec<RecipeBind>,
}

/// One recipe bind: a target (catalog NAME or `iroh://` address) mounted at a
/// guest path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecipeBind {
    /// A catalog NAME (resolved at run time) or an `iroh://` address (dialed
    /// directly).
    pub(crate) target: String,
    /// Guest mount path (stored normalized, no leading slash).
    pub(crate) path: String,
    /// The address a NAME target resolved to at save time — a drift check and
    /// an offline-catalog fallback, never preferred over a live catalog entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hint: Option<String>,
}

/// The default recipes directory, `~/.wanix/recipes` (beside the catalog).
///
/// # Errors
///
/// Returns a CLI error when no home directory is known.
pub(crate) fn default_recipes_dir() -> Result<PathBuf, CliError> {
    Ok(crate::volume::wanix_dir()?.join("recipes"))
}

/// The recipe file for `name`: `<dir>/<name>.recipe`.
fn recipe_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.recipe"))
}

/// Validates a recipe name: same grammar as catalog names (lowercase
/// `[a-z0-9-]`), so a recipe name is never spelled like a ticket or a path.
///
/// # Errors
///
/// Returns a usage error naming the grammar when the name does not fit it.
pub(crate) fn validate_recipe_name(name: &str) -> Result<(), CliError> {
    if !crate::catalog::is_name_spelling(name) {
        return Err(CliError::usage(format!(
            "invalid recipe name {name:?}: use lowercase letters, digits, and - (no dots, \
             slashes, spaces, or uppercase; must start and end with a letter or digit)"
        )));
    }
    Ok(())
}

/// Writes `recipe` into the recipes dir, refusing to overwrite an existing
/// file unless `force`. Returns the written path.
///
/// # Errors
///
/// Returns a usage error for an invalid name or refused overwrite, and a CLI
/// error when the file cannot be written.
pub(crate) fn write_recipe(dir: &Path, recipe: &Recipe, force: bool) -> Result<PathBuf, CliError> {
    validate_recipe_name(&recipe.name)?;
    let path = recipe_path(dir, &recipe.name);
    if !force && path.exists() {
        return Err(CliError::usage(format!(
            "recipe {:?} already exists at {}; pass --force to overwrite",
            recipe.name,
            path.display()
        )));
    }
    std::fs::create_dir_all(dir).map_err(|error| {
        CliError::new(
            format!("failed to create recipes dir {}: {error}", dir.display()),
            1,
        )
    })?;
    let toml = toml::to_string(recipe)
        .map_err(|error| CliError::new(format!("failed to encode recipe: {error}"), 1))?;
    std::fs::write(&path, toml).map_err(|error| {
        CliError::new(
            format!("failed to write recipe {}: {error}", path.display()),
            1,
        )
    })?;
    Ok(path)
}

/// Reads the recipe named `name` from `dir`.
///
/// # Errors
///
/// Returns a CLI error when the recipe does not exist (pointing at `recipe
/// save`) or cannot be read or parsed.
pub(crate) fn read_recipe(dir: &Path, name: &str) -> Result<Recipe, CliError> {
    validate_recipe_name(name)?;
    let path = recipe_path(dir, name);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CliError::new(
                format!(
                    "no recipe {name:?} (recipes {}); save one with `wanix-rust recipe save \
                     {name} --mount NAME[=PATH] ... [--run LINE]`",
                    dir.display()
                ),
                1,
            )
        } else {
            CliError::new(
                format!("failed to read recipe {}: {error}", path.display()),
                1,
            )
        }
    })?;
    toml::from_str(&text).map_err(|error| {
        CliError::new(
            format!("failed to parse recipe {}: {error}", path.display()),
            1,
        )
    })
}

/// Resolves a recipe's binds through the catalog at `catalog_dir` into
/// dialable [`MeshMountSpec`]s, returning the audit/warning lines the runner
/// must surface (resolution log lines, drift warnings, hint fallbacks).
///
/// Per ADR 0007 §2 the catalog is authoritative at launch: a NAME resolves
/// fresh, a saved hint only flags drift or substitutes for a vanished entry.
///
/// # Errors
///
/// Returns a CLI error when a NAME target has neither a catalog entry nor a
/// saved hint, or a target is neither a name nor an `iroh://` address.
pub(crate) fn resolve_binds_in(
    catalog_dir: &Path,
    recipe: &Recipe,
) -> Result<(Vec<MeshMountSpec>, Vec<String>), CliError> {
    let mut mounts = Vec::with_capacity(recipe.binds.len());
    let mut notes = Vec::new();
    for bind in &recipe.binds {
        let addr = resolve_bind_target(catalog_dir, &recipe.name, bind, &mut notes)?;
        mounts.push(MeshMountSpec {
            addr,
            guest_path: crate::qjs_args::mesh_guest_path(&bind.path, "recipe bind")?,
        });
    }
    Ok((mounts, notes))
}

fn resolve_bind_target(
    catalog_dir: &Path,
    recipe_name: &str,
    bind: &RecipeBind,
    notes: &mut Vec<String>,
) -> Result<String, CliError> {
    if !crate::catalog::is_name_spelling(&bind.target) {
        if bind.target.starts_with(crate::mesh::IROH_SCHEME) {
            return Ok(bind.target.clone());
        }
        return Err(CliError::new(
            format!(
                "recipe {recipe_name:?}: bind target {:?} is neither a catalog name nor an \
                 iroh:// address",
                bind.target
            ),
            1,
        ));
    }
    match crate::catalog::try_resolve_name_in(catalog_dir, &bind.target)? {
        Some(address) => {
            notes.push(crate::catalog::resolution_log_line(&bind.target, &address));
            if let Some(hint) = &bind.hint
                && hint != &address
            {
                notes.push(format!(
                    "wanix-rust: recipe {recipe_name:?} bind '{}' DRIFTED since save: saved hint \
                     {hint}, catalog now {address} — using the catalog address",
                    bind.target
                ));
            }
            Ok(address)
        }
        None => match &bind.hint {
            Some(hint) => {
                notes.push(format!(
                    "wanix-rust: recipe {recipe_name:?} bind '{}' is no longer in the catalog; \
                     dialing the saved hint {hint}",
                    bind.target
                ));
                Ok(hint.clone())
            }
            None => Err(CliError::new(
                format!(
                    "recipe {recipe_name:?}: bind '{}' has no catalog entry (catalog {}) and no \
                     saved hint; add one with `wanix-rust catalog add {} IROH_URL`",
                    bind.target,
                    catalog_dir.display(),
                    bind.target
                ),
                1,
            )),
        },
    }
}
