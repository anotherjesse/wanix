//! Namespace operations behind the `mount` verbs.
//!
//! Each function operates on an already-built [`Namespace`] (the remote bound at
//! `/n/remote`) so the same code runs from the CLI (a dialed TCP `RemoteFs`) and
//! from tests (a locally bound `RemoteFs`). The point is that the bytes flow
//! through the namespace and the remote filesystem, never a local shortcut.

use wanix_fs::{File, FileSystem, NormalizedPath, OpenOptions};
use wanix_vfs::Namespace;

use crate::{CliError, CliOutput};

/// Upper bound on bytes streamed out of a single `mount-cat`/`mount-write`.
///
/// Service streams (`#term`, `#task`) are not regular files and never report a
/// length, so reads are bounded by iteration count rather than a stat size.
const MAX_STREAM_BYTES: usize = 1 << 20;

/// Lists the directory at `path` and prints one entry name per line.
pub(super) fn mount_ls(
    namespace: &Namespace,
    path: &NormalizedPath,
) -> Result<CliOutput, CliError> {
    let mut names: Vec<String> = namespace
        .read_dir(path)
        .map_err(|error| CliError::new(format!("mount-ls failed: {error}"), 1))?
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    names.sort();
    let mut stdout = names.join("\n");
    if !stdout.is_empty() {
        stdout.push('\n');
    }
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

/// Reads the file or stream at `path` and prints its bytes verbatim.
///
/// Non-seekable service streams are read with the same bounded loop as regular
/// files: there is no fabricated offset, so the bytes printed are exactly the
/// bytes the server delivered.
pub(super) fn mount_cat(
    namespace: &Namespace,
    path: &NormalizedPath,
) -> Result<CliOutput, CliError> {
    let file = namespace
        .open(path, OpenOptions::read())
        .map_err(|error| CliError::new(format!("mount-cat failed to open: {error}"), 1))?;
    let bytes = read_all(file)?;
    Ok(CliOutput::new(bytes, Vec::new(), 0))
}

/// Streams the file at `path` to `stdout` incrementally until end-of-file.
///
/// One open handle, no byte cap: each chunk is written and flushed as the
/// server delivers it, so a never-EOF device stream (`#pipe/<id>/data`,
/// `#task/<id>/wait`) is consumable live — the read simply parks until the
/// remote has bytes. The loop ends only at EOF or on an error; interrupting a
/// stream that never ends is plain process exit (Ctrl-C).
pub(super) fn mount_cat_follow(
    namespace: &Namespace,
    path: &NormalizedPath,
    stdout: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let mut file = namespace
        .open(path, OpenOptions::read())
        .map_err(|error| CliError::new(format!("mount-cat failed to open: {error}"), 1))?;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| CliError::new(format!("mount-cat failed to read: {error}"), 1))?;
        if read == 0 {
            return Ok(());
        }
        stdout
            .write_all(&chunk[..read])
            .and_then(|()| stdout.flush())
            .map_err(|error| CliError::new(format!("mount-cat failed to write: {error}"), 1))?;
    }
}

/// Writes `text` to the file at `path`, creating or truncating it.
pub(super) fn mount_write(
    namespace: &Namespace,
    path: &NormalizedPath,
    text: &[u8],
) -> Result<CliOutput, CliError> {
    let mut file = namespace
        .open(
            path,
            OpenOptions {
                read: false,
                write: true,
                create: true,
                truncate: true,
            },
        )
        .map_err(|error| CliError::new(format!("mount-write failed to open: {error}"), 1))?;
    write_all(file.as_mut(), text)?;
    let stdout = format!("wrote {} bytes to {}\n", text.len(), path.as_str());
    Ok(CliOutput::new(stdout.into_bytes(), Vec::new(), 0))
}

/// Reads a file handle to EOF (or the stream cap) with a bounded loop.
fn read_all(mut file: Box<dyn File>) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| CliError::new(format!("mount-cat failed to read: {error}"), 1))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() >= MAX_STREAM_BYTES {
            break;
        }
    }
    Ok(bytes)
}

/// Writes every byte of `text`, looping over short writes.
fn write_all(file: &mut dyn File, text: &[u8]) -> Result<(), CliError> {
    let mut written = 0;
    while written < text.len() {
        let n = file
            .write(&text[written..])
            .map_err(|error| CliError::new(format!("mount-write failed: {error}"), 1))?;
        if n == 0 {
            return Err(CliError::new("mount-write made no progress", 1));
        }
        written += n;
    }
    Ok(())
}
