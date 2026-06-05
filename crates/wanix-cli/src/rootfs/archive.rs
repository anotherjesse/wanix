use std::fs;
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use tar::{Archive, Entry};

use crate::CliError;

pub(super) fn extract_tgz(archive_path: &Path, out_path: &Path) -> Result<(), CliError> {
    let mut archive = open_tgz_archive(archive_path)?;
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
        unpack_archive_entry(entry, archive_path, out_path)?;
    }
    Ok(())
}

fn open_tgz_archive(archive_path: &Path) -> Result<Archive<GzDecoder<fs::File>>, CliError> {
    let file = fs::File::open(archive_path).map_err(|error| {
        CliError::new(
            format!(
                "failed to open rootfs archive {}: {error}",
                archive_path.display()
            ),
            1,
        )
    })?;
    Ok(Archive::new(GzDecoder::new(file)))
}

fn unpack_archive_entry(
    entry: Result<Entry<'_, GzDecoder<fs::File>>, std::io::Error>,
    archive_path: &Path,
    out_path: &Path,
) -> Result<(), CliError> {
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
    if unpacked {
        Ok(())
    } else {
        Err(unsafe_archive_path_error(&path))
    }
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
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::{env, fs};

    use super::{extract_tgz, validate_archive_path};

    #[test]
    fn extract_tgz_unpacks_relative_entries() {
        let temp = temp_dir("wanix-cli-archive-extract");
        let archive = temp.join("rootfs.tgz");
        let out = temp.join("out");
        fs::create_dir(&out).unwrap();
        write_tgz_archive(
            &archive,
            &[("./bin/init", 0o755, b"#!/bin/sh\n".as_slice())],
        );

        extract_tgz(&archive, &out).unwrap();

        assert_eq!(fs::read(out.join("bin/init")).unwrap(), b"#!/bin/sh\n");
    }

    #[test]
    fn extract_tgz_reports_missing_archive() {
        let temp = temp_dir("wanix-cli-archive-missing");
        let archive = temp.join("missing.tgz");
        let out = temp.join("out");
        fs::create_dir(&out).unwrap();

        let error = extract_tgz(&archive, &out).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(
            error.to_string().contains("failed to open rootfs archive"),
            "{error}"
        );
    }

    #[test]
    fn extract_tgz_reports_malformed_archive() {
        let temp = temp_dir("wanix-cli-archive-malformed");
        let archive = temp.join("broken.tgz");
        let out = temp.join("out");
        fs::create_dir(&out).unwrap();
        fs::write(&archive, b"not a gzip tarball").unwrap();

        let error = extract_tgz(&archive, &out).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(
            error.to_string().contains("failed to read rootfs archive"),
            "{error}"
        );
    }

    #[test]
    fn extract_tgz_rejects_unsafe_entry_without_writing_escape() {
        let temp = temp_dir("wanix-cli-archive-unsafe");
        let archive = temp.join("unsafe.tgz");
        let out = temp.join("out");
        let escaped = temp.join("escape.txt");
        fs::create_dir(&out).unwrap();
        write_raw_tgz_entry(&archive, "../escape.txt", b"nope");

        let error = extract_tgz(&archive, &out).unwrap_err();

        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("unsafe rootfs archive path"));
        assert!(!escaped.exists());
    }

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

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = env::temp_dir();
        path.push(format!(
            "{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn write_tgz_archive(path: &Path, entries: &[(&str, u32, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, mode, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, *name, &mut &bytes[..])
                .unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn write_raw_tgz_entry(path: &Path, name: &str, bytes: &[u8]) {
        let file = fs::File::create(path).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        write_tar_octal(&mut header[100..108], 0o644);
        write_tar_octal(&mut header[108..116], 0);
        write_tar_octal(&mut header[116..124], 0);
        write_tar_octal(&mut header[124..136], bytes.len() as u64);
        write_tar_octal(&mut header[136..148], 0);
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let checksum = header.iter().map(|byte| u32::from(*byte)).sum::<u32>();
        let checksum = format!("{checksum:06o}\0 ");
        header[148..156].copy_from_slice(checksum.as_bytes());
        encoder.write_all(&header).unwrap();
        encoder.write_all(bytes).unwrap();
        let padding = (512 - (bytes.len() % 512)) % 512;
        encoder.write_all(&vec![0_u8; padding]).unwrap();
        encoder.write_all(&[0_u8; 1024]).unwrap();
        encoder.finish().unwrap();
    }

    fn write_tar_octal(field: &mut [u8], value: u64) {
        field.fill(0);
        let encoded = format!("{value:0width$o}", width = field.len() - 1);
        field[..encoded.len()].copy_from_slice(encoded.as_bytes());
    }
}
