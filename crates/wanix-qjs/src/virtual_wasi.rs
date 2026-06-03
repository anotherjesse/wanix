use std::collections::HashSet;

use rust_wasi_quickjs::QuickJsHostConfig;
use wanix_fs::{FileSystem, FileType, FsError, FsResult, NormalizedPath, OpenOptions};

pub(crate) fn with_namespace_read_only_files(
    config: QuickJsHostConfig,
    namespace: &impl FileSystem,
) -> FsResult<QuickJsHostConfig> {
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    collect_files(
        namespace,
        &NormalizedPath::new(".")?,
        &mut visited,
        &mut files,
    )?;
    with_read_only_files(config, files)
}

fn with_read_only_files(
    mut config: QuickJsHostConfig,
    files: Vec<(NormalizedPath, Vec<u8>)>,
) -> FsResult<QuickJsHostConfig> {
    for (path, bytes) in files {
        let guest_path = format!("/{}", path.as_str());
        config = config
            .with_read_only_virtual_file(&guest_path, bytes)
            .map_err(|err| {
                FsError::Other(format!(
                    "failed to export {path} into QuickJS WASI virtual FS: {err:#}"
                ))
            })?;
    }
    Ok(config)
}

fn collect_files(
    namespace: &impl FileSystem,
    path: &NormalizedPath,
    visited: &mut HashSet<NormalizedPath>,
    files: &mut Vec<(NormalizedPath, Vec<u8>)>,
) -> FsResult<()> {
    if !visited.insert(path.clone()) {
        return Ok(());
    }
    for entry in namespace.read_dir(path)? {
        let child = child_path(path, entry.name())?;
        match entry.metadata().file_type() {
            FileType::File => files.push((child.clone(), read_file(namespace, &child)?)),
            FileType::Directory => collect_files(namespace, &child, visited, files)?,
            FileType::Symlink => {}
        }
    }
    Ok(())
}

fn child_path(parent: &NormalizedPath, name: &str) -> FsResult<NormalizedPath> {
    if parent.as_str() == "." {
        NormalizedPath::new(name)
    } else {
        NormalizedPath::new(format!("{}/{name}", parent.as_str()))
    }
}

fn read_file(namespace: &impl FileSystem, path: &NormalizedPath) -> FsResult<Vec<u8>> {
    let mut file = namespace.open(path, OpenOptions::read())?;
    let mut bytes = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rust_wasi_quickjs::QuickJsHostConfig;
    use wanix_fs::MemFs;
    use wanix_vfs::{BindOptions, Namespace};

    use super::with_namespace_read_only_files;

    #[test]
    fn namespace_files_are_projected_into_quickjs_virtual_wasi_config() {
        let mut namespace = Namespace::new();
        let root = Arc::new(MemFs::new());
        root.write_file("main.js", b"print('main')").unwrap();
        root.write_file("dir/lib.js", b"export const value = 1;")
            .unwrap();
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();

        let config = with_namespace_read_only_files(QuickJsHostConfig::new(), &namespace).unwrap();

        assert_eq!(config.read_only_virtual_file_count(), 2);
    }
}
