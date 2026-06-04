use wanix_fs::{FileSeekFrom, FileType};
use wanix_protocol::{
    P9Frame, p9_decode_tlopen, p9_decode_tread, p9_decode_twrite, p9_rlerror, p9_rlopen, p9_rread,
    p9_rwrite,
};

use crate::attrs::qid_for_metadata;
use crate::{
    EBADF, EISDIR, O_ACCMODE, O_APPEND, O_CREAT, O_TRUNC, P9Server, RLOPEN_OVERHEAD,
    RREAD_HEADER_LEN, Wanix9pError, errno_for_fs, open_options_from_flags,
};

impl P9Server {
    pub(super) fn handle_open(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let open = p9_decode_tlopen(frame)?;
        let Some(path) = self.fids.get(&open.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let metadata = match self.root.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let qid = qid_for_metadata(&path, metadata.clone());
        let is_directory = metadata.file_type() == FileType::Directory;
        let append = !is_directory && open.flags & O_APPEND != 0;
        let file = if is_directory {
            if open.flags & (O_ACCMODE | O_CREAT | O_TRUNC) != 0 {
                return Ok(p9_rlerror(frame.tag(), EISDIR));
            }
            None
        } else {
            match self.root.open(&path, open_options_from_flags(open.flags)) {
                Ok(file) => Some(file),
                Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
            }
        };
        let Some(entry) = self.fids.get_mut(&open.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        entry.file = file;
        entry.append = append;
        Ok(p9_rlopen(
            frame.tag(),
            qid,
            self.msize.saturating_sub(RLOPEN_OVERHEAD),
        ))
    }

    pub(super) fn handle_read(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let read = p9_decode_tread(frame)?;
        let Some(entry) = self.fids.get_mut(&read.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if file.is_seekable()
            && let Err(error) = file.seek(FileSeekFrom::Start(read.offset))
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let count = read.count.min(self.msize.saturating_sub(RREAD_HEADER_LEN)) as usize;
        let mut buf = vec![0; count];
        let read_count = match file.read(&mut buf) {
            Ok(read_count) => read_count,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        buf.truncate(read_count);
        Ok(p9_rread(frame.tag(), &buf)?)
    }

    pub(super) fn handle_write(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let write = p9_decode_twrite(frame)?;
        let Some(entry) = self.fids.get_mut(&write.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let append = entry.append;
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let seek_from = if append {
            FileSeekFrom::End(0)
        } else {
            FileSeekFrom::Start(write.offset)
        };
        if file.is_seekable()
            && let Err(error) = file.seek(seek_from)
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let count = match file.write(&write.data) {
            Ok(count) => count,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        Ok(p9_rwrite(frame.tag(), count as u32))
    }
}
