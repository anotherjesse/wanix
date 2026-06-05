/// Decoded payload for `Tlopen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Open {
    /// Fid to open.
    pub fid: u32,
    /// Linux open flags carried by 9P2000.L.
    pub flags: u32,
}

/// Decoded payload for `Tlcreate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Create {
    /// Directory fid to create within. The fid becomes the opened file on success.
    pub fid: u32,
    /// Basename to create below the directory fid.
    pub name: String,
    /// Linux open flags carried by 9P2000.L.
    pub flags: u32,
    /// POSIX mode requested for the new file.
    pub mode: u32,
    /// Numeric group id requested for the new file.
    pub gid: u32,
}

/// Decoded payload for `Tsymlink`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Symlink {
    /// Directory fid to create the symlink within.
    pub dir_fid: u32,
    /// New symlink basename below `dir_fid`.
    pub name: String,
    /// Uninterpreted symlink target string.
    pub target: String,
    /// Numeric group id requested for the new link.
    pub gid: u32,
}

/// Decoded payload for `Tmknod`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Mknod {
    /// Directory fid to create the special file within.
    pub dir_fid: u32,
    /// New node basename below `dir_fid`.
    pub name: String,
    /// POSIX mode requested for the new node.
    pub mode: u32,
    /// Device major number.
    pub major: u32,
    /// Device minor number.
    pub minor: u32,
    /// Numeric group id requested for the new node.
    pub gid: u32,
}

/// Decoded payload for `Treadlink`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9ReadLink {
    /// Fid naming the symlink to read.
    pub fid: u32,
}
