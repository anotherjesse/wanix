use super::attrs::P9AttrBody;

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

/// Decoded payload for `Rlerror`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Lerror {
    /// Linux errno value reported by a 9P2000.L server.
    pub ecode: u32,
}

/// Decoded payload for `Tstatfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9StatFs {
    /// Fid whose filesystem should be reported.
    pub fid: u32,
}

/// Decoded payload for `Tfsync`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Fsync {
    /// Fid to synchronize.
    pub fid: u32,
}

/// Decoded payload for `Tflush`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Flush {
    /// Tag of the request being flushed.
    pub oldtag: u16,
}

/// Decoded payload for `Tflushf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9FlushF {
    /// Fid whose pending file state should be flushed.
    pub fid: u32,
}

/// 9P2000.L record-lock range fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Lock {
    /// Lock kind: read, write, or unlock.
    pub lock_type: u8,
    /// Starting byte offset for the lock range.
    pub start: u64,
    /// Number of bytes in the lock range.
    pub length: u64,
    /// Process id associated with the lock request.
    pub proc_id: u32,
    /// Client id string, usually the Linux v9fs client nodename.
    pub client_id: String,
}

/// Decoded payload for `Tlock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9LockRequest {
    /// Fid to lock or unlock.
    pub fid: u32,
    /// 9P2000.L lock flags.
    pub flags: u32,
    /// Requested lock range.
    pub lock: P9Lock,
}

/// Decoded payload for `Tgetlock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9GetLockRequest {
    /// Fid whose advisory-lock state should be queried.
    pub fid: u32,
    /// Requested lock range.
    pub lock: P9Lock,
}

/// 9P2000.L filesystem stats returned by `Rstatfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9FsStat {
    /// Filesystem type magic.
    pub fs_type: u32,
    /// Filesystem block size.
    pub block_size: u32,
    /// Total data blocks.
    pub blocks: u64,
    /// Free data blocks.
    pub blocks_free: u64,
    /// Free blocks available to unprivileged users.
    pub blocks_available: u64,
    /// Total file nodes.
    pub files: u64,
    /// Free file nodes.
    pub files_free: u64,
    /// Filesystem id.
    pub fsid: u64,
    /// Maximum filename length.
    pub name_length: u32,
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

/// Decoded payload for `Tread`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Read {
    /// Fid to read from.
    pub fid: u32,
    /// Offset to read from.
    pub offset: u64,
    /// Maximum number of bytes requested.
    pub count: u32,
}

/// Decoded payload for `Treaddir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9ReadDir {
    /// Fid to read directory entries from.
    pub fid: u32,
    /// Opaque directory offset cookie supplied by a previous entry.
    pub offset: u64,
    /// Maximum number of directory-entry bytes requested.
    pub count: u32,
}

/// One 9P2000.L directory entry record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9DirEntry {
    /// QID for the listed child.
    pub qid: P9Qid,
    /// Opaque cookie for the next read position.
    pub offset: u64,
    /// Linux `DT_*` directory entry type.
    pub dirent_type: u8,
    /// Child basename.
    pub name: String,
}

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

/// Decoded payload for `Twrite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P9Write {
    /// Fid to write to.
    pub fid: u32,
    /// Offset to write at.
    pub offset: u64,
    /// Bytes to write.
    pub data: Vec<u8>,
}

/// Decoded payload for `Tclunk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Clunk {
    /// Fid to release.
    pub fid: u32,
}

/// Decoded payload for legacy `Tremove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P9Remove {
    /// Fid to remove and clunk.
    pub fid: u32,
}
