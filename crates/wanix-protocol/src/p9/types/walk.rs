use super::session::P9Qid;
use crate::p9::attrs::P9AttrBody;

/// Decoded payload for `Twalk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Walk {
    /// Existing fid to walk from.
    pub fid: u32,
    /// Fid to bind to the walked result.
    pub newfid: u32,
    /// Path components to walk.
    pub names: Vec<String>,
}

/// Decoded payload for `Rwalkgetattr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9WalkGetAttrResponse {
    /// Attribute bits the server considers valid.
    pub valid: u64,
    /// Attributes for the final walked fid, excluding the qid carried by `Rgetattr`.
    pub attr: P9AttrBody,
    /// QIDs returned for each walked path component.
    pub qids: Vec<P9Qid>,
}
