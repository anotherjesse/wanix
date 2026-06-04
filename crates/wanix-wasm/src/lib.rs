//! Generic WASI Preview 1 task runner for Rust Wanix.
//!
//! This crate runs an arbitrary `wasm32-wasi` command module (compiled from
//! Rust, Go, C, Zig, …) on Wasmtime, with its WASI syscalls backed by a Wanix
//! [`Namespace`](wanix_vfs::Namespace) through the engine-agnostic
//! [`WasiCtx`](wanix_wasi::WasiCtx). It is the "compiled-to-wasm task" sibling of
//! the QuickJS `qjs` task: same sandbox, same namespace/VFS, near-native speed.
//!
//! Two tasks (a `qjs` task and a `wanix-wasm` task) that are built from the same
//! `Namespace` share one filesystem — writes by one are visible to the other.

use std::io::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileType, FsError, FsResult, LocalFs, Metadata};
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::{WasiConfig, WasiCtx};
use wanix_wasi_host::WasiHost;
use wasmtime::error::Context as _;
use wasmtime::{Engine, Error, Linker, Module, Result, Store};

/// Store state for a running WASI command: a [`WasiCtx`] plus the deterministic
/// clock and recorded exit code the shared linker needs via [`WasiHost`].
pub struct WasiState {
    ctx: WasiCtx,
    clock_ns: u64,
    exit_code: Option<i32>,
}

impl WasiState {
    /// Creates state wrapping a WASI context and a fixed clock value.
    #[must_use]
    pub fn new(ctx: WasiCtx, clock_ns: u64) -> Self {
        Self {
            ctx,
            clock_ns,
            exit_code: None,
        }
    }

    /// Returns the recorded `proc_exit` code, if the guest exited.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

impl WasiHost for WasiState {
    fn wasi(&mut self) -> &mut WasiCtx {
        &mut self.ctx
    }

    fn clock_time_ns(&self) -> u64 {
        self.clock_ns
    }

    fn on_proc_exit(&mut self, code: i32) {
        self.exit_code = Some(code);
    }
}

/// A compiled `wasm32-wasi` command module ready to run as Wanix tasks.
pub struct WasiRunner {
    engine: Engine,
    module: Module,
}

impl WasiRunner {
    /// Compiles a `wasm32-wasi` module from bytes with a default engine.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime cannot compile the bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).context("failed to compile wasm module")?;
        Ok(Self { engine, module })
    }

    /// Runs the module's `_start` entry with WASI backed by `config`'s namespace.
    ///
    /// Returns the process exit code: `0` for a normal return, or the code passed
    /// to `proc_exit`.
    ///
    /// # Errors
    ///
    /// Returns an error if the WASI config is invalid, the module is missing a
    /// `_start` export, or a trap other than `proc_exit` occurs.
    pub fn run(&self, config: WasiConfig) -> Result<i32> {
        let clock_ns = config.clock_time_ns();
        let ctx = WasiCtx::try_new(config)
            .map_err(|err| Error::msg(format!("invalid WASI config: {err:?}")))?;
        let mut store = Store::new(&self.engine, WasiState::new(ctx, clock_ns));

        let mut linker = Linker::new(&self.engine);
        wanix_wasi_host::add_to_linker(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &self.module)
            .context("failed to instantiate wasm module")?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .context("module has no _start export (not a WASI command)")?;

        match start.call(&mut store, ()) {
            Ok(()) => Ok(store.data().exit_code().unwrap_or(0)),
            Err(err) => match store.data().exit_code() {
                // A clean proc_exit unwinds the guest via a trap; recover the code.
                Some(code) => Ok(code),
                None => Err(err).context("wasm task trapped"),
            },
        }
    }

    /// Runs the module with sane defaults: `dir` preopened as the namespace root,
    /// `argv` as the guest arguments, and stdout/stderr wired to the host process.
    ///
    /// This is the one-call path for embedders who just want to run a task in a
    /// host directory without assembling a [`WasiConfig`] by hand.
    ///
    /// # Errors
    ///
    /// Returns an error if `dir` cannot be opened, or the task fails to run.
    pub fn run_in_dir<S: AsRef<str>>(&self, dir: impl AsRef<Path>, argv: &[S]) -> Result<i32> {
        let mut namespace = Namespace::new();
        let local = Arc::new(
            LocalFs::new(dir.as_ref())
                .map_err(|e| Error::msg(format!("cannot open {}: {e}", dir.as_ref().display())))?,
        );
        namespace
            .bind(local, ".", ".", BindOptions::default())
            .map_err(|e| Error::msg(format!("bind failed: {e}")))?;
        let config = WasiConfig::new(namespace)
            .with_args(argv.iter().map(|s| s.as_ref().to_owned()))
            .with_stdout(host_stdout(), "stdout")
            .with_stderr(host_stderr(), "stderr");
        self.run(config)
    }
}

/// A [`File`] that forwards writes to the host process stdout or stderr.
///
/// Hands an embedder the same "see the guest's output" default the CLI uses,
/// without wiring up a [`CaptureFile`] and reading it back.
struct HostStream {
    stderr: bool,
}

impl File for HostStream {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        let written = if self.stderr {
            std::io::stderr().write(buf)
        } else {
            std::io::stdout().write(buf)
        };
        written.map_err(|e| FsError::Other(e.to_string()))
    }

    fn write_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o644))
    }
}

/// A stdout sink that forwards to the host process stdout.
#[must_use]
pub fn host_stdout() -> Box<dyn File> {
    Box::new(HostStream { stderr: false })
}

/// A stderr sink that forwards to the host process stderr.
#[must_use]
pub fn host_stderr() -> Box<dyn File> {
    Box::new(HostStream { stderr: true })
}

/// An in-memory [`File`] that captures everything written to it.
///
/// Useful as a stdout/stderr sink so a host can read back what a guest printed.
#[derive(Clone, Default)]
pub struct CaptureFile {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl CaptureFile {
    /// Creates an empty capture file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the bytes written so far as a lossy UTF-8 string.
    #[must_use]
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().expect("capture lock")).into_owned()
    }

    /// Returns the raw bytes written so far.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.buffer.lock().expect("capture lock").clone()
    }
}

impl File for CaptureFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.buffer
            .lock()
            .expect("capture lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn write_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o644))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::MemFs;
    use wanix_vfs::{BindOptions, Namespace};
    use wanix_wasi::WasiConfig;

    use super::{CaptureFile, WasiRunner};

    const RUST_GUEST: &[u8] = include_bytes!("../fixtures/rust-guest.wasm");

    fn namespace_on(fs: &Arc<MemFs>) -> Namespace {
        let mut ns = Namespace::new();
        ns.bind(fs.clone(), ".", ".", BindOptions::default())
            .expect("bind shared fs at root");
        ns
    }

    fn list(fs: &Arc<MemFs>, dir: &str) -> (i32, String) {
        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let config = WasiConfig::new(namespace_on(fs))
            .with_args(["guest", "--list", dir])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        (exit, stdout.contents())
    }

    #[test]
    fn run_in_dir_preopens_host_directory() {
        let dir = std::env::temp_dir().join(format!("wanix-wasm-run-in-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make temp dir");
        std::fs::write(dir.join("in.txt"), b"hello").expect("write in.txt");

        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        // Guest reads /in.txt and writes /out.txt within the preopened host dir.
        let exit = runner
            .run_in_dir(&dir, &["guest", "/in.txt", "/out.txt"])
            .expect("run_in_dir");
        assert_eq!(exit, 0);

        let out = std::fs::read_to_string(dir.join("out.txt")).expect("out.txt on host disk");
        assert_eq!(out, "rust-wasm saw: hello");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fd_readdir_lists_directory_entries() {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("dir/sub").expect("make /dir/sub");
        fs.write_file("dir/alpha.txt", b"a").expect("write alpha");
        fs.write_file("dir/beta.txt", b"bb").expect("write beta");

        let (exit, out) = list(&fs, "/dir");
        assert_eq!(exit, 0, "guest should exit cleanly");

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec![
                "rust-wasm: /dir has 3 entries",
                "alpha.txt file",
                "beta.txt file",
                "sub dir",
            ],
            "unexpected readdir output: {out:?}"
        );
    }

    #[test]
    fn fd_readdir_returns_each_entry_exactly_once_for_many_entries() {
        // A wide directory makes libstd's dirent buffer fill and re-issue
        // fd_readdir with advancing cookies; every entry must appear once with
        // no duplication or omission.
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("dir").expect("make /dir");
        let mut expected: Vec<String> = Vec::new();
        for i in 0..200 {
            let name = format!("entry-{i:04}.txt");
            fs.write_file(format!("dir/{name}"), b"x")
                .expect("write entry");
            expected.push(format!("{name} file"));
        }
        expected.sort();

        let (exit, out) = list(&fs, "/dir");
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");

        let mut rows: Vec<&str> = out
            .lines()
            .filter(|l| !l.starts_with("rust-wasm:"))
            .collect();
        rows.sort_unstable();
        let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
        assert_eq!(rows, expected_refs, "missing/duplicated entries: {out:?}");
        assert!(
            out.contains("/dir has 200 entries"),
            "wrong entry count: {out:?}"
        );
    }

    fn rename(fs: &Arc<MemFs>, src: &str, dst: &str) -> (i32, String) {
        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let config = WasiConfig::new(namespace_on(fs))
            .with_args(["guest", "--rename", src, dst])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        (exit, stdout.contents())
    }

    #[test]
    fn path_rename_moves_file_preserving_bytes() {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("shared").expect("make /shared");
        fs.write_file("shared/a.txt", b"original bytes")
            .expect("seed a.txt");

        let (exit, out) = rename(&fs, "/shared/a.txt", "/shared/b.txt");
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.contains("renamed /shared/a.txt -> /shared/b.txt"),
            "unexpected rename output: {out:?}"
        );

        assert_eq!(
            fs.read_file("shared/b.txt").expect("b.txt exists"),
            b"original bytes",
            "renamed file should keep original bytes"
        );
        assert!(
            fs.read_file("shared/a.txt").is_err(),
            "source a.txt should be gone after rename"
        );
    }

    #[test]
    fn path_rename_missing_source_propagates_error() {
        // Renaming a nonexistent source must surface the Errno as an error line
        // (the guest's Err arm), not panic or trap the runner.
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("shared").expect("make /shared");

        let (exit, out) = rename(&fs, "/shared/missing.txt", "/shared/b.txt");
        assert_eq!(exit, 0, "guest itself exits cleanly: {out:?}");
        assert!(
            out.contains("rename failed /shared/missing.txt -> /shared/b.txt"),
            "expected rename error on missing source, got: {out:?}"
        );
        assert!(
            fs.read_file("shared/b.txt").is_err(),
            "no target should be created on failed rename"
        );
    }

    fn truncate(fs: &Arc<MemFs>, path: &str, len: u64) -> (i32, String) {
        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let len = len.to_string();
        let config = WasiConfig::new(namespace_on(fs))
            .with_args(["guest", "--truncate", path, &len])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        (exit, stdout.contents())
    }

    #[test]
    fn fd_filestat_set_size_shrinks_file() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("data.txt", b"hello world")
            .expect("seed data.txt");

        let (exit, out) = truncate(&fs, "/data.txt", 5);
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.contains("truncated /data.txt to 5"),
            "unexpected truncate output: {out:?}"
        );
        assert_eq!(
            fs.read_file("data.txt").expect("data.txt exists"),
            b"hello",
            "shrunk file should keep only the leading bytes"
        );
    }

    #[test]
    fn fd_filestat_set_size_extends_file_with_zeros() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("data.txt", b"hi").expect("seed data.txt");

        let (exit, out) = truncate(&fs, "/data.txt", 5);
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.contains("truncated /data.txt to 5"),
            "unexpected truncate output: {out:?}"
        );
        assert_eq!(
            fs.read_file("data.txt").expect("data.txt exists"),
            b"hi\0\0\0",
            "extended file should be zero-padded"
        );
    }

    #[test]
    fn fd_filestat_set_size_missing_file_reports_error() {
        // Truncating a path that cannot be opened must surface as the guest's
        // error line, not panic or trap the runner.
        let fs = Arc::new(MemFs::new());

        let (exit, out) = truncate(&fs, "/missing.txt", 4);
        assert_eq!(exit, 0, "guest itself exits cleanly: {out:?}");
        assert!(
            out.contains("truncate failed to open /missing.txt"),
            "expected open error on missing file, got: {out:?}"
        );
    }

    fn tell(fs: &Arc<MemFs>, path: &str, seek: u64) -> (i32, String) {
        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let seek = seek.to_string();
        let config = WasiConfig::new(namespace_on(fs))
            .with_args(["guest", "--tell", path, &seek])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        (exit, stdout.contents())
    }

    #[test]
    fn fd_tell_reports_offset_after_seek() {
        // Seeking to an absolute offset then calling the raw fd_tell syscall must
        // report that exact position (fd_tell, not fd_seek, supplies the answer).
        let fs = Arc::new(MemFs::new());
        fs.write_file("data.txt", b"hello world")
            .expect("seed data.txt");

        let (exit, out) = tell(&fs, "/data.txt", 6);
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.contains("tell /data.txt at 6"),
            "fd_tell should report the post-seek offset, got: {out:?}"
        );
    }

    #[test]
    fn fd_tell_reports_zero_at_file_start() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("data.txt", b"hello world")
            .expect("seed data.txt");

        let (exit, out) = tell(&fs, "/data.txt", 0);
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.contains("tell /data.txt at 0"),
            "fd_tell at start should report offset 0, got: {out:?}"
        );
    }

    fn symlink(fs: &Arc<MemFs>, target: &str, link: &str) -> (i32, String) {
        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let config = WasiConfig::new(namespace_on(fs))
            .with_args(["guest", "--symlink", target, link])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        (exit, stdout.contents())
    }

    fn readlink(fs: &Arc<MemFs>, link: &str) -> (i32, String) {
        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let config = WasiConfig::new(namespace_on(fs))
            .with_args(["guest", "--readlink", link])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        (exit, stdout.contents())
    }

    #[test]
    fn path_symlink_then_readlink_roundtrips_target() {
        use wanix_fs::{FileSystem, FileType, MetadataLookup, NormalizedPath};

        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("shared").expect("make /shared");
        fs.write_file("shared/real.txt", b"payload")
            .expect("seed real.txt");

        let (exit, out) = symlink(&fs, "real.txt", "/shared/link.txt");
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.lines().any(|l| l == "ok"),
            "expected symlink success line, got: {out:?}"
        );

        // The namespace must record a symlink (not following it when stat'd).
        let link_path = NormalizedPath::new("shared/link.txt").expect("valid path");
        let meta = fs
            .metadata_with_lookup(&link_path, MetadataLookup::NoFollow)
            .expect("stat link");
        assert_eq!(
            meta.file_type(),
            FileType::Symlink,
            "link.txt should be a symlink in the namespace"
        );

        let (exit, out) = readlink(&fs, "/shared/link.txt");
        assert_eq!(exit, 0, "guest should exit cleanly: {out:?}");
        assert!(
            out.lines().any(|l| l == "real.txt"),
            "readlink should print the stored target, got: {out:?}"
        );
    }

    #[test]
    fn path_readlink_on_regular_file_reports_error() {
        // Reading a link on a non-symlink path must surface as the guest's error
        // line (Preview1 EINVAL), not panic or trap the runner.
        let fs = Arc::new(MemFs::new());
        fs.write_file("plain.txt", b"hi").expect("seed plain.txt");

        let (exit, out) = readlink(&fs, "/plain.txt");
        assert_eq!(exit, 0, "guest itself exits cleanly: {out:?}");
        assert!(
            out.contains("readlink failed /plain.txt"),
            "expected readlink error on a regular file, got: {out:?}"
        );
    }

    #[test]
    fn fd_readdir_on_regular_file_reports_error() {
        // Calling readdir on a non-directory fd must surface as an error
        // (Preview1 ENOTDIR) rather than succeeding or trapping.
        let fs = Arc::new(MemFs::new());
        fs.write_file("file.txt", b"hi").expect("write file");

        let (exit, out) = list(&fs, "/file.txt");
        assert_eq!(exit, 0, "guest itself exits cleanly: {out:?}");
        assert!(
            out.contains("could not list /file.txt"),
            "expected readdir error on a regular file, got: {out:?}"
        );
    }
}
