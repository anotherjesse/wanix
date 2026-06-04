use wanix_fs::FileType;
use wanix_protocol::{
    P9Frame, p9_decode_tlcreate, p9_decode_tlink, p9_decode_tmkdir, p9_decode_tmknod,
    p9_decode_tremove, p9_decode_trename, p9_decode_trenameat, p9_decode_tsymlink,
    p9_decode_tunlinkat, p9_rlcreate, p9_rlerror, p9_rlink, p9_rmkdir, p9_rremove, p9_rrename,
    p9_rrenameat, p9_rsymlink, p9_runlinkat,
};

use crate::attrs::qid_for_metadata;
use crate::path::join_walk_component;
use crate::{
    AT_REMOVEDIR, EBADF, EOPNOTSUPP, O_APPEND, P9Server, RLOPEN_OVERHEAD, Wanix9pError,
    errno_for_fs, open_options_from_flags,
};

impl P9Server {
    pub(super) fn handle_create(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let create = p9_decode_tlcreate(frame)?;
        let Some(dir_path) = self.fids.get(&create.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &create.name)?;
        let existing_path = self.metadata_no_follow(&path).is_ok();
        let mut options = open_options_from_flags(create.flags);
        options.create = true;
        let file = match self.root.open(&path, options) {
            Ok(file) => file,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let metadata = match self.root.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let qid = qid_for_metadata(&path, metadata);
        if !existing_path {
            self.set_created_gid(&path, create.gid);
        }
        let Some(entry) = self.fids.get_mut(&create.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        entry.path = path;
        entry.file = Some(file);
        entry.append = create.flags & O_APPEND != 0;
        Ok(p9_rlcreate(
            frame.tag(),
            qid,
            self.msize.saturating_sub(RLOPEN_OVERHEAD),
        ))
    }

    pub(super) fn handle_symlink(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let symlink = p9_decode_tsymlink(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&symlink.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &symlink.name)?;
        if let Err(error) = self.root.symlink(symlink.target.as_bytes(), &path) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        };
        let metadata = match self.metadata_no_follow(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        self.set_created_gid(&path, symlink.gid);
        Ok(p9_rsymlink(frame.tag(), qid_for_metadata(&path, metadata)))
    }

    pub(super) fn handle_mknod(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let mknod = p9_decode_tmknod(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&mknod.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let _path = join_walk_component(&dir_path, &mknod.name)?;
        Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
    }

    pub(super) fn handle_link(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let link = p9_decode_tlink(frame)?;
        let Some(dir_path) = self.fids.get(&link.dir_fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(old_path) = self.fids.get(&link.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &link.name)?;
        match self.root.hard_link(&old_path, &path) {
            Ok(()) => {
                self.owners.insert(path, self.owner_attrs(&old_path));
                Ok(p9_rlink(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    pub(super) fn handle_mkdir(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let mkdir = p9_decode_tmkdir(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&mkdir.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &mkdir.name)?;
        if let Err(error) = self.root.create_dir(&path) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        };
        let metadata = match self.root.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        self.set_created_gid(&path, mkdir.gid);
        Ok(p9_rmkdir(frame.tag(), qid_for_metadata(&path, metadata)))
    }

    pub(super) fn handle_rename(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let rename = p9_decode_trename(frame)?;
        let Some(old_path) = self.fids.get(&rename.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(new_dir_path) = self
            .fids
            .get(&rename.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let new_path = join_walk_component(&new_dir_path, &rename.name)?;
        match self.root.rename(&old_path, &new_path) {
            Ok(()) => {
                self.move_owner_attrs(&old_path, &new_path);
                self.remove_replaced_fid_paths(&old_path, &new_path);
                self.move_fid_paths(&old_path, &new_path);
                Ok(p9_rrename(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    pub(super) fn handle_renameat(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let rename = p9_decode_trenameat(frame)?;
        let Some(old_dir_path) = self
            .fids
            .get(&rename.old_dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(new_dir_path) = self
            .fids
            .get(&rename.new_dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let old_path = join_walk_component(&old_dir_path, &rename.old_name)?;
        let new_path = join_walk_component(&new_dir_path, &rename.new_name)?;
        match self.root.rename(&old_path, &new_path) {
            Ok(()) => {
                self.move_owner_attrs(&old_path, &new_path);
                self.remove_replaced_fid_paths(&old_path, &new_path);
                self.move_fid_paths(&old_path, &new_path);
                Ok(p9_rrenameat(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    pub(super) fn handle_remove(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let remove = p9_decode_tremove(frame)?;
        let Some(entry) = self.fids.remove(&remove.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = entry.path;
        let metadata = match self.metadata_no_follow(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let result = if metadata.file_type() == FileType::Directory {
            self.root.remove_dir(&path)
        } else {
            self.root.remove_file(&path)
        };
        match result {
            Ok(()) => {
                self.remove_owner_attrs(&path);
                Ok(p9_rremove(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    pub(super) fn handle_unlinkat(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let unlink = p9_decode_tunlinkat(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&unlink.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &unlink.name)?;
        let result = if unlink.flags & AT_REMOVEDIR != 0 {
            self.root.remove_dir(&path)
        } else {
            self.root.remove_file(&path)
        };
        match result {
            Ok(()) => {
                self.remove_owner_attrs(&path);
                Ok(p9_runlinkat(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }
}
