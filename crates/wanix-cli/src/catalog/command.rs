//! `wanix catalog (add | show | rm | ls)` parsing and execution.
//!
//! `ls` is the liveness surface: by default it probes every entry with one
//! bounded dial (concurrent in chunks of [`MAX_CONCURRENT_PROBES`], each probe
//! never longer than the shared CLI mount deadline) and renders
//! `online`/`offline`/`unknown` —
//! pre-ACL, "offline" means "no route authenticated within the deadline",
//! which is all Layer 1 can know (ADR 0007/0008). `--no-probe` lists
//! instantly with a `-` status column.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use wanix_id::NodeIdentity;

use super::{
    CatalogEntry, default_catalog_dir, list_entries, read_entry, remove_entry, write_entry,
};
use crate::mesh::{CLI_MESH_MOUNT_DEADLINE, ProbeOutcome, dialer_identity, probe_iroh};
use crate::{CliError, CliOutput};

/// One parsed `catalog` subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CatalogCommand {
    Add { entry: CatalogEntry, force: bool },
    Show { name: String },
    Rm { name: String },
    Ls { probe: bool },
}

/// Parses `catalog (add NAME IROH_URL [--description TEXT] [--tags a,b]
/// [--force] | show NAME | rm NAME | ls [--no-probe])`.
///
/// # Errors
///
/// Returns a usage error when the subcommand is missing/unknown, an operand or
/// flag value is missing, duplicated, or invalid, or the name/address does not
/// fit the catalog grammar.
pub(crate) fn parse_catalog_command(args: &[OsString]) -> Result<CatalogCommand, CliError> {
    let mut words = Vec::with_capacity(args.len());
    for arg in args {
        words.push(
            arg.to_str()
                .ok_or_else(|| CliError::usage("catalog: arguments must be valid UTF-8"))?,
        );
    }
    let Some((&verb, rest)) = words.split_first() else {
        return Err(CliError::usage(
            "catalog: expected a subcommand (add NAME IROH_URL | show NAME | rm NAME | ls)",
        ));
    };
    match verb {
        "add" => parse_add(rest),
        "show" => Ok(CatalogCommand::Show {
            name: one_name(rest, "show")?,
        }),
        "rm" => Ok(CatalogCommand::Rm {
            name: one_name(rest, "rm")?,
        }),
        "ls" => match rest {
            [] => Ok(CatalogCommand::Ls { probe: true }),
            ["--no-probe"] => Ok(CatalogCommand::Ls { probe: false }),
            _ => Err(CliError::usage("catalog ls: takes only [--no-probe]")),
        },
        other => Err(CliError::usage(format!(
            "catalog: unknown subcommand {other:?} (expected add | show | rm | ls)"
        ))),
    }
}

fn parse_add(rest: &[&str]) -> Result<CatalogCommand, CliError> {
    let mut positional = Vec::new();
    let mut description = None;
    let mut tags = Vec::new();
    let mut force = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index] {
            "--force" => {
                force = true;
                index += 1;
            }
            "--description" => {
                if description.is_some() {
                    return Err(CliError::usage(
                        "catalog add: --description given more than once",
                    ));
                }
                description = Some(flag_value(rest, index, "--description")?.to_owned());
                index += 2;
            }
            "--tags" => {
                if !tags.is_empty() {
                    return Err(CliError::usage("catalog add: --tags given more than once"));
                }
                tags = flag_value(rest, index, "--tags")?
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                if tags.is_empty() {
                    return Err(CliError::usage(
                        "catalog add: --tags expects a comma-separated list (e.g. photo,volume)",
                    ));
                }
                index += 2;
            }
            other => {
                positional.push(other);
                index += 1;
            }
        }
    }
    let [name, address] = positional[..] else {
        return Err(CliError::usage(
            "catalog add: expected exactly NAME and IROH_URL (plus optional --description TEXT, \
             --tags a,b, --force)",
        ));
    };
    super::validate_catalog_name(name)?;
    super::validate_catalog_address(address)?;
    Ok(CatalogCommand::Add {
        entry: CatalogEntry {
            name: name.to_owned(),
            description,
            tags,
            address: address.to_owned(),
        },
        force,
    })
}

fn flag_value<'a>(rest: &[&'a str], index: usize, flag: &str) -> Result<&'a str, CliError> {
    rest.get(index + 1)
        .copied()
        .ok_or_else(|| CliError::usage(format!("catalog add {flag} expects a value")))
}

fn one_name(rest: &[&str], verb: &str) -> Result<String, CliError> {
    let [name] = rest else {
        return Err(CliError::usage(format!(
            "catalog {verb}: expected exactly one NAME"
        )));
    };
    super::validate_catalog_name(name)?;
    Ok((*name).to_owned())
}

/// Runs a parsed `catalog` subcommand against the default `~/.wanix/catalog`,
/// probing `ls` with the persisted dialer identity and the shared CLI mount
/// deadline.
///
/// # Errors
///
/// Returns a CLI error when the catalog directory cannot be resolved or the
/// subcommand fails.
pub(crate) fn run_catalog_command(command: CatalogCommand) -> Result<CliOutput, CliError> {
    let dir = default_catalog_dir()?;
    match command {
        CatalogCommand::Add { entry, force } => run_add_in(&dir, &entry, force),
        CatalogCommand::Show { name } => run_show_in(&dir, &name),
        CatalogCommand::Rm { name } => run_rm_in(&dir, &name),
        CatalogCommand::Ls { probe } => {
            let probe = probe
                .then(|| -> Result<LsProbe, CliError> {
                    Ok(LsProbe {
                        identity: dialer_identity()?,
                        deadline: CLI_MESH_MOUNT_DEADLINE,
                    })
                })
                .transpose()?;
            run_ls_in(&dir, probe.as_ref())
        }
    }
}

pub(crate) fn run_add_in(
    dir: &Path,
    entry: &CatalogEntry,
    force: bool,
) -> Result<CliOutput, CliError> {
    let path = write_entry(dir, entry, force)?;
    let line = format!(
        "added catalog entry {} -> {} ({})\n",
        entry.name,
        entry.address,
        path.display()
    );
    Ok(CliOutput::new(line.into_bytes(), Vec::new(), 0))
}

pub(crate) fn run_show_in(dir: &Path, name: &str) -> Result<CliOutput, CliError> {
    let entry = read_entry(dir, name)?;
    let mut text = format!("name: {}\naddress: {}\n", entry.name, entry.address);
    if let Some(description) = &entry.description {
        text.push_str(&format!("description: {description}\n"));
    }
    if !entry.tags.is_empty() {
        text.push_str(&format!("tags: {}\n", entry.tags.join(", ")));
    }
    Ok(CliOutput::new(text.into_bytes(), Vec::new(), 0))
}

pub(crate) fn run_rm_in(dir: &Path, name: &str) -> Result<CliOutput, CliError> {
    remove_entry(dir, name)?;
    let line = format!("removed catalog entry {name}\n");
    Ok(CliOutput::new(line.into_bytes(), Vec::new(), 0))
}

/// How `catalog ls` probes: the dialing principal and the per-entry deadline.
pub(crate) struct LsProbe {
    pub(crate) identity: NodeIdentity,
    pub(crate) deadline: Duration,
}

/// Lists the catalog at `dir` as one `NAME\tSTATUS\tADDRESS` line per entry,
/// sorted by name. With a probe, STATUS is `online`/`offline`/`unknown (...)`
/// from one bounded dial per entry (concurrent in chunks of
/// [`MAX_CONCURRENT_PROBES`], so the whole listing is bounded by one deadline
/// per chunk, not the sum); without one it is `-`.
///
/// # Errors
///
/// Returns a CLI error when the catalog cannot be listed.
pub(crate) fn run_ls_in(dir: &Path, probe: Option<&LsProbe>) -> Result<CliOutput, CliError> {
    let entries = list_entries(dir)?;
    let statuses = match probe {
        Some(probe) if !entries.is_empty() => probe_statuses(&entries, probe),
        _ => vec!["-".to_owned(); entries.len()],
    };
    let mut text = String::new();
    for (entry, status) in entries.iter().zip(statuses) {
        text.push_str(&format!("{}\t{status}\t{}\n", entry.name, entry.address));
    }
    Ok(CliOutput::new(text.into_bytes(), Vec::new(), 0))
}

/// Cap on concurrent probes. Each probe is a blocking dial that binds its own
/// dial-out endpoint, exactly like a mount — a full multi-thread tokio runtime
/// plus an iroh socket — so an unbounded fan-out across a large address book
/// exhausts threads and fds. Entries are probed in chunks of this size.
const MAX_CONCURRENT_PROBES: usize = 8;

/// One bounded dial per entry, concurrently within each
/// [`MAX_CONCURRENT_PROBES`]-sized chunk (scoped threads).
fn probe_statuses(entries: &[CatalogEntry], probe: &LsProbe) -> Vec<String> {
    let mut statuses = Vec::with_capacity(entries.len());
    for chunk in entries.chunks(MAX_CONCURRENT_PROBES) {
        statuses.extend(std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|entry| {
                    let address = &entry.address;
                    scope.spawn(move || probe_iroh(&probe.identity, address, probe.deadline))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    render_probe(
                        handle
                            .join()
                            .unwrap_or_else(|_| ProbeOutcome::Unknown("probe panicked".to_owned())),
                    )
                })
                .collect::<Vec<_>>()
        }));
    }
    statuses
}

fn render_probe(outcome: ProbeOutcome) -> String {
    match outcome {
        ProbeOutcome::Online => "online".to_owned(),
        ProbeOutcome::Offline => "offline".to_owned(),
        ProbeOutcome::Unknown(detail) => format!("unknown ({detail})"),
    }
}
