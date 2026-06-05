use super::PayloadCursor;
use crate::p9::{P9AttrBody, P9Error};

struct AttrIdentity {
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u64,
    rdev: u64,
}

struct AttrStorage {
    size: u64,
    block_size: u64,
    blocks: u64,
}

struct AttrTimes {
    atime_seconds: u64,
    atime_nanoseconds: u64,
    mtime_seconds: u64,
    mtime_nanoseconds: u64,
    ctime_seconds: u64,
    ctime_nanoseconds: u64,
    btime_seconds: u64,
    btime_nanoseconds: u64,
}

struct AttrVersion {
    generation: u64,
    data_version: u64,
}

impl<'a> PayloadCursor<'a> {
    pub(in crate::p9) fn read_attr_body(&mut self) -> Result<P9AttrBody, P9Error> {
        let identity = self.read_attr_identity()?;
        let storage = self.read_attr_storage()?;
        let times = self.read_attr_times()?;
        let version = self.read_attr_version()?;
        Ok(P9AttrBody {
            mode: identity.mode,
            uid: identity.uid,
            gid: identity.gid,
            nlink: identity.nlink,
            rdev: identity.rdev,
            size: storage.size,
            block_size: storage.block_size,
            blocks: storage.blocks,
            atime_seconds: times.atime_seconds,
            atime_nanoseconds: times.atime_nanoseconds,
            mtime_seconds: times.mtime_seconds,
            mtime_nanoseconds: times.mtime_nanoseconds,
            ctime_seconds: times.ctime_seconds,
            ctime_nanoseconds: times.ctime_nanoseconds,
            btime_seconds: times.btime_seconds,
            btime_nanoseconds: times.btime_nanoseconds,
            generation: version.generation,
            data_version: version.data_version,
        })
    }

    fn read_attr_identity(&mut self) -> Result<AttrIdentity, P9Error> {
        Ok(AttrIdentity {
            mode: self.read_u32()?,
            uid: self.read_u32()?,
            gid: self.read_u32()?,
            nlink: self.read_u64()?,
            rdev: self.read_u64()?,
        })
    }

    fn read_attr_storage(&mut self) -> Result<AttrStorage, P9Error> {
        Ok(AttrStorage {
            size: self.read_u64()?,
            block_size: self.read_u64()?,
            blocks: self.read_u64()?,
        })
    }

    fn read_attr_times(&mut self) -> Result<AttrTimes, P9Error> {
        Ok(AttrTimes {
            atime_seconds: self.read_u64()?,
            atime_nanoseconds: self.read_u64()?,
            mtime_seconds: self.read_u64()?,
            mtime_nanoseconds: self.read_u64()?,
            ctime_seconds: self.read_u64()?,
            ctime_nanoseconds: self.read_u64()?,
            btime_seconds: self.read_u64()?,
            btime_nanoseconds: self.read_u64()?,
        })
    }

    fn read_attr_version(&mut self) -> Result<AttrVersion, P9Error> {
        Ok(AttrVersion {
            generation: self.read_u64()?,
            data_version: self.read_u64()?,
        })
    }
}
