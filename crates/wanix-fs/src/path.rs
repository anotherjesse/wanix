use std::fmt;

use crate::{FsError, FsResult};

/// Wanix file path normalized to the `io/fs.ValidPath` shape used by Go.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedPath(String);

impl NormalizedPath {
    /// Creates a normalized path after validating relative path components.
    ///
    /// `.` is the filesystem root. Other paths must be slash-separated,
    /// relative, non-empty, and contain no `.` or `..` components.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::InvalidPath`] when the input is not a valid Wanix
    /// filesystem path.
    pub fn new(path: impl AsRef<str>) -> FsResult<Self> {
        let path = path.as_ref();
        if is_valid_path(path) {
            Ok(Self(path.to_owned()))
        } else {
            Err(FsError::InvalidPath(path.to_owned()))
        }
    }

    /// Returns the normalized path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the parent path, or `None` for root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.0 == "." {
            return None;
        }
        match self.0.rsplit_once('/') {
            Some((parent, _name)) => Some(Self(parent.to_owned())),
            None => Some(Self(".".to_owned())),
        }
    }

    /// Returns the final path component.
    #[must_use]
    pub fn file_name(&self) -> &str {
        if self.0 == "." {
            "."
        } else {
            self.0.rsplit('/').next().unwrap_or(self.0.as_str())
        }
    }
}

impl fmt::Display for NormalizedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn is_valid_path(path: &str) -> bool {
    if path == "." {
        return true;
    }
    if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
        return false;
    }
    path.split('/')
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

#[cfg(test)]
mod tests {
    use super::NormalizedPath;
    use crate::FsError;

    #[test]
    fn normalized_path_matches_valid_path_shape() {
        for path in [".", "#task", "dir/file", "a-b/c_d"] {
            assert_eq!(NormalizedPath::new(path).unwrap().as_str(), path);
        }

        for path in [
            "",
            "/",
            "/abs",
            "dir/",
            "dir//file",
            "dir/.",
            "../x",
            "x/../y",
        ] {
            assert!(matches!(
                NormalizedPath::new(path),
                Err(FsError::InvalidPath(_))
            ));
        }
    }

    #[test]
    fn parent_and_file_name_are_available() {
        let path = NormalizedPath::new("a/b/c").unwrap();

        assert_eq!(path.file_name(), "c");
        assert_eq!(path.parent().unwrap().as_str(), "a/b");
        assert_eq!(path.parent().unwrap().parent().unwrap().as_str(), "a");
        assert_eq!(
            path.parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .as_str(),
            "."
        );
        assert!(NormalizedPath::new(".").unwrap().parent().is_none());
    }
}
