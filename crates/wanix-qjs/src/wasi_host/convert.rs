use rust_wasi_quickjs::{
    QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiFileType,
    QuickJsWasiWhence,
};
use wanix_fs::FileType;
use wanix_wasi::{Errno, FileStat, WasiFdStat, WasiFileType, WasiWhence};

macro_rules! wanix_errno_to_quickjs {
    ($errno:expr) => {
        match $errno {
            Errno::Success => QuickJsWasiErrno::Io,
            Errno::Badf => QuickJsWasiErrno::Badf,
            Errno::Inval => QuickJsWasiErrno::Inval,
            Errno::Nametoolong => QuickJsWasiErrno::Nametoolong,
            Errno::Noent => QuickJsWasiErrno::Noent,
            Errno::Exist => QuickJsWasiErrno::Exist,
            Errno::Notdir => QuickJsWasiErrno::Notdir,
            Errno::Isdir => QuickJsWasiErrno::Isdir,
            Errno::Notempty => QuickJsWasiErrno::Notempty,
            Errno::Nosys => QuickJsWasiErrno::Nosys,
            Errno::Notcapable => QuickJsWasiErrno::Notcapable,
            Errno::Io => QuickJsWasiErrno::Io,
        }
    };
}

pub(super) fn convert_errno(errno: Errno) -> QuickJsWasiErrno {
    wanix_errno_to_quickjs!(errno)
}

fn convert_file_type(file_type: WasiFileType) -> QuickJsWasiFileType {
    match file_type {
        WasiFileType::Unknown => QuickJsWasiFileType::Unknown,
        WasiFileType::CharacterDevice => QuickJsWasiFileType::CharacterDevice,
        WasiFileType::Directory => QuickJsWasiFileType::Directory,
        WasiFileType::RegularFile => QuickJsWasiFileType::RegularFile,
        WasiFileType::SymbolicLink => QuickJsWasiFileType::SymbolicLink,
    }
}

pub(super) fn convert_wanix_file_type(file_type: FileType) -> QuickJsWasiFileType {
    match file_type {
        FileType::File => QuickJsWasiFileType::RegularFile,
        FileType::Directory => QuickJsWasiFileType::Directory,
        FileType::Symlink => QuickJsWasiFileType::SymbolicLink,
    }
}

pub(super) fn convert_fdstat(stat: WasiFdStat) -> QuickJsWasiFdStat {
    QuickJsWasiFdStat::new_with_fdflags(
        convert_file_type(stat.file_type()),
        stat.fdflags(),
        stat.rights_base().bits(),
        stat.rights_inheriting().bits(),
    )
}

pub(super) fn convert_filestat(stat: FileStat) -> QuickJsWasiFileStat {
    QuickJsWasiFileStat::new_with_times(
        convert_file_type(stat.wasi_file_type()),
        stat.len(),
        stat.accessed_time_ns(),
        stat.modified_time_ns(),
        stat.changed_time_ns(),
    )
}

pub(super) fn convert_whence(whence: QuickJsWasiWhence) -> WasiWhence {
    match whence {
        QuickJsWasiWhence::Set => WasiWhence::Set,
        QuickJsWasiWhence::Cur => WasiWhence::Cur,
        QuickJsWasiWhence::End => WasiWhence::End,
    }
}

#[cfg(test)]
mod tests {
    use rust_wasi_quickjs::QuickJsWasiErrno;
    use wanix_wasi::Errno;

    use super::convert_errno;

    #[test]
    fn errno_mapping_projects_each_wanix_errno_to_quickjs_errno() {
        let cases = [
            (Errno::Success, QuickJsWasiErrno::Io),
            (Errno::Badf, QuickJsWasiErrno::Badf),
            (Errno::Inval, QuickJsWasiErrno::Inval),
            (Errno::Nametoolong, QuickJsWasiErrno::Nametoolong),
            (Errno::Noent, QuickJsWasiErrno::Noent),
            (Errno::Exist, QuickJsWasiErrno::Exist),
            (Errno::Notdir, QuickJsWasiErrno::Notdir),
            (Errno::Isdir, QuickJsWasiErrno::Isdir),
            (Errno::Notempty, QuickJsWasiErrno::Notempty),
            (Errno::Nosys, QuickJsWasiErrno::Nosys),
            (Errno::Notcapable, QuickJsWasiErrno::Notcapable),
            (Errno::Io, QuickJsWasiErrno::Io),
        ];
        for (wanix, quickjs) in cases {
            assert_eq!(convert_errno(wanix), quickjs, "{wanix:?}");
        }
    }

    #[test]
    fn success_errno_maps_to_io_because_errors_should_not_be_successful() {
        assert_eq!(convert_errno(Errno::Success), QuickJsWasiErrno::Io);
    }
}
