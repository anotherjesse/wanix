use wanix_fs::NormalizedPath;
use wanix_protocol::P9Frame;

use crate::path::join_walk_component;
use crate::{P9Server, Wanix9pError};

impl P9Server {
    pub(super) fn child_path_or_reply(
        &self,
        tag: u16,
        dir_fid: u32,
        name: &str,
    ) -> Result<Result<NormalizedPath, P9Frame>, Wanix9pError> {
        let dir_path = match self.fid_path_or_reply(tag, dir_fid) {
            Ok(path) => path,
            Err(response) => return Ok(Err(response)),
        };
        Ok(Ok(join_walk_component(&dir_path, name)?))
    }
}
