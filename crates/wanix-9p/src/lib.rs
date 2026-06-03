//! 9P server adapters backed by Rust Wanix filesystems.
//!
//! This crate maps typed `wanix-protocol` 9P frames onto `wanix-fs`
//! filesystems. It owns fid state and filesystem error mapping, while the
//! protocol crate remains dependency-free and wire-only.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use wanix_fs::{
    File, FileSeekFrom, FileSystem, FileType, FsError, Metadata, NormalizedPath, OpenOptions,
};
use wanix_protocol::{
    P9_TATTACH, P9_TCLUNK, P9_TLOPEN, P9_TREAD, P9_TVERSION, P9_TWALK, P9_TWRITE,
    P9_VERSION_9P2000_L, P9Error, P9Frame, P9Qid, P9Version, p9_decode_tattach, p9_decode_tclunk,
    p9_decode_tlopen, p9_decode_tread, p9_decode_tversion, p9_decode_twalk, p9_decode_twrite,
    p9_rattach, p9_rclunk, p9_rlerror, p9_rlopen, p9_rread, p9_rversion, p9_rwalk, p9_rwrite,
};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix 9P filesystem server adapters";

/// Default maximum 9P message size accepted by the Rust Wanix server.
pub const DEFAULT_MAX_MSIZE: u32 = 131_072;

const RREAD_HEADER_LEN: u32 = 11;
const RLOPEN_OVERHEAD: u32 = 24;

const EBADF: u32 = 9;
const EACCES: u32 = 13;
const EEXIST: u32 = 17;
const ENOTDIR: u32 = 20;
const EISDIR: u32 = 21;
const EINVAL: u32 = 22;
const ENOTEMPTY: u32 = 39;
const EOPNOTSUPP: u32 = 95;

const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_CREAT: u32 = 0o100;
const O_TRUNC: u32 = 0o1000;

/// Error returned when a request is too malformed to turn into a 9P reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanix9pError {
    /// The request payload failed protocol decoding.
    Protocol(P9Error),
    /// A filesystem path cannot be represented as a normalized Wanix path.
    InvalidPath(String),
}

impl fmt::Display for Wanix9pError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "9P protocol error: {error}"),
            Self::InvalidPath(path) => write!(f, "invalid 9P walk path: {path}"),
        }
    }
}

impl Error for Wanix9pError {}

impl From<P9Error> for Wanix9pError {
    fn from(error: P9Error) -> Self {
        Self::Protocol(error)
    }
}

/// Small in-process 9P2000.L server backed by one Wanix filesystem root.
pub struct P9Server {
    root: Arc<dyn FileSystem>,
    fids: BTreeMap<u32, FidEntry>,
    msize: u32,
    max_msize: u32,
}

struct FidEntry {
    path: NormalizedPath,
    file: Option<Box<dyn File>>,
}

impl P9Server {
    /// Creates a server around `root` using [`DEFAULT_MAX_MSIZE`].
    #[must_use]
    pub fn new(root: Arc<dyn FileSystem>) -> Self {
        Self {
            root,
            fids: BTreeMap::new(),
            msize: DEFAULT_MAX_MSIZE,
            max_msize: DEFAULT_MAX_MSIZE,
        }
    }

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
        match frame.message_type() {
            P9_TVERSION => self.handle_version(frame),
            P9_TATTACH => self.handle_attach(frame),
            P9_TWALK => self.handle_walk(frame),
            P9_TLOPEN => self.handle_open(frame),
            P9_TREAD => self.handle_read(frame),
            P9_TWRITE => self.handle_write(frame),
            P9_TCLUNK => self.handle_clunk(frame),
            _ => Ok(p9_rlerror(frame.tag(), EOPNOTSUPP)),
        }
    }

    fn handle_version(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let P9Version { msize, version } = p9_decode_tversion(frame)?;
        self.fids.clear();
        self.msize = msize.min(self.max_msize);
        let version = if version == P9_VERSION_9P2000_L {
            P9_VERSION_9P2000_L
        } else {
            "unknown"
        };
        Ok(p9_rversion(frame.tag(), self.msize, version)?)
    }

    fn handle_attach(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let attach = p9_decode_tattach(frame)?;
        let path = NormalizedPath::new(".").expect("root path is valid");
        let qid = match self.qid_for_path(&path) {
            Ok(qid) => qid,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        self.fids.insert(attach.fid, FidEntry { path, file: None });
        Ok(p9_rattach(frame.tag(), qid))
    }

    fn handle_walk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let walk = p9_decode_twalk(frame)?;
        let Some(source) = self.fids.get(&walk.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let mut path = source.path.clone();
        let mut qids = Vec::with_capacity(walk.names.len());
        for name in &walk.names {
            path = join_walk_component(&path, name)?;
            match self.qid_for_path(&path) {
                Ok(qid) => qids.push(qid),
                Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
            }
        }
        self.fids.insert(walk.newfid, FidEntry { path, file: None });
        Ok(p9_rwalk(frame.tag(), &qids)?)
    }

    fn handle_open(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let open = p9_decode_tlopen(frame)?;
        let Some(path) = self.fids.get(&open.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let qid = match self.qid_for_path(&path) {
            Ok(qid) => qid,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let file = match self.root.open(&path, open_options_from_flags(open.flags)) {
            Ok(file) => file,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let Some(entry) = self.fids.get_mut(&open.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        entry.file = Some(file);
        Ok(p9_rlopen(
            frame.tag(),
            qid,
            self.msize.saturating_sub(RLOPEN_OVERHEAD),
        ))
    }

    fn handle_read(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let read = p9_decode_tread(frame)?;
        let Some(entry) = self.fids.get_mut(&read.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(error) = file.seek(FileSeekFrom::Start(read.offset)) {
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

    fn handle_write(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let write = p9_decode_twrite(frame)?;
        let Some(entry) = self.fids.get_mut(&write.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(error) = file.seek(FileSeekFrom::Start(write.offset)) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let count = match file.write(&write.data) {
            Ok(count) => count,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        Ok(p9_rwrite(frame.tag(), count as u32))
    }

    fn handle_clunk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let clunk = p9_decode_tclunk(frame)?;
        if self.fids.remove(&clunk.fid).is_some() {
            Ok(p9_rclunk(frame.tag()))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    fn qid_for_path(&self, path: &NormalizedPath) -> Result<P9Qid, FsError> {
        Ok(qid_for_metadata(path, self.root.metadata(path)?))
    }
}

fn join_walk_component(
    base: &NormalizedPath,
    component: &str,
) -> Result<NormalizedPath, Wanix9pError> {
    let path = if base.as_str() == "." {
        component.to_owned()
    } else {
        format!("{}/{component}", base.as_str())
    };
    NormalizedPath::new(&path).map_err(|_| Wanix9pError::InvalidPath(path))
}

fn qid_for_metadata(path: &NormalizedPath, metadata: Metadata) -> P9Qid {
    let qid_type = match metadata.file_type() {
        FileType::Directory => 0x80,
        FileType::Symlink => 0x02,
        FileType::File => 0,
    };
    P9Qid {
        qid_type,
        version: 0,
        path: fnv1a_64(path.as_str().as_bytes()),
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn open_options_from_flags(flags: u32) -> OpenOptions {
    let access = flags & O_ACCMODE;
    OpenOptions {
        read: access != O_WRONLY,
        write: access == O_WRONLY || access == O_RDWR,
        create: flags & O_CREAT != 0,
        truncate: flags & O_TRUNC != 0,
    }
}

fn errno_for_fs(error: &FsError) -> u32 {
    match error {
        FsError::InvalidPath(_) | FsError::InvalidOffset | FsError::InvalidTime => EINVAL,
        FsError::NotFound => 2,
        FsError::NotSupported => EOPNOTSUPP,
        FsError::PermissionDenied => EACCES,
        FsError::AlreadyExists => EEXIST,
        FsError::NotDirectory => ENOTDIR,
        FsError::IsDirectory => EISDIR,
        FsError::InvalidFd => EBADF,
        FsError::NotEmpty => ENOTEMPTY,
        FsError::Other(_) => EINVAL,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::MemFs;
    use wanix_protocol::{
        P9_RATTACH, P9_RLERROR, P9_RLOPEN, P9_RREAD, P9_RVERSION, P9_RWALK, P9_RWRITE,
        p9_decode_rlerror, p9_decode_rlopen, p9_decode_rread, p9_decode_rversion, p9_decode_rwalk,
        p9_decode_rwrite, p9_tattach, p9_tclunk, p9_tlopen, p9_tread, p9_tversion, p9_twalk,
        p9_twrite,
    };

    use super::*;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn server_reads_file_through_version_attach_walk_open_read_clunk() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello 9p").unwrap();
        let mut server = server(fs);

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        assert_eq!(p9_decode_rversion(&response).unwrap().msize, 8192);

        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server
            .handle_frame(&p9_twalk(3, 1, 2, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 1);

        let response = server.handle_frame(&p9_tlopen(4, 2, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
        assert_eq!(
            p9_decode_rlopen(&response).unwrap().1,
            8192 - RLOPEN_OVERHEAD
        );

        let response = server.handle_frame(&p9_tread(5, 2, 0, 5)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"hello");

        let response = server.handle_frame(&p9_tclunk(6, 2)).unwrap();
        assert_eq!(response.message_type(), wanix_protocol::P9_RCLUNK);
    }

    #[test]
    fn server_writes_file_through_open_write() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("out.txt", b"xxxxxxxx").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["out.txt"]);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(3, 2, O_RDWR | O_TRUNC))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );

        let response = server
            .handle_frame(&p9_twrite(4, 2, 0, b"made").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 4);
        assert_eq!(fs.read_file("out.txt").unwrap(), b"made");
    }

    #[test]
    fn walk_missing_path_returns_lerror() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_twalk(2, 1, 2, &["missing"]).unwrap())
            .unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, 2);
    }

    #[test]
    fn version_resets_fid_table() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tversion(2, 4096, P9_VERSION_9P2000_L).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);

        let response = server
            .handle_frame(&p9_twalk(3, 1, 2, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn zero_name_walk_clones_fid_path() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_twalk(2, 1, 2, &[]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert!(p9_decode_rwalk(&response).unwrap().is_empty());

        let response = server
            .handle_frame(&p9_twalk(3, 2, 3, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
    }

    fn server(fs: Arc<MemFs>) -> P9Server {
        let root: Arc<dyn FileSystem> = fs;
        P9Server::new(root)
    }

    fn attach_root(server: &mut P9Server) {
        let response = server
            .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);
    }

    fn walk(server: &mut P9Server, fid: u32, newfid: u32, names: &[&str]) {
        let response = server
            .handle_frame(&p9_twalk(2, fid, newfid, names).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
    }
}
