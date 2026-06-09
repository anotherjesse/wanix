use wanix_fs::{File, FsError, FsResult, OpenOptions};

use crate::PipeDevice;
use crate::files::{BytesFile, NewPipeFile, PipeReader, PipeWriter, require_read_only};
use crate::path::PipePath;

pub(crate) fn open_pipe_path(
    device: &PipeDevice,
    path: PipePath<'_>,
    options: OpenOptions,
) -> FsResult<Box<dyn File>> {
    match path {
        PipePath::Root | PipePath::Channel(_) => Err(FsError::IsDirectory),
        PipePath::New => {
            require_read_only(options)?;
            Ok(Box::new(NewPipeFile::new(device.clone())))
        }
        PipePath::Id(id) => {
            require_read_only(options)?;
            device.channel(id)?;
            Ok(Box::new(BytesFile::new(format!("{id}\n").into_bytes())))
        }
        PipePath::Data(id) => open_data(device, id, options),
    }
}

fn open_data(device: &PipeDevice, id: &str, options: OpenOptions) -> FsResult<Box<dyn File>> {
    if options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    // A pipe end is unidirectional: reject opening one handle for both
    // directions, and require exactly one of read/write.
    if options.read && options.write {
        return Err(FsError::NotSupported);
    }
    let channel = device.channel(id)?;
    if options.read {
        Ok(Box::new(PipeReader::new(channel)?))
    } else if options.write {
        Ok(Box::new(PipeWriter::new(channel)?))
    } else {
        Err(FsError::PermissionDenied)
    }
}
