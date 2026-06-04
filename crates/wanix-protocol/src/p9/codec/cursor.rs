use super::super::{P9Attr, P9AttrBody, P9DirEntry, P9Error, P9FsStat, P9Lock, P9Qid, P9SetAttr};

pub(in crate::p9) struct PayloadCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PayloadCursor<'a> {
    pub(in crate::p9) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(in crate::p9) fn read_u32(&mut self) -> Result<u32, P9Error> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .expect("read_exact returned the requested byte count"),
        ))
    }

    pub(in crate::p9) fn read_u16(&mut self) -> Result<u16, P9Error> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes(
            bytes
                .try_into()
                .expect("read_exact returned the requested byte count"),
        ))
    }

    pub(in crate::p9) fn read_u8(&mut self) -> Result<u8, P9Error> {
        Ok(self.read_exact(1)?[0])
    }

    pub(in crate::p9) fn read_u64(&mut self) -> Result<u64, P9Error> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes(
            bytes
                .try_into()
                .expect("read_exact returned the requested byte count"),
        ))
    }

    pub(in crate::p9) fn read_qid(&mut self) -> Result<P9Qid, P9Error> {
        let qid_type = self.read_exact(1)?[0];
        let version = self.read_u32()?;
        let path = self.read_u64()?;
        Ok(P9Qid {
            qid_type,
            version,
            path,
        })
    }

    pub(in crate::p9) fn read_fs_stat(&mut self) -> Result<P9FsStat, P9Error> {
        let fs_type = self.read_u32()?;
        let block_size = self.read_u32()?;
        let blocks = self.read_u64()?;
        let blocks_free = self.read_u64()?;
        let blocks_available = self.read_u64()?;
        let files = self.read_u64()?;
        let files_free = self.read_u64()?;
        let fsid = self.read_u64()?;
        let name_length = self.read_u32()?;
        Ok(P9FsStat {
            fs_type,
            block_size,
            blocks,
            blocks_free,
            blocks_available,
            files,
            files_free,
            fsid,
            name_length,
        })
    }

    pub(in crate::p9) fn read_dir_entry(&mut self) -> Result<P9DirEntry, P9Error> {
        let qid = self.read_qid()?;
        let offset = self.read_u64()?;
        let dirent_type = self.read_u8()?;
        let name = self.read_string()?;
        Ok(P9DirEntry {
            qid,
            offset,
            dirent_type,
            name,
        })
    }

    pub(in crate::p9) fn read_attr(&mut self) -> Result<P9Attr, P9Error> {
        let valid = self.read_u64()?;
        let qid = self.read_qid()?;
        let body = self.read_attr_body()?;
        Ok(P9Attr {
            valid,
            qid,
            mode: body.mode,
            uid: body.uid,
            gid: body.gid,
            nlink: body.nlink,
            rdev: body.rdev,
            size: body.size,
            block_size: body.block_size,
            blocks: body.blocks,
            atime_seconds: body.atime_seconds,
            atime_nanoseconds: body.atime_nanoseconds,
            mtime_seconds: body.mtime_seconds,
            mtime_nanoseconds: body.mtime_nanoseconds,
            ctime_seconds: body.ctime_seconds,
            ctime_nanoseconds: body.ctime_nanoseconds,
            btime_seconds: body.btime_seconds,
            btime_nanoseconds: body.btime_nanoseconds,
            generation: body.generation,
            data_version: body.data_version,
        })
    }

    pub(in crate::p9) fn read_attr_body(&mut self) -> Result<P9AttrBody, P9Error> {
        let mode = self.read_u32()?;
        let uid = self.read_u32()?;
        let gid = self.read_u32()?;
        let nlink = self.read_u64()?;
        let rdev = self.read_u64()?;
        let size = self.read_u64()?;
        let block_size = self.read_u64()?;
        let blocks = self.read_u64()?;
        let atime_seconds = self.read_u64()?;
        let atime_nanoseconds = self.read_u64()?;
        let mtime_seconds = self.read_u64()?;
        let mtime_nanoseconds = self.read_u64()?;
        let ctime_seconds = self.read_u64()?;
        let ctime_nanoseconds = self.read_u64()?;
        let btime_seconds = self.read_u64()?;
        let btime_nanoseconds = self.read_u64()?;
        let generation = self.read_u64()?;
        let data_version = self.read_u64()?;
        Ok(P9AttrBody {
            mode,
            uid,
            gid,
            nlink,
            rdev,
            size,
            block_size,
            blocks,
            atime_seconds,
            atime_nanoseconds,
            mtime_seconds,
            mtime_nanoseconds,
            ctime_seconds,
            ctime_nanoseconds,
            btime_seconds,
            btime_nanoseconds,
            generation,
            data_version,
        })
    }

    pub(in crate::p9) fn read_set_attr(&mut self) -> Result<P9SetAttr, P9Error> {
        let permissions = self.read_u32()?;
        let uid = self.read_u32()?;
        let gid = self.read_u32()?;
        let size = self.read_u64()?;
        let atime_seconds = self.read_u64()?;
        let atime_nanoseconds = self.read_u64()?;
        let mtime_seconds = self.read_u64()?;
        let mtime_nanoseconds = self.read_u64()?;
        Ok(P9SetAttr {
            permissions,
            uid,
            gid,
            size,
            atime_seconds,
            atime_nanoseconds,
            mtime_seconds,
            mtime_nanoseconds,
        })
    }

    pub(in crate::p9) fn read_lock(&mut self, lock_type: u8) -> Result<P9Lock, P9Error> {
        let start = self.read_u64()?;
        let length = self.read_u64()?;
        let proc_id = self.read_u32()?;
        let client_id = self.read_string()?;
        Ok(P9Lock {
            lock_type,
            start,
            length,
            proc_id,
            client_id,
        })
    }

    pub(in crate::p9) fn read_string(&mut self) -> Result<String, P9Error> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| P9Error::InvalidUtf8)
    }

    pub(in crate::p9) fn read_counted_data(&mut self) -> Result<Vec<u8>, P9Error> {
        let len = self.read_u32()? as usize;
        Ok(self.read_exact(len)?.to_vec())
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], P9Error> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if remaining < len {
            return Err(P9Error::UnexpectedEof {
                needed: len,
                remaining,
            });
        }
        let start = self.offset;
        self.offset += len;
        Ok(&self.bytes[start..start + len])
    }

    pub(in crate::p9) fn finish(self) -> Result<(), P9Error> {
        let count = self.bytes.len().saturating_sub(self.offset);
        if count == 0 {
            Ok(())
        } else {
            Err(P9Error::TrailingPayload { count })
        }
    }

    pub(in crate::p9) fn remaining_len(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}
