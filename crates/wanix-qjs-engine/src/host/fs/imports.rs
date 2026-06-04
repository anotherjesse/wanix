use super::{
    HostState, fd_close, fd_fdstat_get, fd_fdstat_set_flags, fd_filestat_get, fd_filestat_set_size,
    fd_filestat_set_times, fd_prestat_dir_name, fd_prestat_get, fd_readdir, fd_seek, fd_tell,
    path_create_directory, path_filestat_get, path_filestat_set_times, path_open, path_readlink,
    path_remove_directory, path_rename, path_symlink, path_unlink_file,
};
use wasmtime::Linker;

pub(in crate::host) fn define_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    define_fd_imports(linker)?;
    define_path_imports(linker)?;
    Ok(())
}

fn define_fd_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    define_fd_prestat_imports(linker)?;
    define_fd_io_imports(linker)?;
    define_fd_stat_imports(linker)?;
    Ok(())
}

fn define_fd_prestat_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "fd_prestat_get", fd_prestat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_prestat_dir_name",
        fd_prestat_dir_name,
    )?;
    Ok(())
}

fn define_fd_io_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "fd_readdir", fd_readdir)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_seek", fd_seek)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_tell", fd_tell)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_close", fd_close)?;
    Ok(())
}

fn define_fd_stat_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "fd_fdstat_get", fd_fdstat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_fdstat_set_flags",
        fd_fdstat_set_flags,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_filestat_get", fd_filestat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_filestat_set_times",
        fd_filestat_set_times,
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_filestat_set_size",
        fd_filestat_set_size,
    )?;
    Ok(())
}

fn define_path_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "path_open", path_open)?;
    define_path_metadata_imports(linker)?;
    define_path_mutation_imports(linker)?;
    Ok(())
}

fn define_path_metadata_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "path_readlink", path_readlink)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_get",
        path_filestat_get,
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_set_times",
        path_filestat_set_times,
    )?;
    Ok(())
}

fn define_path_mutation_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_create_directory",
        path_create_directory,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "path_symlink", path_symlink)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_remove_directory",
        path_remove_directory,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "path_rename", path_rename)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_unlink_file",
        path_unlink_file,
    )?;
    Ok(())
}
