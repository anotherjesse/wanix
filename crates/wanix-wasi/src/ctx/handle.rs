use std::fmt;

use wanix_fs::NormalizedPath;

use crate::{WasiFile, WasiRights};

pub(super) enum Handle {
    Stdio {
        file: WasiFile,
    },
    Preopen {
        source_path: NormalizedPath,
        guest_path: NormalizedPath,
    },
    Directory {
        path: NormalizedPath,
        rights_base: WasiRights,
        rights_inheriting: WasiRights,
    },
    File {
        file: WasiFile,
        path: NormalizedPath,
        read: bool,
        write: bool,
        rights_base: WasiRights,
        fdflags: u16,
    },
}

pub(super) struct OpenFileHandle {
    pub(super) file: WasiFile,
    pub(super) path: NormalizedPath,
    pub(super) read: bool,
    pub(super) write: bool,
    pub(super) rights_base: WasiRights,
    pub(super) fdflags: u16,
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stdio { file } => f.debug_struct("Stdio").field("file", file).finish(),
            Self::Preopen {
                source_path,
                guest_path,
            } => f
                .debug_struct("Preopen")
                .field("source_path", source_path)
                .field("guest_path", guest_path)
                .finish(),
            Self::Directory {
                path,
                rights_base,
                rights_inheriting,
            } => f
                .debug_struct("Directory")
                .field("path", path)
                .field("rights_base", rights_base)
                .field("rights_inheriting", rights_inheriting)
                .finish(),
            Self::File {
                path,
                read,
                write,
                rights_base,
                fdflags,
                ..
            } => f
                .debug_struct("File")
                .field("path", path)
                .field("read", read)
                .field("write", write)
                .field("rights_base", rights_base)
                .field("fdflags", fdflags)
                .finish(),
        }
    }
}
