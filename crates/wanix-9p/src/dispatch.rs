use wanix_protocol::{
    P9_TATTACH, P9_TAUTH, P9_TCLUNK, P9_TFLUSH, P9_TFLUSHF, P9_TFSYNC, P9_TGETATTR, P9_TGETLOCK,
    P9_TLCREATE, P9_TLINK, P9_TLOCK, P9_TLOPEN, P9_TMKDIR, P9_TMKNOD, P9_TREAD, P9_TREADDIR,
    P9_TREADLINK, P9_TREMOVE, P9_TRENAME, P9_TRENAMEAT, P9_TSETATTR, P9_TSTATFS, P9_TSYMLINK,
    P9_TUNLINKAT, P9_TVERSION, P9_TWALK, P9_TWALKGETATTR, P9_TWRITE, P9_TXATTRCREATE,
    P9_TXATTRWALK, P9Frame, p9_rlerror,
};

use crate::{EOPNOTSUPP, P9Server, Wanix9pError};

type DispatchResult = Result<Option<P9Frame>, Wanix9pError>;

impl P9Server {
    /// Handles one decoded 9P request frame and returns the response frame.
    ///
    /// Filesystem and unsupported-operation failures become `Rlerror` replies
    /// using Linux errno values. Malformed typed payloads return
    /// [`Wanix9pError`] because a caller may need to tear down the connection.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the request payload cannot be decoded.
    pub fn handle_frame(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        if let Some(response) = self.handle_session_frame(frame)? {
            return Ok(response);
        }
        if let Some(response) = self.handle_walk_frame(frame)? {
            return Ok(response);
        }
        if let Some(response) = self.handle_io_frame(frame)? {
            return Ok(response);
        }
        if let Some(response) = self.handle_metadata_frame(frame)? {
            return Ok(response);
        }
        if let Some(response) = self.handle_mutation_frame(frame)? {
            return Ok(response);
        }
        Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
    }

    fn handle_session_frame(&mut self, frame: &P9Frame) -> DispatchResult {
        match frame.message_type() {
            P9_TVERSION => self.handle_version(frame).map(Some),
            P9_TAUTH => self.handle_auth(frame).map(Some),
            P9_TATTACH => self.handle_attach(frame).map(Some),
            P9_TFLUSH => self.handle_flush(frame).map(Some),
            P9_TFLUSHF => self.handle_flushf(frame).map(Some),
            P9_TFSYNC => self.handle_fsync(frame).map(Some),
            P9_TCLUNK => self.handle_clunk(frame).map(Some),
            _ => Ok(None),
        }
    }

    fn handle_walk_frame(&mut self, frame: &P9Frame) -> DispatchResult {
        match frame.message_type() {
            P9_TWALK => self.handle_walk(frame).map(Some),
            P9_TWALKGETATTR => self.handle_walkgetattr(frame).map(Some),
            _ => Ok(None),
        }
    }

    fn handle_io_frame(&mut self, frame: &P9Frame) -> DispatchResult {
        match frame.message_type() {
            P9_TLOPEN => self.handle_open(frame).map(Some),
            P9_TLCREATE => self.handle_create(frame).map(Some),
            P9_TREAD => self.handle_read(frame).map(Some),
            P9_TWRITE => self.handle_write(frame).map(Some),
            P9_TREADDIR => self.handle_readdir(frame).map(Some),
            _ => Ok(None),
        }
    }

    fn handle_metadata_frame(&mut self, frame: &P9Frame) -> DispatchResult {
        match frame.message_type() {
            P9_TSTATFS => self.handle_statfs(frame).map(Some),
            P9_TREADLINK => self.handle_readlink(frame).map(Some),
            P9_TGETATTR => self.handle_getattr(frame).map(Some),
            P9_TSETATTR => self.handle_setattr(frame).map(Some),
            P9_TXATTRWALK => self.handle_xattrwalk(frame).map(Some),
            P9_TXATTRCREATE => self.handle_xattrcreate(frame).map(Some),
            P9_TLOCK => self.handle_lock(frame).map(Some),
            P9_TGETLOCK => self.handle_getlock(frame).map(Some),
            _ => Ok(None),
        }
    }

    fn handle_mutation_frame(&mut self, frame: &P9Frame) -> DispatchResult {
        match frame.message_type() {
            P9_TSYMLINK => self.handle_symlink(frame).map(Some),
            P9_TMKNOD => self.handle_mknod(frame).map(Some),
            P9_TLINK => self.handle_link(frame).map(Some),
            P9_TMKDIR => self.handle_mkdir(frame).map(Some),
            P9_TRENAME => self.handle_rename(frame).map(Some),
            P9_TRENAMEAT => self.handle_renameat(frame).map(Some),
            P9_TREMOVE => self.handle_remove(frame).map(Some),
            P9_TUNLINKAT => self.handle_unlinkat(frame).map(Some),
            _ => Ok(None),
        }
    }
}
