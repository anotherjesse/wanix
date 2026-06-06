use wanix_fs::{FileType, Metadata, MetadataTimes};
use wanix_protocol::{P9_MODE_DIR, P9_MODE_LNK, P9_MODE_TYPE_MASK, P9Attr};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Inverts a server `Rgetattr` [`P9Attr`] into `wanix-fs` [`Metadata`].
///
/// The file type is taken from the POSIX file-type bits of `attr.mode`, the
/// byte length and link count are copied directly, and the access, modification,
/// and change timestamps are recombined from their seconds and nanoseconds
/// fields into nanoseconds since the Unix epoch. This is the client-side inverse
/// of the server's metadata-to-attribute mapping.
#[must_use]
pub fn metadata_from_attr(attr: &P9Attr) -> Metadata {
    let file_type = file_type_for_mode(attr.mode);
    let times = MetadataTimes::new(
        join_time_ns(attr.atime_seconds, attr.atime_nanoseconds),
        join_time_ns(attr.mtime_seconds, attr.mtime_nanoseconds),
        join_time_ns(attr.ctime_seconds, attr.ctime_nanoseconds),
    );
    Metadata::new_with_links(file_type, attr.size, attr.mode, attr.nlink, times)
}

fn file_type_for_mode(mode: u32) -> FileType {
    match mode & P9_MODE_TYPE_MASK {
        P9_MODE_DIR => FileType::Directory,
        P9_MODE_LNK => FileType::Symlink,
        _ => FileType::File,
    }
}

fn join_time_ns(seconds: u64, nanoseconds: u64) -> u64 {
    seconds
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(nanoseconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wanix_protocol::{P9_MODE_REG, P9_QID_TYPE_DIR, P9Qid};

    fn attr_with(mode: u32, qid_type: u8) -> P9Attr {
        P9Attr {
            valid: u64::MAX,
            qid: P9Qid {
                qid_type,
                version: 0,
                path: 0,
            },
            mode,
            uid: 0,
            gid: 0,
            nlink: 2,
            rdev: 0,
            size: 17,
            block_size: 65_536,
            blocks: 1,
            atime_seconds: 1_234,
            atime_nanoseconds: 5,
            mtime_seconds: 2_345,
            mtime_nanoseconds: 6,
            ctime_seconds: 3_456,
            ctime_nanoseconds: 7,
            btime_seconds: 0,
            btime_nanoseconds: 0,
            generation: 0,
            data_version: 0,
        }
    }

    #[test]
    fn directory_attr_inverts_to_directory_metadata() {
        let metadata = metadata_from_attr(&attr_with(P9_MODE_DIR | 0o755, P9_QID_TYPE_DIR));
        assert_eq!(metadata.file_type(), FileType::Directory);
        assert_eq!(metadata.len(), 17);
        assert_eq!(metadata.mode(), P9_MODE_DIR | 0o755);
        assert_eq!(metadata.link_count(), 2);
    }

    #[test]
    fn regular_file_timestamps_recombine_to_nanoseconds() {
        let metadata = metadata_from_attr(&attr_with(P9_MODE_REG | 0o644, 0));
        assert_eq!(metadata.file_type(), FileType::File);
        assert_eq!(metadata.accessed_time_ns(), 1_234_000_000_005);
        assert_eq!(metadata.modified_time_ns(), 2_345_000_000_006);
        assert_eq!(metadata.changed_time_ns(), 3_456_000_000_007);
    }

    #[test]
    fn symlink_mode_bits_invert_to_symlink_type() {
        let metadata = metadata_from_attr(&attr_with(P9_MODE_LNK | 0o777, 0x02));
        assert_eq!(metadata.file_type(), FileType::Symlink);
    }
}
