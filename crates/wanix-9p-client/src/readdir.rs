//! Bounded `Treaddir` paging into [`wanix_fs::DirEntry`] listings.
//!
//! A directory is read by opening its fid, then issuing `Treaddir` requests in a
//! cookie loop until the server returns an empty page. Both the total entry
//! count and the iteration count are bounded: a server that streams entries
//! forever, or never advances its cookie, cannot hang the importer.
//!
//! Listings are best-effort snapshots, not atomic: the server re-lists from its
//! own directory on each `Treaddir`, so concurrent mutation can shift entries
//! between pages. Without the Google.2 `Twalkgetattr` extension the per-entry
//! metadata is listing-only (file type from the directory-entry byte; size and
//! mode are placeholders), which is sufficient for a directory browse but not a
//! substitute for a per-file `metadata` call.

use wanix_fs::{DirEntry, FileType, Metadata};
use wanix_protocol::{
    DT_DIR, DT_LNK, P9_MODE_DIR, P9_MODE_LNK, P9_MODE_REG, P9DirEntry, RREADDIR_HEADER_LEN,
    p9_decode_rreaddir, p9_treaddir,
};

use crate::conn::P9Conn;
use crate::error::ClientResult;

/// Maximum directory entries a single `read_dir` will accumulate.
pub const MAX_ENTRIES: usize = 1_000_000;

/// Maximum `Treaddir` round trips a single `read_dir` will issue.
pub const MAX_ITERATIONS: usize = 100_000;

/// Pages every entry of the directory bound to `fid` into a listing.
///
/// `fid` must already be opened for reading. The loop advances by the cookie of
/// the last entry on each page and stops on an empty page, on reaching
/// [`MAX_ENTRIES`], or after [`MAX_ITERATIONS`] requests.
///
/// # Errors
///
/// Returns [`crate::error::ClientError`] when a `Treaddir` exchange fails.
pub fn read_all_entries(conn: &mut P9Conn, fid: u32) -> ClientResult<Vec<DirEntry>> {
    let count = conn.msize().saturating_sub(RREADDIR_HEADER_LEN).max(1);
    let mut entries = Vec::new();
    let mut cookie = 0_u64;

    for _ in 0..MAX_ITERATIONS {
        let reply = conn.rpc(|tag| Ok(p9_treaddir(tag, fid, cookie, count)))?;
        let page = p9_decode_rreaddir(&reply)?;
        if page.is_empty() {
            break;
        }
        let last_cookie = page.last().map_or(cookie, |entry| entry.offset);
        for raw in page {
            if let Some(entry) = listing_entry(&raw) {
                entries.push(entry);
            }
            if entries.len() >= MAX_ENTRIES {
                return Ok(entries);
            }
        }
        if last_cookie == cookie {
            // A server that fails to advance the cookie would loop forever;
            // stop rather than trust a stalled stream.
            break;
        }
        cookie = last_cookie;
    }
    Ok(entries)
}

/// Builds a listing-only [`DirEntry`] from one raw `Rreaddir` record.
///
/// The `.` and `..` synthetic entries are dropped so the listing matches the
/// Wanix `read_dir` contract. Size and mode are placeholders derived from the
/// directory-entry type byte; a caller needing real attributes must `metadata`
/// each child or rely on a Google.2 walkgetattr batch.
fn listing_entry(raw: &P9DirEntry) -> Option<DirEntry> {
    if raw.name == "." || raw.name == ".." {
        return None;
    }
    let (file_type, mode_type) = match raw.dirent_type {
        DT_DIR => (FileType::Directory, P9_MODE_DIR),
        DT_LNK => (FileType::Symlink, P9_MODE_LNK),
        _ => (FileType::File, P9_MODE_REG),
    };
    let metadata = Metadata::new(file_type, 0, mode_type);
    Some(DirEntry::new(raw.name.clone(), metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wanix_protocol::{DT_REG, P9Qid};

    fn raw(name: &str, dirent_type: u8) -> P9DirEntry {
        P9DirEntry {
            qid: P9Qid {
                qid_type: 0,
                version: 0,
                path: 0,
            },
            offset: 1,
            dirent_type,
            name: name.to_owned(),
        }
    }

    #[test]
    fn dot_entries_are_dropped() {
        assert!(listing_entry(&raw(".", DT_DIR)).is_none());
        assert!(listing_entry(&raw("..", DT_DIR)).is_none());
    }

    #[test]
    fn dirent_type_maps_to_file_type() {
        let dir = listing_entry(&raw("sub", DT_DIR)).unwrap();
        assert_eq!(dir.metadata().file_type(), FileType::Directory);
        let link = listing_entry(&raw("ln", DT_LNK)).unwrap();
        assert_eq!(link.metadata().file_type(), FileType::Symlink);
        let file = listing_entry(&raw("f", DT_REG)).unwrap();
        assert_eq!(file.metadata().file_type(), FileType::File);
    }
}
