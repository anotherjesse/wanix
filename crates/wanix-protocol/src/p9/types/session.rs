/// Decoded payload for `Tversion` and `Rversion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Version {
    /// Maximum 9P message size requested or accepted by the peer.
    pub msize: u32,
    /// Version string, such as `9P2000.L`.
    pub version: String,
}

/// 9P QID value used to identify files across a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Qid {
    /// QID type byte.
    pub qid_type: u8,
    /// Server-controlled version value.
    pub version: u32,
    /// Server-controlled path identity.
    pub path: u64,
}

/// Decoded payload for `Tauth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Auth {
    /// Fid to attach authentication state to.
    pub afid: u32,
    /// User name string.
    pub uname: String,
    /// Attach name string.
    pub aname: String,
    /// Numeric user id in 9P2000.L auth messages.
    pub n_uname: u32,
}

/// Decoded payload for `Tattach`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Attach {
    /// Fid to attach to the root of the selected tree.
    pub fid: u32,
    /// Auth fid, or `P9_NOFID` when unauthenticated.
    pub afid: u32,
    /// User name string.
    pub uname: String,
    /// Attach name string.
    pub aname: String,
    /// Numeric user id in 9P2000.L attach messages.
    pub n_uname: u32,
}
