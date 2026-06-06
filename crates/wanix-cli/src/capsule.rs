//! `wanix capsule` — freeze a Wanix world (a directory the agent built) into a
//! portable, content-addressed `.wcap` (a gzipped tar) and restore it
//! deterministically. The capsule id is a sha256 over the sorted file contents,
//! so the same world always yields the same id — a verifiable, shippable world.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

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
    dir: PathBuf,
    archive: PathBuf,
}

/// Parses `capsule save <DIR> <FILE.wcap>` / `capsule load <FILE.wcap> <DIR>`.
///
/// # Errors
///
/// Returns a usage error when the action or paths are missing.
pub(super) fn parse_capsule_command(args: &[OsString]) -> Result<CapsuleCommand, CliError> {
    let action = match args.first().and_then(|a| a.to_str()) {
        Some("save") => CapsuleAction::Save,
        Some("load") => CapsuleAction::Load,
        _ => return Err(CliError::usage("capsule: expected `save` or `load`")),
    };
    let first = args
        .get(1)
        .ok_or_else(|| CliError::usage("capsule: missing path"))?;
    let second = args
        .get(2)
        .ok_or_else(|| CliError::usage("capsule: missing path"))?;
    if args.len() > 3 {
        return Err(CliError::usage("capsule: too many arguments"));
    }
    let (dir, archive) = match action {
        CapsuleAction::Save => (PathBuf::from(first), PathBuf::from(second)),
        CapsuleAction::Load => (PathBuf::from(second), PathBuf::from(first)),
    };
    Ok(CapsuleCommand {
        action,
        dir,
        archive,
    })
}

/// Runs the capsule command.
///
/// # Errors
///
/// Returns an error when the directory or archive cannot be read or written.
pub(super) fn run_capsule_command(command: CapsuleCommand) -> Result<CliOutput, CliError> {
    let stdout = match command.action {
        CapsuleAction::Save => save(&command.dir, &command.archive)?,
        CapsuleAction::Load => load(&command.archive, &command.dir)?,
    };
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

fn save(dir: &Path, archive: &Path) -> Result<String, CliError> {
    let file = fs::File::create(archive)
        .map_err(|error| io_error(&format!("create {}", archive.display()), &error))?;
    let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::default()));
    builder
        .append_dir_all(".", dir)
        .map_err(|error| io_error("build capsule", &error))?;
    builder
        .into_inner()
        .and_then(GzEncoder::finish)
        .map_err(|error| io_error("finish capsule", &error))?;
    let (id, count) = world_id(dir)?;
    Ok(format!(
        "capsule {id} saved ({count} files) to {}\n",
        archive.display()
    ))
}

fn load(archive: &Path, dir: &Path) -> Result<String, CliError> {
    let file = fs::File::open(archive)
        .map_err(|error| io_error(&format!("open {}", archive.display()), &error))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    fs::create_dir_all(dir).map_err(|error| io_error("create target", &error))?;
    for entry in tar
        .entries()
        .map_err(|error| io_error("read capsule", &error))?
    {
        let mut entry = entry.map_err(|error| io_error("read capsule entry", &error))?;
        let path = entry
            .path()
            .map_err(|error| io_error("read capsule path", &error))?
            .into_owned();
        if !is_safe_relative(&path) {
            return Err(CliError::new(
                format!("capsule: unsafe path {}", path.display()),
                1,
            ));
        }
        entry
            .unpack(dir.join(&path))
            .map_err(|error| io_error("unpack capsule entry", &error))?;
    }
    let (id, count) = world_id(dir)?;
    Ok(format!(
        "capsule {id} restored ({count} files) to {}\n",
        dir.display()
    ))
}

fn is_safe_relative(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

/// Content-addressed id of a directory: sha256 over each file's `path\0sha256`,
/// in sorted path order (so the same world always yields the same id).
fn world_id(dir: &Path) -> Result<(String, usize), CliError> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();
    let mut root = Sha256::new();
    for (path, hash) in &files {
        root.update(path.as_bytes());
        root.update([0]);
        root.update(hash.as_bytes());
        root.update([b'\n']);
    }
    Ok((hex(&root.finalize()), files.len()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), CliError> {
    let entries = fs::read_dir(dir).map_err(|error| io_error("read dir", &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error("read dir entry", &error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().into_owned());
            out.push((relative, file_hash(&path)?));
        }
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String, CliError> {
    let mut file = fs::File::open(path).map_err(|error| io_error("open file", &error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|error| io_error("read file", &error))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn io_error(context: &str, error: &io::Error) -> CliError {
    CliError::new(format!("capsule: {context}: {error}"), 1)
}

#[cfg(test)]
mod tests;
