use std::fs;
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::CliError;

pub(super) fn extract_tgz(archive_path: &Path, out_path: &Path) -> Result<(), CliError> {
    let file = fs::File::open(archive_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open rootfs archive {}: {error}",
                archive_path.display()
            ),
            1,
        )
    })?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let entries = archive.entries().map_err(|error| {
        CliError::new(
            format!(
                "failed to read rootfs archive {}: {error}",
                archive_path.display()
            ),
            1,
        )
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| {
            CliError::new(
                format!(
                    "failed to read rootfs archive entry from {}: {error}",
                    archive_path.display()
                ),
                1,
            )
        })?;
        let path = entry
            .path()
            .map_err(|error| {
                CliError::new(
                    format!(
                        "failed to read rootfs archive entry path from {}: {error}",
                        archive_path.display()
                    ),
                    1,
                )
            })?
            .into_owned();
        validate_archive_path(&path)?;
        let unpacked = entry.unpack_in(out_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to unpack rootfs archive entry {}: {error}",
                    path.display()
                ),
                1,
            )
        })?;
        if !unpacked {
            return Err(unsafe_archive_path_error(&path));
        }
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> Result<(), CliError> {
    let mut has_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_archive_path_error(path));
            }
        }
    }
    if has_normal {
        Ok(())
    } else {
        Err(CliError::new("rootfs archive contains an empty path", 1))
    }
}

fn unsafe_archive_path_error(path: &Path) -> CliError {
    CliError::new(format!("unsafe rootfs archive path {}", path.display()), 1)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::validate_archive_path;

    #[test]
    fn archive_path_validation_accepts_relative_entries() {
        validate_archive_path(Path::new("./bin/init")).unwrap();
    }

    #[test]
    fn archive_path_validation_rejects_escape_entries() {
        let error = validate_archive_path(Path::new("../escape.txt")).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("unsafe rootfs archive path"));
    }

    #[test]
    fn archive_path_validation_rejects_empty_entries() {
        let error = validate_archive_path(Path::new(".")).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("empty path"));
    }
}
