use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug)]
pub(in crate::serve) struct RootChangeTracker {
    root: PathBuf,
    snapshot: BTreeMap<String, EntrySignature>,
}

impl RootChangeTracker {
    pub(in crate::serve) fn new(root: &Path) -> io::Result<Self> {
        Ok(Self {
            root: root.to_path_buf(),
            snapshot: scan_root(root)?,
        })
    }

    pub(in crate::serve) fn take_changed_paths(&mut self) -> io::Result<Vec<String>> {
        let next = scan_root(&self.root)?;
        let mut changed = BTreeSet::new();
        for (path, signature) in &next {
            if self.snapshot.get(path) != Some(signature) {
                changed.insert(path.clone());
            }
        }
        for path in self.snapshot.keys() {
            if !next.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        self.snapshot = next;
        Ok(changed.into_iter().collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntrySignature {
    kind: EntryKind,
    len: u64,
    modified_nanos: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

fn scan_root(root: &Path) -> io::Result<BTreeMap<String, EntrySignature>> {
    let mut entries = BTreeMap::new();
    scan_dir(root, root, &mut entries)?;
    Ok(entries)
}

fn scan_dir(
    root: &Path,
    dir: &Path,
    entries: &mut BTreeMap<String, EntrySignature>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        entries.insert(wanix_path(root, &path), signature(&metadata));
        if metadata.is_dir() {
            scan_dir(root, &path, entries)?;
        }
    }
    Ok(())
}

fn signature(metadata: &fs::Metadata) -> EntrySignature {
    let file_type = metadata.file_type();
    EntrySignature {
        kind: if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::File
        } else if file_type.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::Other
        },
        len: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos()),
    }
}

fn wanix_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let components = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        });
    format!("/{}", components.collect::<Vec<_>>().join("/"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::RootChangeTracker;

    #[test]
    fn tracks_created_modified_and_removed_paths() {
        let root = unique_temp_dir("wanix-root-change-tracker");
        fs::create_dir(&root).unwrap();
        let mut tracker = RootChangeTracker::new(&root).unwrap();

        fs::write(root.join("made.txt"), "one").unwrap();
        assert_eq!(tracker.take_changed_paths().unwrap(), ["/made.txt"]);

        fs::write(root.join("made.txt"), "two").unwrap();
        assert_eq!(tracker.take_changed_paths().unwrap(), ["/made.txt"]);

        fs::remove_file(root.join("made.txt")).unwrap();
        assert_eq!(tracker.take_changed_paths().unwrap(), ["/made.txt"]);

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{label}-{}-{nanos}", std::process::id()))
    }
}
