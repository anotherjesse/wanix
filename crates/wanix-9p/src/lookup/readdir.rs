use wanix_fs::{DirEntry, NormalizedPath};
use wanix_protocol::{
    P9DirEntry, P9Frame, p9_decode_treaddir, p9_dir_entry_encoded_len, p9_rlerror, p9_rreaddir,
};

use crate::attrs::{dirent_type_for_metadata, qid_for_metadata};
use crate::path::join_walk_component;
use crate::{P9Server, RREADDIR_HEADER_LEN, Wanix9pError, errno_for_fs};

struct ReaddirEntries {
    base_path: NormalizedPath,
    offset: u64,
    max_data_len: u32,
    data_len: u32,
    cursor: u64,
    entries: Vec<P9DirEntry>,
}

impl ReaddirEntries {
    fn new(base_path: NormalizedPath, offset: u64, max_data_len: u32) -> Self {
        Self {
            base_path,
            offset,
            max_data_len,
            data_len: 0,
            cursor: 0,
            entries: Vec::new(),
        }
    }

    fn push(&mut self, entry: DirEntry) -> Result<bool, Wanix9pError> {
        self.cursor += 1;
        if self.cursor <= self.offset {
            return Ok(true);
        }
        let p9_entry = self.p9_entry(entry)?;
        let Some(next_len) = self.next_data_len(&p9_entry)? else {
            return Ok(false);
        };
        self.data_len = next_len;
        self.entries.push(p9_entry);
        Ok(true)
    }

    fn p9_entry(&self, entry: DirEntry) -> Result<P9DirEntry, Wanix9pError> {
        let child_path = join_walk_component(&self.base_path, entry.name())?;
        Ok(P9DirEntry {
            qid: qid_for_metadata(&child_path, entry.metadata().clone()),
            offset: self.cursor,
            dirent_type: dirent_type_for_metadata(entry.metadata()),
            name: entry.name().to_owned(),
        })
    }

    fn next_data_len(&self, entry: &P9DirEntry) -> Result<Option<u32>, Wanix9pError> {
        let entry_len = p9_dir_entry_encoded_len(entry)?;
        let Ok(entry_len) = u32::try_from(entry_len) else {
            return Ok(None);
        };
        let Some(next_len) = self.data_len.checked_add(entry_len) else {
            return Ok(None);
        };
        Ok((next_len <= self.max_data_len).then_some(next_len))
    }

    fn finish(self) -> Vec<P9DirEntry> {
        self.entries
    }
}

impl P9Server {
    pub(crate) fn handle_readdir(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let read = p9_decode_treaddir(frame)?;
        let path = match self.fid_path_or_reply(frame.tag(), read.fid) {
            Ok(path) => path,
            Err(response) => return Ok(response),
        };
        self.readdir_response(frame.tag(), path, read.offset, read.count)
    }

    fn readdir_response(
        &self,
        tag: u16,
        path: NormalizedPath,
        offset: u64,
        count: u32,
    ) -> Result<P9Frame, Wanix9pError> {
        let entries = match self.root.read_dir(&path) {
            Ok(entries) => entries,
            Err(error) => return Ok(p9_rlerror(tag, errno_for_fs(&error))),
        };
        let max_data_len = count.min(self.msize.saturating_sub(RREADDIR_HEADER_LEN));
        let p9_entries = self.collect_readdir_entries(path, offset, max_data_len, entries)?;
        Ok(p9_rreaddir(tag, &p9_entries)?)
    }

    fn collect_readdir_entries(
        &self,
        path: NormalizedPath,
        offset: u64,
        max_data_len: u32,
        entries: Vec<DirEntry>,
    ) -> Result<Vec<P9DirEntry>, Wanix9pError> {
        let mut p9_entries = ReaddirEntries::new(path, offset, max_data_len);
        for entry in entries {
            if !p9_entries.push(entry)? {
                break;
            };
        }
        Ok(p9_entries.finish())
    }
}
