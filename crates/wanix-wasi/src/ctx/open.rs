use crate::{Errno, WasiOpenOptions, WasiPathOpen, WasiRights};

pub(super) struct FileOpenRequest {
    options: WasiOpenOptions,
    request: Option<WasiPathOpen>,
    parent_rights_inheriting: WasiRights,
}

impl FileOpenRequest {
    pub(super) fn new(
        options: WasiOpenOptions,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<Self, Errno> {
        let request = Self {
            options,
            request,
            parent_rights_inheriting,
        };
        request.validate_requested_rights()?;
        Ok(request)
    }

    pub(super) fn rights_base_for_opened_file(&self, seekable: bool) -> Result<WasiRights, Errno> {
        let supported_rights = open_file_rights(self.options.read, self.options.write, seekable);
        let rights_base = self.request.map_or_else(
            || supported_rights.intersection(self.parent_rights_inheriting),
            |request| request.file_rights_base().intersection(supported_rights),
        );
        self.validate_required_io_rights(rights_base)?;
        Ok(rights_base)
    }

    fn validate_requested_rights(&self) -> Result<(), Errno> {
        let requested_file_rights = open_file_rights(self.options.read, self.options.write, true);
        if let Some(request) = self.request {
            let requested_base = request.file_rights_base();
            if !requested_file_rights.contains(requested_base)
                || !self.parent_rights_inheriting.contains(requested_base)
            {
                return Err(Errno::Notcapable);
            }
            return Ok(());
        }

        let default_file_rights = requested_file_rights.intersection(self.parent_rights_inheriting);
        self.validate_required_io_rights(default_file_rights)
    }

    fn validate_required_io_rights(&self, rights: WasiRights) -> Result<(), Errno> {
        if self.options.read && !rights.contains(WasiRights::FD_READ) {
            return Err(Errno::Notcapable);
        }
        if self.options.write && !rights.contains(WasiRights::FD_WRITE) {
            return Err(Errno::Notcapable);
        }
        Ok(())
    }
}

fn open_file_rights(read: bool, write: bool, seekable: bool) -> WasiRights {
    let mut rights = WasiRights::FD_FILESTAT_GET | WasiRights::FD_FILESTAT_SET_TIMES;
    if read {
        rights |= WasiRights::FD_READ;
    }
    if write {
        rights |= WasiRights::FD_WRITE | WasiRights::FD_FILESTAT_SET_SIZE;
    }
    if seekable {
        rights |= WasiRights::FD_SEEK | WasiRights::FD_TELL;
    }
    rights
}
