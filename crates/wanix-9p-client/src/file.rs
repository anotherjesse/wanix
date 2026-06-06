//! [`RemoteFile`]: an open 9P fid exposed through the [`wanix_fs::File`] trait.
//!
//! A `RemoteFile` owns an opened fid, the server-advertised `iounit`, and a
//! seekability decision made at open time. It clunks its fid on [`Drop`].
//!
//! Two corrections are baked in here. **Honest seekability**: only files the
//! server reported as regular at open carry a client-tracked offset and accept
//! [`seek`]; a device or service stream (where the server ignores `Tread.offset`
//! and reads sequentially) reports `is_seekable() == false` and refuses `seek`
//! rather than inventing a fictional offset. **Append via server**: when opened
//! for append, the fid carries `O_APPEND` so the server seeks to end before each
//! write; the client never races a `Tgetattr`-per-write to compute the offset.
//!
//! [`seek`]: wanix_fs::File::seek

use std::sync::{Arc, Mutex};

use wanix_fs::{FileSeekFrom, FsError, FsResult, Metadata};
use wanix_protocol::{
    RREAD_HEADER_LEN, RWRITE_HEADER_LEN, p9_decode_rgetattr, p9_decode_rread, p9_decode_rwrite,
    p9_tgetattr, p9_tread, p9_twrite,
};

use crate::attr::metadata_from_attr;
use crate::conn::P9Conn;
use crate::error::ClientResult;
use crate::walk::lock;

/// `Tgetattr` mask requesting every attribute the server can supply.
const GETATTR_ALL: u64 = u64::MAX;

/// An open handle to a remote 9P file backed by one opened fid.
pub struct RemoteFile {
    conn: Arc<Mutex<P9Conn>>,
    fid: u32,
    iounit: u32,
    seekable: bool,
    append: bool,
    offset: u64,
}

impl RemoteFile {
    /// Builds a remote file handle from an opened fid and open-time facts.
    ///
    /// `iounit` is the server's advertised transfer unit (zero means "use the
    /// negotiated `msize`"). `seekable` records whether the server treats the
    /// file as a regular, offset-addressable file. `append` records that the fid
    /// was opened `O_APPEND` so writes append server-side.
    #[must_use]
    pub fn new(
        conn: Arc<Mutex<P9Conn>>,
        fid: u32,
        iounit: u32,
        seekable: bool,
        append: bool,
    ) -> Self {
        Self {
            conn,
            fid,
            iounit,
            seekable,
            append,
            offset: 0,
        }
    }

    /// Returns the per-request read transfer size honoring `iounit` and `msize`.
    fn read_chunk(&self, conn: &P9Conn) -> u32 {
        let ceiling = conn.msize().saturating_sub(RREAD_HEADER_LEN);
        clamp_chunk(self.iounit, ceiling)
    }

    /// Returns the per-request write transfer size honoring `iounit` and `msize`.
    fn write_chunk(&self, conn: &P9Conn) -> u32 {
        let ceiling = conn.msize().saturating_sub(RWRITE_HEADER_LEN);
        clamp_chunk(self.iounit, ceiling)
    }

    fn read_into(&mut self, buf: &mut [u8]) -> ClientResult<usize> {
        let fid = self.fid;
        let offset = self.offset;
        let mut held = lock(&self.conn)?;
        let chunk = self.read_chunk(&held) as usize;
        let count = chunk.min(buf.len()) as u32;
        let reply = held.rpc(|tag| Ok(p9_tread(tag, fid, offset, count)))?;
        let data = p9_decode_rread(&reply)?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        if self.seekable {
            self.offset = self.offset.saturating_add(n as u64);
        }
        Ok(n)
    }

    fn write_from(&mut self, buf: &[u8]) -> ClientResult<usize> {
        let fid = self.fid;
        // Append fids let the server seek to end; the offset field is ignored
        // server-side, so the client sends a stable zero rather than racing a
        // size probe. Non-append seekable files advance the tracked offset.
        let offset = if self.append { 0 } else { self.offset };
        let mut held = lock(&self.conn)?;
        let chunk = self.write_chunk(&held) as usize;
        let slice = &buf[..buf.len().min(chunk)];
        let reply = held.rpc(|tag| p9_twrite(tag, fid, offset, slice))?;
        let written = p9_decode_rwrite(&reply)? as usize;
        if self.seekable && !self.append {
            self.offset = self.offset.saturating_add(written as u64);
        }
        Ok(written)
    }

    fn metadata_via_getattr(&self) -> ClientResult<Metadata> {
        let fid = self.fid;
        let mut held = lock(&self.conn)?;
        let reply = held.rpc(|tag| Ok(p9_tgetattr(tag, fid, GETATTR_ALL)))?;
        let attr = p9_decode_rgetattr(&reply)?;
        Ok(metadata_from_attr(&attr))
    }
}

/// Chooses the smaller of a non-zero `iounit` and the `msize`-derived ceiling.
fn clamp_chunk(iounit: u32, ceiling: u32) -> u32 {
    let ceiling = ceiling.max(1);
    if iounit == 0 {
        ceiling
    } else {
        iounit.min(ceiling)
    }
}

impl wanix_fs::File for RemoteFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        Ok(self.read_into(buf)?)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        Ok(self.write_from(buf)?)
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        if !self.seekable {
            return Err(FsError::NotSupported);
        }
        let next = match from {
            FileSeekFrom::Start(offset) => Some(offset),
            FileSeekFrom::Current(delta) => add_signed(self.offset, delta),
            FileSeekFrom::End(delta) => {
                let len = self.metadata_via_getattr()?.len();
                add_signed(len, delta)
            }
        };
        let next = next.ok_or(FsError::InvalidOffset)?;
        self.offset = next;
        Ok(next)
    }

    fn tell(&self) -> FsResult<u64> {
        if self.seekable {
            Ok(self.offset)
        } else {
            Err(FsError::NotSupported)
        }
    }

    fn is_seekable(&self) -> bool {
        self.seekable
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(self.metadata_via_getattr()?)
    }
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        if let Ok(mut conn) = self.conn.lock() {
            conn.clunk_fid(self.fid);
        }
    }
}

/// Applies a signed delta to an unsigned offset, returning `None` on overflow.
fn add_signed(base: u64, delta: i64) -> Option<u64> {
    if delta >= 0 {
        base.checked_add(delta as u64)
    } else {
        base.checked_sub(delta.unsigned_abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_clamps_to_ceiling_when_iounit_zero() {
        assert_eq!(clamp_chunk(0, 4096), 4096);
    }

    #[test]
    fn chunk_prefers_smaller_iounit() {
        assert_eq!(clamp_chunk(1024, 4096), 1024);
        assert_eq!(clamp_chunk(8192, 4096), 4096);
    }

    #[test]
    fn signed_offset_math_guards_overflow() {
        assert_eq!(add_signed(10, 5), Some(15));
        assert_eq!(add_signed(10, -4), Some(6));
        assert_eq!(add_signed(3, -4), None);
    }
}
