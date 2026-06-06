use wanix_fs::{FsError, FsResult, NormalizedPath};

/// A parsed `#pipe` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipePath<'a> {
    /// The device root directory.
    Root,
    /// The `new` allocation file.
    New,
    /// A channel directory `<id>`.
    Channel(&'a str),
    /// A channel's `<id>/id` file.
    Id(&'a str),
    /// A channel's `<id>/data` stream.
    Data(&'a str),
}

pub(crate) fn parse_path(path: &NormalizedPath) -> FsResult<PipePath<'_>> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(PipePath::Root);
    }
    let mut parts = raw.split('/');
    let first = parts.next().ok_or(FsError::NotFound)?;
    match (first, parts.next(), parts.next()) {
        ("new", None, _) => Ok(PipePath::New),
        (id, None, _) => Ok(PipePath::Channel(id)),
        (id, Some("id"), None) => Ok(PipePath::Id(id)),
        (id, Some("data"), None) => Ok(PipePath::Data(id)),
        _ => Err(FsError::NotFound),
    }
}
