use wanix_fs::{FsError, FsResult, NormalizedPath};

/// A parsed `#agent` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentPath<'a> {
    /// The device root directory.
    Root,
    /// The `new` allocation file.
    New,
    /// A session directory `<id>`.
    Session(&'a str),
    /// `<id>/id`: the session id.
    Id(&'a str),
    /// `<id>/prompt`: write to submit a turn.
    Prompt(&'a str),
    /// `<id>/events`: read the normalized JSONL event stream.
    Events(&'a str),
    /// `<id>/ctl`: write control verbs (`close`, `approve`, `deny`).
    Ctl(&'a str),
    /// `<id>/status`: read a one-line status snapshot.
    Status(&'a str),
    /// `<id>/pending`: read open approval requests as a JSON array.
    Pending(&'a str),
}

pub(crate) fn parse_path(path: &NormalizedPath) -> FsResult<AgentPath<'_>> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(AgentPath::Root);
    }
    let mut parts = raw.split('/');
    let first = parts.next().ok_or(FsError::NotFound)?;
    match (first, parts.next(), parts.next()) {
        ("new", None, _) => Ok(AgentPath::New),
        (id, None, _) => Ok(AgentPath::Session(id)),
        (id, Some("id"), None) => Ok(AgentPath::Id(id)),
        (id, Some("prompt"), None) => Ok(AgentPath::Prompt(id)),
        (id, Some("events"), None) => Ok(AgentPath::Events(id)),
        (id, Some("ctl"), None) => Ok(AgentPath::Ctl(id)),
        (id, Some("status"), None) => Ok(AgentPath::Status(id)),
        (id, Some("pending"), None) => Ok(AgentPath::Pending(id)),
        _ => Err(FsError::NotFound),
    }
}
