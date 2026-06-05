use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::os::wasi::io::AsRawFd;

const ROOT_FD: u32 = 3;
const MTIM_FLAG: u16 = 1 << 2;

// Raw WASI imports whose stable std wrappers are unavailable or not direct
// enough for the syscall contract these fixture commands exercise.
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn path_symlink(
        old_path: *const u8,
        old_path_len: usize,
        fd: u32,
        new_path: *const u8,
        new_path_len: usize,
    ) -> u16;
    fn fd_tell(fd: u32, out: *mut u64) -> u16;
    fn path_filestat_set_times(
        fd: u32,
        flags: u32,
        path: *const u8,
        path_len: usize,
        atim: u64,
        mtim: u64,
        fst_flags: u16,
    ) -> u16;
}

/// Sets `path`'s modified time to `mtime` ns via the `path_filestat_set_times`
/// syscall (fst_flags = MTIM). Resolves `path` against the preopened root (fd 3).
pub(crate) fn utime(path: &str, mtime: u64) {
    let rel = path.strip_prefix('/').unwrap_or(path);
    let errno = unsafe {
        path_filestat_set_times(ROOT_FD, 0, rel.as_ptr(), rel.len(), 0, mtime, MTIM_FLAG)
    };
    if errno == 0 {
        println!("ok");
    } else {
        println!("rust-wasm: utime failed {path}: errno {errno}");
    }
}

/// Seeks `path` to absolute offset `seek`, then reports the cursor via the raw
/// `fd_tell` syscall (not `fd_seek`, so this genuinely exercises `fd_tell`).
pub(crate) fn tell(path: &str, seek: u64) {
    let mut file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) => {
            println!("rust-wasm: tell failed to open {path}: {err}");
            return;
        }
    };
    if let Err(err) = file.seek(SeekFrom::Start(seek)) {
        println!("rust-wasm: tell seek failed {path}: {err}");
        return;
    }
    let mut pos: u64 = 0;
    let errno = unsafe { fd_tell(file.as_raw_fd() as u32, &mut pos) };
    if errno == 0 {
        println!("rust-wasm: tell {path} at {pos}");
    } else {
        println!("rust-wasm: tell failed {path}: errno {errno}");
    }
}

/// Creates a symbolic link `link` pointing at `target` via the `path_symlink`
/// syscall, then prints `ok`. Resolves `link` against the preopened root (fd 3)
/// so the absolute `link` path becomes namespace-relative.
pub(crate) fn symlink(target: &str, link: &str) {
    let rel = link.strip_prefix('/').unwrap_or(link);
    let errno = unsafe {
        path_symlink(
            target.as_ptr(),
            target.len(),
            ROOT_FD,
            rel.as_ptr(),
            rel.len(),
        )
    };
    if errno == 0 {
        println!("ok");
    } else {
        println!("rust-wasm: symlink failed {link} -> {target}: errno {errno}");
    }
}
