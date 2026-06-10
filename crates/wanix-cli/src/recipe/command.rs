//! `wanix recipe (save | run)` parsing and the default-directory dispatcher
//! (execution against explicit directories lives in [`super::exec`]).

use std::ffi::OsString;
use std::io::Read;

use super::default_recipes_dir;
use super::exec::{run_run_in, run_save_in};
use crate::{CliError, CliOutput};

/// One parsed `recipe` subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecipeCommand {
    Save {
        name: String,
        description: Option<String>,
        run: Option<String>,
        /// Raw `--mount TARGET[=PATH]` values, resolved against the catalog at
        /// save time (hints are recorded then, not at parse time).
        mounts: Vec<String>,
        force: bool,
    },
    Run {
        name: String,
        /// Words after `--`, appended to the recipe's run line.
        extra: Vec<String>,
    },
}

/// Parses `recipe (save NAME [--description TEXT] [--run LINE] [--force]
/// --mount TARGET[=PATH] ... | run NAME [-- ARG ...])`.
///
/// # Errors
///
/// Returns a usage error when the subcommand is missing/unknown or an operand
/// or flag value is missing, duplicated, or invalid.
pub(crate) fn parse_recipe_command(args: &[OsString]) -> Result<RecipeCommand, CliError> {
    let mut words = Vec::with_capacity(args.len());
    for arg in args {
        words.push(
            arg.to_str()
                .ok_or_else(|| CliError::usage("recipe: arguments must be valid UTF-8"))?,
        );
    }
    let Some((&verb, rest)) = words.split_first() else {
        return Err(CliError::usage(
            "recipe: expected a subcommand (save NAME --mount TARGET[=PATH] ... [--run LINE] | \
             run NAME [-- ARG ...])",
        ));
    };
    match verb {
        "save" => parse_save(rest),
        "run" => parse_run(rest),
        other => Err(CliError::usage(format!(
            "recipe: unknown subcommand {other:?} (expected save | run)"
        ))),
    }
}

fn parse_save(rest: &[&str]) -> Result<RecipeCommand, CliError> {
    let mut positional = Vec::new();
    let mut description = None;
    let mut run = None;
    let mut mounts = Vec::new();
    let mut force = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index] {
            "--force" => {
                force = true;
                index += 1;
            }
            "--description" => {
                set_once(&mut description, flag_value(rest, index, "--description")?)?;
                index += 2;
            }
            "--run" => {
                set_once(&mut run, flag_value(rest, index, "--run")?)?;
                index += 2;
            }
            "--mount" => {
                mounts.push(flag_value(rest, index, "--mount")?.to_owned());
                index += 2;
            }
            other => {
                positional.push(other);
                index += 1;
            }
        }
    }
    let [name] = positional[..] else {
        return Err(CliError::usage(
            "recipe save: expected exactly one NAME (plus --mount TARGET[=PATH] ..., optional \
             --description TEXT, --run LINE, --force)",
        ));
    };
    super::validate_recipe_name(name)?;
    if mounts.is_empty() {
        return Err(CliError::usage(
            "recipe save: expected at least one --mount TARGET[=PATH]",
        ));
    }
    Ok(RecipeCommand::Save {
        name: name.to_owned(),
        description,
        run,
        mounts,
        force,
    })
}

fn parse_run(rest: &[&str]) -> Result<RecipeCommand, CliError> {
    let (before, extra) = match rest.iter().position(|word| *word == "--") {
        Some(split) => (&rest[..split], &rest[split + 1..]),
        None => (rest, &[][..]),
    };
    let [name] = before else {
        return Err(CliError::usage(
            "recipe run: expected exactly one NAME (extra command words go after --)",
        ));
    };
    super::validate_recipe_name(name)?;
    Ok(RecipeCommand::Run {
        name: (*name).to_owned(),
        extra: extra.iter().map(|word| (*word).to_owned()).collect(),
    })
}

fn set_once(slot: &mut Option<String>, value: &str) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError::usage(
            "recipe save: --description/--run given more than once",
        ));
    }
    *slot = Some(value.to_owned());
    Ok(())
}

fn flag_value<'a>(rest: &[&'a str], index: usize, flag: &str) -> Result<&'a str, CliError> {
    rest.get(index + 1)
        .copied()
        .ok_or_else(|| CliError::usage(format!("recipe save {flag} expects a value")))
}

/// Runs a parsed `recipe` subcommand against the default `~/.wanix/recipes`
/// and `~/.wanix/catalog`.
///
/// # Errors
///
/// Returns a CLI error when the directories cannot be resolved or the
/// subcommand fails.
pub(crate) fn run_recipe_command(
    command: RecipeCommand,
    process_stdin: &mut dyn Read,
) -> Result<CliOutput, CliError> {
    let recipes = default_recipes_dir()?;
    let catalog = crate::catalog::default_catalog_dir()?;
    match command {
        RecipeCommand::Save {
            name,
            description,
            run,
            mounts,
            force,
        } => run_save_in(&recipes, &catalog, name, description, run, &mounts, force),
        RecipeCommand::Run { name, extra } => {
            run_run_in(&recipes, &catalog, &name, &extra, process_stdin)
        }
    }
}
