use crate::{Errno, WasiOpenOptions, WasiPathOpen, WasiRights};

pub(super) struct DirectoryOpenRights {
    pub(super) base: WasiRights,
    pub(super) inheriting: WasiRights,
}

impl DirectoryOpenRights {
    pub(super) fn new(
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<Self, Errno> {
        let rights = request.map_or_else(
            || Self::default_from_parent(parent_rights_inheriting),
            Self::requested,
        );
        rights.validate_supported(request)?;
        rights.validate_parent_rights(request, parent_rights_inheriting)?;
        Ok(rights)
    }

    fn default_from_parent(parent_rights_inheriting: WasiRights) -> Self {
        Self {
            base: WasiRights::DIRECTORY_BASE.intersection(parent_rights_inheriting),
            inheriting: WasiRights::DIRECTORY_INHERITING.intersection(parent_rights_inheriting),
        }
    }

    const fn requested(request: WasiPathOpen) -> Self {
        Self {
            base: request.rights_base(),
            inheriting: request.rights_inheriting(),
        }
    }

    fn validate_supported(&self, request: Option<WasiPathOpen>) -> Result<(), Errno> {
        let supported = if request.is_some() {
            WasiRights::DIRECTORY_INHERITING
        } else {
            WasiRights::DIRECTORY_BASE
        };
        if supported.contains(self.base) {
            return Ok(());
        }
        Err(Errno::Notcapable)
    }

    fn validate_parent_rights(
        &self,
        request: Option<WasiPathOpen>,
        parent_rights_inheriting: WasiRights,
    ) -> Result<(), Errno> {
        if request.is_none()
            || (parent_rights_inheriting.contains(self.base)
                && parent_rights_inheriting.contains(self.inheriting))
        {
            return Ok(());
        }
        Err(Errno::Notcapable)
    }
}

pub(super) fn reject_directory_write_options(options: WasiOpenOptions) -> Result<(), Errno> {
    if options.write || options.create || options.truncate {
        return Err(Errno::Isdir);
    }
    if options.append {
        return Err(Errno::Notcapable);
    }
    Ok(())
}
