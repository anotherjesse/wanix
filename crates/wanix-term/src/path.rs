use wanix_fs::{FsError, FsResult, NormalizedPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TermPath<'a> {
    Root,
    New,
    Resource(&'a str),
    Id(&'a str),
    Ctl(&'a str),
    Data(&'a str),
    Program(&'a str),
    Winch(&'a str),
}

pub(crate) fn parse_path(path: &NormalizedPath) -> FsResult<TermPath<'_>> {
    if path.as_str() == "." {
        return Ok(TermPath::Root);
    }
    let parts = path.as_str().split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["new"] => Ok(TermPath::New),
        [id] => Ok(TermPath::Resource(id)),
        [id, "id"] => Ok(TermPath::Id(id)),
        [id, "ctl"] => Ok(TermPath::Ctl(id)),
        [id, "data"] => Ok(TermPath::Data(id)),
        [id, "program"] => Ok(TermPath::Program(id)),
        [id, "winch"] => Ok(TermPath::Winch(id)),
        _ => Err(FsError::NotFound),
    }
}
