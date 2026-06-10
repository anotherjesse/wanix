//! `recipe save` / `recipe run` execution against explicit directories (the
//! command dispatcher supplies the default `~/.wanix` ones; tests inject temp
//! dirs).

use std::io::Read;
use std::path::Path;

use wanix_fs::NormalizedPath;

use super::{Recipe, RecipeBind, read_recipe, resolve_binds_in, write_recipe};
use crate::{CliError, CliOutput};

/// `recipe save`: parses each `--mount TARGET[=PATH]`, resolves NAME targets
/// through the catalog NOW (recording the resolved address as the bind's
/// drift-check hint), and writes the recipe file.
///
/// # Errors
///
/// Returns a CLI error when a mount spec is malformed, a NAME target has no
/// catalog entry, or the recipe cannot be written.
pub(crate) fn run_save_in(
    recipes_dir: &Path,
    catalog_dir: &Path,
    name: String,
    description: Option<String>,
    run: Option<String>,
    mounts: &[String],
    force: bool,
) -> Result<CliOutput, CliError> {
    let mut binds = Vec::with_capacity(mounts.len());
    for mount in mounts {
        binds.push(save_bind(catalog_dir, mount)?);
    }
    let recipe = Recipe {
        name,
        description,
        run,
        binds,
    };
    let path = write_recipe(recipes_dir, &recipe, force)?;
    let mut text = format!("saved recipe {} -> {}\n", recipe.name, path.display());
    for bind in &recipe.binds {
        text.push_str(&format!(
            "bind {} -> /{}{}\n",
            bind.target,
            bind.path,
            bind.hint
                .as_deref()
                .map(|hint| format!(" (hint {hint})"))
                .unwrap_or_default()
        ));
    }
    Ok(CliOutput::new(text.into_bytes(), Vec::new(), 0))
}

/// One `--mount TARGET[=PATH]` into a [`RecipeBind`]: the target/path boundary
/// is the LAST `=` (an iroh URL's own `?addr=` equals stay in the target), and
/// a bare catalog NAME defaults to `n/NAME`.
fn save_bind(catalog_dir: &Path, mount: &str) -> Result<RecipeBind, CliError> {
    let (target, path) = match mount.rsplit_once('=') {
        Some((target, path)) if !target.is_empty() && !path.is_empty() => (target, Some(path)),
        Some(_) => {
            return Err(CliError::usage(
                "recipe save --mount expects TARGET=PATH or a bare catalog NAME",
            ));
        }
        None => (mount, None),
    };
    let is_name = crate::catalog::is_name_spelling(target);
    let path = match path {
        Some(path) => crate::qjs_args::mesh_guest_path(path, "recipe save --mount")?,
        None if is_name => NormalizedPath::new(format!("n/{target}"))?,
        None => {
            return Err(CliError::usage(format!(
                "recipe save --mount: target {target:?} is not a catalog name, so it needs an \
                 explicit =PATH"
            )));
        }
    };
    let hint = if is_name {
        // Resolved at save time: the hint pins what the author meant today.
        Some(crate::catalog::resolve_name_in(catalog_dir, target)?)
    } else {
        crate::mesh::MeshTicket::parse(target)?;
        None
    };
    Ok(RecipeBind {
        target: target.to_owned(),
        path: path.as_str().to_owned(),
        hint,
    })
}

/// `recipe run` with a run line: resolves the binds, composes the mounts, and
/// runs the line (plus any `-- extra` words) through the captured `sh` path.
/// The resolution/drift notes are prepended to the returned stderr.
///
/// # Errors
///
/// Returns a usage error when the recipe has no run line (that is the
/// interactive session, which needs a live terminal) and a CLI error when
/// resolution, dialing, or the shell run fails.
pub(crate) fn run_run_in(
    recipes_dir: &Path,
    catalog_dir: &Path,
    name: &str,
    extra: &[String],
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let recipe = read_recipe(recipes_dir, name)?;
    let Some(line) = recipe.run.clone() else {
        return Err(CliError::usage(format!(
            "recipe {name:?} has no run line, so `recipe run {name}` opens an interactive shell \
             over its mounts; run the wanix binary on a tty or save it with --run LINE"
        )));
    };
    let (mesh_mounts, notes) = resolve_binds_in(catalog_dir, &recipe)?;
    let command = crate::sh::ShCommand {
        line: Some(append_extra(&line, extra)),
        env: Vec::new(),
        cwd: NormalizedPath::new(".")?,
        mesh_mounts,
    };
    let output = crate::sh::run_sh(command, process_stdin)?;
    let mut stderr = notes.join("\n").into_bytes();
    if !stderr.is_empty() {
        stderr.push(b'\n');
    }
    stderr.extend_from_slice(output.stderr());
    Ok(CliOutput::new(
        output.stdout().to_vec(),
        stderr,
        output.exit_code(),
    ))
}

/// Appends `-- extra` words to the run line, single-quoting anything the shell
/// would split or expand.
fn append_extra(line: &str, extra: &[String]) -> String {
    let mut out = line.to_owned();
    for word in extra {
        out.push(' ');
        let plain = !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c));
        if plain {
            out.push_str(word);
        } else {
            out.push('\'');
            out.push_str(&word.replace('\'', "'\\''"));
            out.push('\'');
        }
    }
    out
}

/// The interactive route (`recipe run NAME` where the recipe has no run line):
/// resolves the binds against the default catalog, prints the
/// resolution/drift notes to stderr, and hands back an interactive
/// [`crate::sh::ShCommand`] over the recipe's mounts for the fd-aware terminal
/// path. Returns `Ok(None)` when the invocation is not an interactive recipe
/// run (the collected path then handles it).
///
/// # Errors
///
/// Returns a CLI error when the recipe is missing, extra words are given
/// without a run line, or resolution fails.
#[cfg(unix)]
pub(crate) fn interactive_recipe_session(
    rest: &[std::ffi::OsString],
) -> Result<Option<crate::sh::ShCommand>, CliError> {
    use super::command::{RecipeCommand, parse_recipe_command};

    let RecipeCommand::Run { name, extra } = parse_recipe_command(rest)? else {
        return Ok(None);
    };
    let recipe = read_recipe(&super::default_recipes_dir()?, &name)?;
    if recipe.run.is_some() {
        return Ok(None);
    }
    if !extra.is_empty() {
        return Err(CliError::usage(format!(
            "recipe {name:?} has no run line to append `--` words to; save it with --run LINE"
        )));
    }
    let catalog = crate::catalog::default_catalog_dir()?;
    let (mesh_mounts, notes) = resolve_binds_in(&catalog, &recipe)?;
    for note in notes {
        eprintln!("{note}");
    }
    Ok(Some(crate::sh::ShCommand {
        line: None,
        env: Vec::new(),
        cwd: NormalizedPath::new(".")?,
        mesh_mounts,
    }))
}
