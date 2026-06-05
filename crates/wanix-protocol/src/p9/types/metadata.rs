/// Decoded payload for `Tgetattr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9GetAttr {
    /// Fid to stat.
    pub fid: u32,
    /// 9P2000.L attribute request mask.
    pub request_mask: u64,
}

/// Decoded payload for `Txattrwalk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9XattrWalk {
    /// Fid naming the file whose extended attribute is being opened.
    pub fid: u32,
    /// Fid to bind to the extended-attribute stream on success.
    pub newfid: u32,
    /// Extended attribute name, or empty string for the xattr name list.
    pub name: String,
}

/// Decoded payload for `Txattrcreate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9XattrCreate {
    /// Fid naming the file whose extended attribute should be created.
    pub fid: u32,
    /// Extended attribute name.
    pub name: String,
    /// Expected extended attribute byte length.
    pub attr_size: u64,
    /// Linux xattr create/replace flags.
    pub flags: u32,
}
