//! The compiled `wasm32-wasi` command runner.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wanix_fs::LocalFs;
use wanix_vfs::{BindOptions, Namespace};
use wanix_wasi::{WasiConfig, WasiCtx};
use wasmtime::error::Context as _;
use wasmtime::{Config, Engine, Error, Linker, Module, Result, Store};

use crate::capture::{host_stderr, host_stdout};
use crate::state::WasiState;

/// A compiled `wasm32-wasi` command module ready to run as Wanix tasks.
pub struct WasiRunner {
    engine: Engine,
    module: Module,
    interrupted: Arc<AtomicBool>,
}

/// A clonable handle that interrupts every guest currently running on one
/// [`WasiRunner`] by bumping the engine's Wasmtime epoch (the ADR 0010
/// `#task/<id>/ctl kill` seam). The Wanix wasm driver builds one runner per
/// task run, so interrupting it kills exactly that task's guest.
///
/// The epoch trips inside guest *code*: a guest parked in a blocking host
/// import returns to the host's control first and traps on its next entry
/// into guest execution.
#[derive(Clone)]
pub struct EpochInterrupter {
    engine: Engine,
    interrupted: Arc<AtomicBool>,
}

impl EpochInterrupter {
    /// Interrupts the runner's running guests: any in-flight or future
    /// [`WasiRunner::run`] on the same runner traps with an epoch interrupt.
    pub fn interrupt(&self) {
        // Flag first: `run` checks the flag after arming its store deadline,
        // so an interrupt the flag-check misses must have bumped the epoch
        // after the deadline was armed — and therefore trips it.
        self.interrupted.store(true, Ordering::SeqCst);
        self.engine.increment_epoch();
    }
}

/// Engine configuration for command guests: epoch interruption is enabled so
/// a running task can be killed (`#task/<id>/ctl kill`).
fn command_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.epoch_interruption(true);
    Engine::new(&config)
}

impl WasiRunner {
    /// Compiles a `wasm32-wasi` module from bytes with a default engine.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime cannot compile the bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let engine = command_engine()?;
        let module = Module::new(&engine, bytes).context("failed to compile wasm module")?;
        Ok(Self::new(engine, module))
    }

    fn new(engine: Engine, module: Module) -> Self {
        Self {
            engine,
            module,
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the kill handle for guests run by this runner.
    #[must_use]
    pub fn interrupter(&self) -> EpochInterrupter {
        EpochInterrupter {
            engine: self.engine.clone(),
            interrupted: Arc::clone(&self.interrupted),
        }
    }

    /// Compiles a `wasm32-wasi` module from bytes, loading a cached compiled
    /// artifact from `cache_dir` when one is present and trusted.
    ///
    /// Cranelift-compiling a non-trivial guest costs tens to hundreds of
    /// milliseconds; the cache stores the Wasmtime serialized module keyed by
    /// `sha256(bytes)` and deserializes it in well under a millisecond on a warm
    /// run. The cache is advisory: a missing, stale, or *untrusted* artifact
    /// falls back to a fresh compile.
    ///
    /// `cache_dir` must be owner-private or it is ignored — the artifact is
    /// loaded through `unsafe Module::deserialize`, so a hostile (e.g.
    /// world-writable or symlinked) directory or artifact is never deserialized.
    /// Use [`module_cache_dir`](crate::module_cache_dir) for the owner-private
    /// per-user default.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime cannot compile the bytes on a cache miss.
    pub fn from_bytes_cached(bytes: &[u8], cache_dir: &Path) -> Result<Self> {
        use sha2::{Digest, Sha256};

        let engine = command_engine()?;
        let wasm_sha256: [u8; 32] = Sha256::digest(bytes).into();
        let module =
            wanix_module_cache::load_or_compile(&engine, bytes, &wasm_sha256, cache_dir)
                .map_err(|err| Error::msg(format!("failed to compile wasm module: {err:#}")))?;
        Ok(Self::new(engine, module))
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
        // One epoch tick kills the guest. Arm the deadline BEFORE checking the
        // interrupt flag: an interrupt the check misses then necessarily bumped
        // the epoch after the deadline was armed, so it still trips.
        store.set_epoch_deadline(1);
        store.epoch_deadline_trap();
        if self.interrupted.load(Ordering::SeqCst) {
            return Err(Error::msg("wasm task interrupted before start"));
        }

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs, NormalizedPath};
    use wanix_vfs::{BindOptions, Namespace};
    use wanix_wasi::WasiConfig;

    use crate::CaptureFile;
    use crate::WasiRunner;

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

    fn unique_cache_dir(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "wanix-wasm-cache-test-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn from_bytes_cached_warms_then_reuses_artifact() {
        let dir = unique_cache_dir("warm");

        // Cold run: compiles and writes the serialized artifact.
        let runner = WasiRunner::from_bytes_cached(RUST_GUEST, &dir).expect("cold compile");
        let exit = runner
            .run(
                WasiConfig::new(namespace_on(&Arc::new(MemFs::new())))
                    .with_args(["guest", "--echo", "hi"]),
            )
            .expect("cold run");
        assert_eq!(exit, 0);

        // An artifact file must now exist in the cache directory.
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("cache dir exists")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "cwasm"))
            .collect();
        assert_eq!(entries.len(), 1, "exactly one cached artifact expected");

        // Warm run: the same cache directory is reused and still runs correctly.
        let warm = WasiRunner::from_bytes_cached(RUST_GUEST, &dir).expect("warm load");
        let exit = warm
            .run(
                WasiConfig::new(namespace_on(&Arc::new(MemFs::new())))
                    .with_args(["guest", "--echo", "hi"]),
            )
            .expect("warm run");
        assert_eq!(exit, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn from_bytes_cached_ignores_world_writable_cache_dir() {
        use std::os::unix::fs::PermissionsExt;

        // A world-writable cache dir is a trust-boundary violation: a peer could
        // pre-seed a hostile artifact. The cache layer must refuse to trust it,
        // so the runner falls back to a fresh compile and still runs correctly.
        let dir = unique_cache_dir("hostile");
        std::fs::create_dir_all(&dir).expect("make hostile dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("chmod 0777");

        let runner =
            WasiRunner::from_bytes_cached(RUST_GUEST, &dir).expect("compile despite hostile dir");
        let exit = runner
            .run(
                WasiConfig::new(namespace_on(&Arc::new(MemFs::new())))
                    .with_args(["guest", "--echo", "hi"]),
            )
            .expect("runs from fresh compile");
        assert_eq!(exit, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn from_bytes_cached_rejects_symlinked_artifact() {
        // Warm a valid artifact, then replace it with a symlink to that artifact.
        // `O_NOFOLLOW` on the final component must refuse it (cache miss), so the
        // runner recompiles cleanly rather than deserializing through a symlink.
        let dir = unique_cache_dir("symlink");
        WasiRunner::from_bytes_cached(RUST_GUEST, &dir).expect("warm compile");

        let artifact = std::fs::read_dir(&dir)
            .expect("cache dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "cwasm"))
            .expect("artifact written");
        let real = dir.join("real.cwasm");
        std::fs::rename(&artifact, &real).expect("move real artifact aside");
        std::os::unix::fs::symlink(&real, &artifact).expect("symlink in place of artifact");

        // Still compiles + runs; the symlinked artifact is never deserialized.
        let runner =
            WasiRunner::from_bytes_cached(RUST_GUEST, &dir).expect("recompile on rejection");
        let exit = runner
            .run(
                WasiConfig::new(namespace_on(&Arc::new(MemFs::new())))
                    .with_args(["guest", "--echo", "hi"]),
            )
            .expect("runs from fresh compile");
        assert_eq!(exit, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_filestat_set_times_sets_mtime() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("f.txt", b"x").expect("write f.txt");

        let runner = WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest");
        let stdout = CaptureFile::new();
        let config = WasiConfig::new(namespace_on(&fs))
            .with_args(["guest", "--utime", "/f.txt", "123456789"])
            .with_stdout(Box::new(stdout.clone()), "stdout");
        let exit = runner.run(config).expect("rust wasm task ran");
        assert_eq!(exit, 0);
        assert!(stdout.contents().contains("ok"), "{}", stdout.contents());

        let meta = fs
            .metadata(&NormalizedPath::new("f.txt").unwrap())
            .expect("metadata");
        assert_eq!(meta.modified_time_ns(), 123_456_789);
    }

    #[test]
    fn run_in_dir_preopens_host_directory() {
        let dir =
            std::env::temp_dir().join(format!("wanix-wasm-run-in-dir-{}", std::process::id()));
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
