use std::path::Path;

use wanix_fs::{File, FileSystem, MemFs, NormalizedPath, OpenOptions};

use crate::CliError;

const WANIX_FILE_READ_CHUNK_BYTES: usize = 1024;

pub(crate) fn copy_script_directory(
    script_path: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    copy_script_directory_into(script_path, root, cwd, ".")
}

pub(crate) fn copy_script_directory_into(
    script_path: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
    guest_dir: &str,
) -> Result<(), CliError> {
    let base = script_path.parent().unwrap_or_else(|| Path::new("."));
    let guest_dir = NormalizedPath::new(guest_dir)?;
    copy_directory_tree(base, base, root, cwd, &guest_dir)
}

fn copy_directory_tree(
    base: &Path,
    dir: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
    guest_dir: &NormalizedPath,
) -> Result<(), CliError> {
    for entry in read_directory_entries(dir)? {
        copy_host_tree_entry(base, entry, root, cwd, guest_dir)?;
    }
    Ok(())
}

fn read_directory_entries(dir: &Path) -> Result<Vec<HostTreeEntry>, CliError> {
    std::fs::read_dir(dir)
        .map_err(|error| {
            CliError::new(
                format!("failed to read directory {}: {error}", dir.display()),
                1,
            )
        })?
        .map(|entry| host_tree_entry(dir, entry))
        .collect()
}

fn host_tree_entry(
    dir: &Path,
    entry: Result<std::fs::DirEntry, std::io::Error>,
) -> Result<HostTreeEntry, CliError> {
    let entry = entry.map_err(|error| {
        CliError::new(
            format!(
                "failed to read directory entry in {}: {error}",
                dir.display()
            ),
            1,
        )
    })?;
    let path = entry.path();
    let file_type = entry
        .file_type()
        .map_err(|error| CliError::new(format!("failed to stat {}: {error}", path.display()), 1))?;
    Ok(HostTreeEntry::from_path(path, file_type))
}

enum HostTreeEntry {
    Directory(std::path::PathBuf),
    File(std::path::PathBuf),
    Other,
}

impl HostTreeEntry {
    fn from_path(path: std::path::PathBuf, file_type: std::fs::FileType) -> Self {
        if file_type.is_dir() {
            return Self::Directory(path);
        }
        if file_type.is_file() {
            return Self::File(path);
        }
        Self::Other
    }
}

fn copy_host_tree_entry(
    base: &Path,
    entry: HostTreeEntry,
    root: &MemFs,
    cwd: &NormalizedPath,
    guest_dir: &NormalizedPath,
) -> Result<(), CliError> {
    match entry {
        HostTreeEntry::Directory(path) => copy_directory_tree(base, &path, root, cwd, guest_dir),
        HostTreeEntry::File(path) => copy_host_file(base, &path, root, cwd, guest_dir),
        HostTreeEntry::Other => Ok(()),
    }
}

fn copy_host_file(
    base: &Path,
    path: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
    guest_dir: &NormalizedPath,
) -> Result<(), CliError> {
    let guest_path = guest_file_path(base, path, cwd, guest_dir)?;
    copy_host_file_if_mapped(path, root, guest_path)
}

fn guest_file_path(
    base: &Path,
    path: &Path,
    cwd: &NormalizedPath,
    guest_dir: &NormalizedPath,
) -> Result<Option<String>, CliError> {
    let Some(guest_path) = guest_path_for_host_file(base, path)? else {
        return Ok(None);
    };
    let guest_path = guest_path_under_dir(guest_dir, &guest_path)?;
    let guest_path = guest_path_in_cwd(cwd, &guest_path)?;
    Ok(Some(guest_path))
}

fn copy_host_file_if_mapped(
    path: &Path,
    root: &MemFs,
    guest_path: Option<String>,
) -> Result<(), CliError> {
    let Some(guest_path) = guest_path else {
        return Ok(());
    };
    let bytes = read_host_file(path)?;
    root.write_file(guest_path, bytes)?;
    Ok(())
}

fn read_host_file(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path)
        .map_err(|error| CliError::new(format!("failed to read {}: {error}", path.display()), 1))
}

fn guest_path_under_dir(guest_dir: &NormalizedPath, path: &str) -> Result<String, CliError> {
    if guest_dir.as_str() == "." {
        return Ok(path.to_owned());
    }
    Ok(NormalizedPath::new(format!("{guest_dir}/{path}"))?.to_string())
}

pub(crate) fn guest_path_in_cwd(cwd: &NormalizedPath, path: &str) -> Result<String, CliError> {
    if cwd.as_str() == "." {
        return Ok(path.to_owned());
    }
    Ok(NormalizedPath::new(format!("{cwd}/{path}"))?.to_string())
}

fn guest_path_for_host_file(base: &Path, path: &Path) -> Result<Option<String>, CliError> {
    let relative = path.strip_prefix(base).map_err(|error| {
        CliError::new(
            format!(
                "failed to map {} under {}: {error}",
                path.display(),
                base.display()
            ),
            1,
        )
    })?;
    let Some(path) = relative.to_str() else {
        return Ok(None);
    };
    let path = path.replace(std::path::MAIN_SEPARATOR, "/");
    if path.is_empty() || NormalizedPath::new(&path).is_err() {
        return Ok(None);
    }
    Ok(Some(path))
}

pub(crate) fn read_utf8_script(path: &Path) -> Result<String, CliError> {
    let script = std::fs::read(path)
        .map_err(|error| CliError::new(format!("failed to read {}: {error}", path.display()), 1))?;
    String::from_utf8(script).map_err(|error| {
        CliError::new(
            format!("script {} is not valid UTF-8: {error}", path.display()),
            1,
        )
    })
}

pub(crate) fn read_file(fs: &dyn FileSystem, path: &str) -> Result<Vec<u8>, CliError> {
    let mut file = fs.open(&NormalizedPath::new(path)?, OpenOptions::read())?;
    let mut out = Vec::new();
    while read_next_chunk(file.as_mut(), &mut out)? {}
    Ok(out)
}

fn read_next_chunk(file: &mut dyn File, out: &mut Vec<u8>) -> Result<bool, CliError> {
    let mut buf = [0; WANIX_FILE_READ_CHUNK_BYTES];
    let n = file.read(&mut buf)?;
    if n == 0 {
        return Ok(false);
    }
    out.extend_from_slice(&buf[..n]);
    Ok(true)
}
