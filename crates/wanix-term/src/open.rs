use wanix_fs::{File, FsError, FsResult, OpenOptions};

use crate::TermDevice;
use crate::files::{
    BytesFile, ControlFile, NewTermFile, TermFile, WinchFile, access, require_file_open,
    require_read_only,
};
use crate::path::TermPath;
use crate::state::TermSide;

pub(crate) fn open_term_path(
    device: &TermDevice,
    path: TermPath<'_>,
    options: OpenOptions,
) -> FsResult<Box<dyn File>> {
    match path {
        TermPath::Root | TermPath::Resource(_) => Err(FsError::IsDirectory),
        TermPath::New => open_new(device, options),
        TermPath::Id(id) => open_id(device, id, options),
        TermPath::Ctl(id) => open_ctl(device, id, options),
        TermPath::Data(id) => open_stream(device, id, TermSide::Data, options),
        TermPath::Program(id) => open_stream(device, id, TermSide::Program, options),
        TermPath::Winch(id) => open_winch(device, id, options),
    }
}

fn open_new(device: &TermDevice, options: OpenOptions) -> FsResult<Box<dyn File>> {
    require_read_only(options)?;
    Ok(Box::new(NewTermFile::new(device.clone())))
}

fn open_id(device: &TermDevice, id: &str, options: OpenOptions) -> FsResult<Box<dyn File>> {
    require_read_only(options)?;
    let resource = device.resource(id)?;
    Ok(Box::new(BytesFile::new(
        format!("{}\n", resource.id).into_bytes(),
    )))
}

fn open_ctl(device: &TermDevice, id: &str, options: OpenOptions) -> FsResult<Box<dyn File>> {
    Ok(Box::new(ControlFile::new(
        device.clone(),
        id.to_owned(),
        access(options)?,
    )))
}

fn open_stream(
    device: &TermDevice,
    id: &str,
    side: TermSide,
    options: OpenOptions,
) -> FsResult<Box<dyn File>> {
    require_file_open(options)?;
    Ok(Box::new(TermFile::new(device.resource(id)?, side)))
}

fn open_winch(device: &TermDevice, id: &str, options: OpenOptions) -> FsResult<Box<dyn File>> {
    require_file_open(options)?;
    Ok(Box::new(WinchFile::new(
        device.resource(id)?,
        options.read,
        options.write,
    )?))
}
