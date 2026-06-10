use std::collections::BTreeMap;

use wanix_fs::{FileSystem, FileType};

use crate::{Errno, WasiConfig, WasiCtx, WasiFd};

use super::{FIRST_PREOPEN_FD, Handle};

impl WasiCtx {
    /// Creates a WASI context and validates all configured preopens.
    pub fn try_new(config: WasiConfig) -> Result<Self, Errno> {
        let fds = configured_fds(&config)?;
        let next_fd = next_fd_after_preopens(&config)?;
        Ok(Self {
            namespace: config.namespace().clone(),
            fds,
            next_fd,
            args: config.args().to_vec(),
            env: config.env().to_vec(),
            clock_time_ns: config.clock_time_ns(),
            fd_observer: config.fd_observer(),
            cancel: config.cancel(),
        })
    }
}

fn configured_fds(config: &WasiConfig) -> Result<BTreeMap<WasiFd, Handle>, Errno> {
    let mut fds = BTreeMap::new();
    insert_stdio(config, &mut fds);
    insert_preopens(config, &mut fds)?;
    Ok(fds)
}

fn insert_stdio(config: &WasiConfig, fds: &mut BTreeMap<WasiFd, Handle>) {
    for (fd, file) in config.stdio() {
        fds.insert(*fd, Handle::Stdio { file: file.clone() });
    }
}

fn insert_preopens(config: &WasiConfig, fds: &mut BTreeMap<WasiFd, Handle>) -> Result<(), Errno> {
    for (index, preopen) in config.preopens().iter().enumerate() {
        validate_preopen_directory(config, preopen.source_path())?;
        let fd = preopen_fd(index)?;
        fds.insert(
            fd,
            Handle::Preopen {
                source_path: preopen.source_path().clone(),
                guest_path: preopen.guest_path().clone(),
            },
        );
    }
    Ok(())
}

fn validate_preopen_directory(
    config: &WasiConfig,
    path: &wanix_fs::NormalizedPath,
) -> Result<(), Errno> {
    let metadata = config.namespace().metadata(path).map_err(Errno::from)?;
    if metadata.file_type() == FileType::Directory {
        return Ok(());
    }
    Err(Errno::Notdir)
}

fn preopen_fd(index: usize) -> Result<WasiFd, Errno> {
    Ok(WasiFd::new(
        FIRST_PREOPEN_FD + u32::try_from(index).map_err(|_| Errno::Inval)?,
    ))
}

fn next_fd_after_preopens(config: &WasiConfig) -> Result<u32, Errno> {
    Ok(FIRST_PREOPEN_FD + u32::try_from(config.preopens().len()).map_err(|_| Errno::Inval)?)
}
