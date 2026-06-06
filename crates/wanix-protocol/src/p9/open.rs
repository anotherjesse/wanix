//! 9P2000.L open-flag constants and pure-numeric flag helpers.
//!
//! These constants and helpers are wire-only: they describe the POSIX-style
//! open flags carried by `Tlopen`/`Tlcreate` and convert between those flags
//! and a small set of access booleans. They stay free of Wanix filesystem
//! types so both the 9P server and a `wanix-fs`-aware 9P client can share them
//! without `wanix-protocol` gaining a `wanix-fs` dependency.

/// Mask selecting the access-mode bits of a 9P2000.L open-flags word.
pub const O_ACCMODE: u32 = 0o3;
/// Read-only access mode.
pub const O_RDONLY: u32 = 0o0;
/// Write-only access mode.
pub const O_WRONLY: u32 = 0o1;
/// Read-write access mode.
pub const O_RDWR: u32 = 0o2;
/// Create the file if it does not already exist.
pub const O_CREAT: u32 = 0o100;
/// Truncate the file to zero length when opening.
pub const O_TRUNC: u32 = 0o1000;
/// Append all writes to the current end of the file.
pub const O_APPEND: u32 = 0o2000;

/// Access intent decoded from a 9P2000.L open-flags word.
///
/// This mirrors the boolean shape of a filesystem open request without naming
/// any `wanix-fs` type, so it can be lowered into `OpenOptions` by callers that
/// depend on `wanix-fs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct P9OpenIntent {
    /// Whether the handle should permit reads.
    pub read: bool,
    /// Whether the handle should permit writes.
    pub write: bool,
    /// Whether a missing file should be created.
    pub create: bool,
    /// Whether the file should be truncated on open.
    pub truncate: bool,
    /// Whether writes should be forced to the end of the file.
    pub append: bool,
}

/// Decodes 9P2000.L open `flags` into a [`P9OpenIntent`].
///
/// `O_WRONLY` implies write-only; every other access mode keeps read enabled,
/// matching the server's existing open semantics.
#[must_use]
pub fn p9_open_intent(flags: u32) -> P9OpenIntent {
    let access = flags & O_ACCMODE;
    P9OpenIntent {
        read: access != O_WRONLY,
        write: access == O_WRONLY || access == O_RDWR,
        create: flags & O_CREAT != 0,
        truncate: flags & O_TRUNC != 0,
        append: flags & O_APPEND != 0,
    }
}

/// Encodes a [`P9OpenIntent`] into a 9P2000.L open-flags word.
///
/// This is the inverse of [`p9_open_intent`] and is used by 9P clients that
/// need to build `Tlopen`/`Tlcreate` flags from a desired access intent. The
/// access mode is `O_RDWR` when both read and write are requested, `O_WRONLY`
/// for write-only, and `O_RDONLY` otherwise.
#[must_use]
pub fn p9_open_flags(intent: P9OpenIntent) -> u32 {
    let mut flags = if intent.write {
        if intent.read { O_RDWR } else { O_WRONLY }
    } else {
        O_RDONLY
    };
    if intent.create {
        flags |= O_CREAT;
    }
    if intent.truncate {
        flags |= O_TRUNC;
    }
    if intent.append {
        flags |= O_APPEND;
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_modes_round_trip_through_intent() {
        assert_eq!(
            p9_open_intent(O_RDONLY),
            P9OpenIntent {
                read: true,
                ..P9OpenIntent::default()
            }
        );
        assert_eq!(
            p9_open_intent(O_WRONLY),
            P9OpenIntent {
                read: false,
                write: true,
                ..P9OpenIntent::default()
            }
        );
        assert_eq!(
            p9_open_intent(O_RDWR),
            P9OpenIntent {
                read: true,
                write: true,
                ..P9OpenIntent::default()
            }
        );
    }

    #[test]
    fn flag_bits_decode_into_intent() {
        let intent = p9_open_intent(O_WRONLY | O_CREAT | O_TRUNC | O_APPEND);
        assert!(!intent.read);
        assert!(intent.write);
        assert!(intent.create);
        assert!(intent.truncate);
        assert!(intent.append);
    }

    #[test]
    fn encode_is_inverse_of_decode_for_access_modes() {
        for flags in [O_RDONLY, O_WRONLY, O_RDWR] {
            assert_eq!(p9_open_flags(p9_open_intent(flags)) & O_ACCMODE, flags);
        }
    }

    #[test]
    fn encode_round_trips_full_flag_set() {
        let flags = O_RDWR | O_CREAT | O_TRUNC | O_APPEND;
        assert_eq!(p9_open_flags(p9_open_intent(flags)), flags);
    }
}
