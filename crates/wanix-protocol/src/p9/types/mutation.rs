/// Decoded payload for `Tmkdir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Mkdir {
    /// Directory fid to create within.
    pub dir_fid: u32,
    /// Basename to create below the directory fid.
    pub name: String,
    /// POSIX mode requested for the new directory.
    pub mode: u32,
    /// Numeric group id requested for the new directory.
    pub gid: u32,
}

/// Decoded payload for `Tlink`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Link {
    /// Directory fid to create the hard-link name within.
    pub dir_fid: u32,
    /// Existing fid to link to.
    pub fid: u32,
    /// New hard-link basename below `dir_fid`.
    pub name: String,
}

/// Decoded payload for legacy `Trename`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Rename {
    /// Existing fid to move.
    pub fid: u32,
    /// Destination parent directory fid.
    pub dir_fid: u32,
    /// Destination basename below `dir_fid`.
    pub name: String,
}

/// Decoded payload for `Trenameat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9RenameAt {
    /// Source parent directory fid.
    pub old_dir_fid: u32,
    /// Source basename below `old_dir_fid`.
    pub old_name: String,
    /// Destination parent directory fid.
    pub new_dir_fid: u32,
    /// Destination basename below `new_dir_fid`.
    pub new_name: String,
}

/// Decoded payload for `Tunlinkat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9UnlinkAt {
    /// Parent directory fid.
    pub dir_fid: u32,
    /// Basename to remove below `dir_fid`.
    pub name: String,
    /// Linux `unlinkat` flags, including `AT_REMOVEDIR`.
    pub flags: u32,
}

/// Decoded payload for legacy `Tremove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Remove {
    /// Fid to remove and clunk.
    pub fid: u32,
}
