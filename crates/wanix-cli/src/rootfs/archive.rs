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
        if !entry.unpack_in(out_path).map_err(|error| {
            CliError::new(
                format!(
                    "failed to unpack rootfs archive entry {}: {error}",
                    path.display()
                ),
                1,
            )
        })? {
            return Err(CliError::new(
                format!("unsafe rootfs archive path {}", path.display()),
                1,
            ));
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
                return Err(CliError::new(
                    format!("unsafe rootfs archive path {}", path.display()),
                    1,
                ));
            }
        }
    }
    if has_normal {
        Ok(())
    } else {
        Err(CliError::new("rootfs archive contains an empty path", 1))
    }
}
