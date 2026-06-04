/// 9P2000.L version string used by the existing v86 integration.
pub const P9_VERSION_9P2000_L: &str = "9P2000.L";

/// 9P2000.L Google extension version that adds `Tflushf`.
pub const P9_VERSION_9P2000_L_GOOGLE_1: &str = "9P2000.L.Google.1";

/// 9P2000.L Google extension version that adds `Twalkgetattr`.
pub const P9_VERSION_9P2000_L_GOOGLE_2: &str = "9P2000.L.Google.2";

/// The 9P `NOTAG` value used by version negotiation.
pub const P9_NOTAG: u16 = 0xffff;

/// The 9P `NOFID` value used when no auth fid is supplied.
pub const P9_NOFID: u32 = 0xffff_ffff;

/// Minimum 9P frame size: `size[4] type[1] tag[2]`.
pub const P9_HEADER_LEN: usize = 7;

/// 9P `Rlerror` message type.
pub const P9_RLERROR: u8 = 7;

/// 9P2000.L `Tstatfs` message type.
pub const P9_TSTATFS: u8 = 8;

/// 9P2000.L `Rstatfs` message type.
pub const P9_RSTATFS: u8 = 9;

/// 9P2000.L `Tlopen` message type.
pub const P9_TLOPEN: u8 = 12;

/// 9P2000.L `Rlopen` message type.
pub const P9_RLOPEN: u8 = 13;

/// 9P2000.L `Tlcreate` message type.
pub const P9_TLCREATE: u8 = 14;

/// 9P2000.L `Rlcreate` message type.
pub const P9_RLCREATE: u8 = 15;

/// 9P2000.L `Tsymlink` message type.
pub const P9_TSYMLINK: u8 = 16;

/// 9P2000.L `Rsymlink` message type.
pub const P9_RSYMLINK: u8 = 17;

/// 9P2000.L `Tmknod` message type.
pub const P9_TMKNOD: u8 = 18;

/// 9P2000.L `Rmknod` message type.
pub const P9_RMKNOD: u8 = 19;

/// 9P2000.L `Trename` message type.
pub const P9_TRENAME: u8 = 20;

/// 9P2000.L `Rrename` message type.
pub const P9_RRENAME: u8 = 21;

/// 9P2000.L `Treadlink` message type.
pub const P9_TREADLINK: u8 = 22;

/// 9P2000.L `Rreadlink` message type.
pub const P9_RREADLINK: u8 = 23;

/// 9P2000.L `Tgetattr` message type.
pub const P9_TGETATTR: u8 = 24;

/// 9P2000.L `Rgetattr` message type.
pub const P9_RGETATTR: u8 = 25;

/// 9P2000.L `Tsetattr` message type.
pub const P9_TSETATTR: u8 = 26;

/// 9P2000.L `Rsetattr` message type.
pub const P9_RSETATTR: u8 = 27;

/// 9P2000.L `Txattrwalk` message type.
pub const P9_TXATTRWALK: u8 = 30;

/// 9P2000.L `Rxattrwalk` message type.
pub const P9_RXATTRWALK: u8 = 31;

/// 9P2000.L `Txattrcreate` message type.
pub const P9_TXATTRCREATE: u8 = 32;

/// 9P2000.L `Rxattrcreate` message type.
pub const P9_RXATTRCREATE: u8 = 33;

/// 9P2000.L `Treaddir` message type.
pub const P9_TREADDIR: u8 = 40;

/// 9P2000.L `Rreaddir` message type.
pub const P9_RREADDIR: u8 = 41;

/// 9P2000.L `Tfsync` message type.
pub const P9_TFSYNC: u8 = 50;

/// 9P2000.L `Rfsync` message type.
pub const P9_RFSYNC: u8 = 51;

/// 9P2000.L `Tlock` message type.
pub const P9_TLOCK: u8 = 52;

/// 9P2000.L `Rlock` message type.
pub const P9_RLOCK: u8 = 53;

/// 9P2000.L `Tgetlock` message type.
pub const P9_TGETLOCK: u8 = 54;

/// 9P2000.L `Rgetlock` message type.
pub const P9_RGETLOCK: u8 = 55;

/// 9P2000.L `Tlink` message type.
pub const P9_TLINK: u8 = 70;

/// 9P2000.L `Rlink` message type.
pub const P9_RLINK: u8 = 71;

/// 9P2000.L `Tmkdir` message type.
pub const P9_TMKDIR: u8 = 72;

/// 9P2000.L `Rmkdir` message type.
pub const P9_RMKDIR: u8 = 73;

/// 9P2000.L `Trenameat` message type.
pub const P9_TRENAMEAT: u8 = 74;

/// 9P2000.L `Rrenameat` message type.
pub const P9_RRENAMEAT: u8 = 75;

/// 9P2000.L `Tunlinkat` message type.
pub const P9_TUNLINKAT: u8 = 76;

/// 9P2000.L `Runlinkat` message type.
pub const P9_RUNLINKAT: u8 = 77;

/// 9P `Tversion` message type.
pub const P9_TVERSION: u8 = 100;

/// 9P `Rversion` message type.
pub const P9_RVERSION: u8 = 101;

/// 9P `Tauth` message type.
pub const P9_TAUTH: u8 = 102;

/// 9P `Rauth` message type.
pub const P9_RAUTH: u8 = 103;

/// 9P `Tattach` message type.
pub const P9_TATTACH: u8 = 104;

/// 9P `Rattach` message type.
pub const P9_RATTACH: u8 = 105;

/// 9P `Tflush` message type.
pub const P9_TFLUSH: u8 = 108;

/// 9P `Rflush` message type.
pub const P9_RFLUSH: u8 = 109;

/// 9P `Twalk` message type.
pub const P9_TWALK: u8 = 110;

/// 9P `Rwalk` message type.
pub const P9_RWALK: u8 = 111;

/// 9P `Tread` message type.
pub const P9_TREAD: u8 = 116;

/// 9P `Rread` message type.
pub const P9_RREAD: u8 = 117;

/// 9P `Twrite` message type.
pub const P9_TWRITE: u8 = 118;

/// 9P `Rwrite` message type.
pub const P9_RWRITE: u8 = 119;

/// 9P `Tclunk` message type.
pub const P9_TCLUNK: u8 = 120;

/// 9P `Rclunk` message type.
pub const P9_RCLUNK: u8 = 121;

/// 9P `Tremove` message type.
pub const P9_TREMOVE: u8 = 122;

/// 9P `Rremove` message type.
pub const P9_RREMOVE: u8 = 123;

/// 9P2000.L.Google.1 `Tflushf` message type.
pub const P9_TFLUSHF: u8 = 124;

/// 9P2000.L.Google.1 `Rflushf` message type.
pub const P9_RFLUSHF: u8 = 125;

/// 9P2000.L.Google.2 `Twalkgetattr` message type.
pub const P9_TWALKGETATTR: u8 = 126;

/// 9P2000.L.Google.2 `Rwalkgetattr` message type.
pub const P9_RWALKGETATTR: u8 = 127;

/// 9P2000.L `Tsetattr` permissions-valid bit.
pub const P9_SETATTR_PERMISSIONS: u32 = 0x0000_0001;

/// 9P2000.L `Tsetattr` uid-valid bit.
pub const P9_SETATTR_UID: u32 = 0x0000_0002;

/// 9P2000.L `Tsetattr` gid-valid bit.
pub const P9_SETATTR_GID: u32 = 0x0000_0004;

/// 9P2000.L `Tsetattr` size-valid bit.
pub const P9_SETATTR_SIZE: u32 = 0x0000_0008;

/// 9P2000.L `Tsetattr` access-time-valid bit.
pub const P9_SETATTR_ATIME: u32 = 0x0000_0010;

/// 9P2000.L `Tsetattr` modification-time-valid bit.
pub const P9_SETATTR_MTIME: u32 = 0x0000_0020;

/// 9P2000.L `Tsetattr` metadata-change-time-valid bit.
pub const P9_SETATTR_CTIME: u32 = 0x0000_0040;

/// 9P2000.L `Tsetattr` access time is explicit rather than server current time.
pub const P9_SETATTR_ATIME_NOT_SYSTEM_TIME: u32 = 0x0000_0080;

/// 9P2000.L `Tsetattr` modification time is explicit rather than server current time.
pub const P9_SETATTR_MTIME_NOT_SYSTEM_TIME: u32 = 0x0000_0100;

/// 9P2000.L read-lock type.
pub const P9_LOCK_TYPE_READ: u8 = 0;

/// 9P2000.L write-lock type.
pub const P9_LOCK_TYPE_WRITE: u8 = 1;

/// 9P2000.L unlock/no-conflict type.
pub const P9_LOCK_TYPE_UNLOCK: u8 = 2;

/// 9P2000.L lock request succeeded.
pub const P9_LOCK_STATUS_OK: u8 = 0;

/// 9P2000.L lock request blocked.
pub const P9_LOCK_STATUS_BLOCKED: u8 = 1;

/// 9P2000.L lock request failed.
pub const P9_LOCK_STATUS_ERROR: u8 = 2;

/// 9P2000.L lock request is in grace period.
pub const P9_LOCK_STATUS_GRACE: u8 = 3;

type MessageTypeName = (u8, &'static str);

const MESSAGE_TYPE_NAMES: &[MessageTypeName] = &[
    (P9_RLERROR, "Rlerror"),
    (P9_TSTATFS, "Tstatfs"),
    (P9_RSTATFS, "Rstatfs"),
    (P9_TLOPEN, "Tlopen"),
    (P9_RLOPEN, "Rlopen"),
    (P9_TLCREATE, "Tlcreate"),
    (P9_RLCREATE, "Rlcreate"),
    (P9_TSYMLINK, "Tsymlink"),
    (P9_RSYMLINK, "Rsymlink"),
    (P9_TMKNOD, "Tmknod"),
    (P9_RMKNOD, "Rmknod"),
    (P9_TRENAME, "Trename"),
    (P9_RRENAME, "Rrename"),
    (P9_TREADLINK, "Treadlink"),
    (P9_RREADLINK, "Rreadlink"),
    (P9_TGETATTR, "Tgetattr"),
    (P9_RGETATTR, "Rgetattr"),
    (P9_TSETATTR, "Tsetattr"),
    (P9_RSETATTR, "Rsetattr"),
    (P9_TXATTRWALK, "Txattrwalk"),
    (P9_RXATTRWALK, "Rxattrwalk"),
    (P9_TXATTRCREATE, "Txattrcreate"),
    (P9_RXATTRCREATE, "Rxattrcreate"),
    (P9_TREADDIR, "Treaddir"),
    (P9_RREADDIR, "Rreaddir"),
    (P9_TFSYNC, "Tfsync"),
    (P9_RFSYNC, "Rfsync"),
    (P9_TLOCK, "Tlock"),
    (P9_RLOCK, "Rlock"),
    (P9_TGETLOCK, "Tgetlock"),
    (P9_RGETLOCK, "Rgetlock"),
    (P9_TLINK, "Tlink"),
    (P9_RLINK, "Rlink"),
    (P9_TMKDIR, "Tmkdir"),
    (P9_RMKDIR, "Rmkdir"),
    (P9_TRENAMEAT, "Trenameat"),
    (P9_RRENAMEAT, "Rrenameat"),
    (P9_TUNLINKAT, "Tunlinkat"),
    (P9_RUNLINKAT, "Runlinkat"),
    (P9_TVERSION, "Tversion"),
    (P9_RVERSION, "Rversion"),
    (P9_TAUTH, "Tauth"),
    (P9_RAUTH, "Rauth"),
    (P9_TATTACH, "Tattach"),
    (P9_RATTACH, "Rattach"),
    (P9_TFLUSH, "Tflush"),
    (P9_RFLUSH, "Rflush"),
    (P9_TWALK, "Twalk"),
    (P9_RWALK, "Rwalk"),
    (P9_TREAD, "Tread"),
    (P9_RREAD, "Rread"),
    (P9_TWRITE, "Twrite"),
    (P9_RWRITE, "Rwrite"),
    (P9_TCLUNK, "Tclunk"),
    (P9_RCLUNK, "Rclunk"),
    (P9_TREMOVE, "Tremove"),
    (P9_RREMOVE, "Rremove"),
    (P9_TFLUSHF, "Tflushf"),
    (P9_RFLUSHF, "Rflushf"),
    (P9_TWALKGETATTR, "Twalkgetattr"),
    (P9_RWALKGETATTR, "Rwalkgetattr"),
];

/// Returns a stable name for message types the Rust port currently identifies.
#[must_use]
pub const fn p9_message_type_name(message_type: u8) -> Option<&'static str> {
    let mut index = 0;
    while index < MESSAGE_TYPE_NAMES.len() {
        let (known_type, name) = MESSAGE_TYPE_NAMES[index];
        if known_type == message_type {
            return Some(name);
        }
        index += 1;
    }
    None
}
