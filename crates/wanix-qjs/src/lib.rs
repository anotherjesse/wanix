//! QuickJS/WASI task driver for Rust Wanix.
//!
//! This crate adapts the workspace QuickJS engine crate into a Wanix task
//! driver. QuickJS runs inside a WASI reactor hosted by Wasmtime; Wanix supplies
//! the namespace, fds, module loading policy, and snapshot reattachment policy
//! through live WASI providers and task-owned host resources.

use std::fmt;
use std::path::Path;

use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsHostConfig, QuickJsModule};
use wanix_fs::{FsError, FsResult};
use wanix_wasi::WasiConfig;

mod bundled;
mod driver;
mod fd_api;
mod host_api;
mod runner;
mod runtime_control;
mod task_command;
mod task_context;
mod task_runtime;
mod task_runtime_attach;
mod task_stdio;
mod wanix_runtime;
mod wasi_host;

pub use bundled::bundled_module_cache_dir;
pub use driver::QuickJsTaskDriver;
pub use runner::RunOutput;
pub use task_runtime::QuickJsTaskRuntime;

#[cfg(test)]
use task_command::task_env_map;
use wasi_host::WanixQuickJsWasiHost;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "quickjs wasi task driver";

/// First externally visible demo target for this crate.
pub const FIRST_DEMO_TARGET: &str =
    "run JavaScript outside Chrome with access to a Wanix namespace";

/// Wanix-owned configuration for QuickJS task execution.
#[derive(Debug, Clone)]
pub struct QuickJsWanixConfig {
    wasi: WasiConfig,
}

impl QuickJsWanixConfig {
    /// Creates a QuickJS/Wanix config from Wanix-backed WASI settings.
    #[must_use]
    pub fn new(wasi: WasiConfig) -> Self {
        Self { wasi }
    }

    /// Returns the Wanix-backed WASI settings.
    #[must_use]
    pub fn wasi(&self) -> &WasiConfig {
        &self.wasi
    }
}

/// QuickJS runtime runner backed by the workspace engine crate.
pub struct QuickJsRunner {
    module: QuickJsModule,
}

impl fmt::Debug for QuickJsRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuickJsRunner").finish_non_exhaustive()
    }
}

impl QuickJsRunner {
    /// Loads a QuickJS WASM module from disk.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the module cannot be compiled.
    pub fn from_wasm_file(path: impl AsRef<Path>) -> FsResult<Self> {
        let module = QuickJsModule::from_file_with_default_engine(path)
            .map_err(|err| FsError::Other(format!("failed to load QuickJS wasm: {err:#}")))?;
        Ok(Self { module })
    }

    /// Compiles a QuickJS WASM module from in-memory bytes.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the module cannot be compiled.
    pub fn from_wasm_bytes(bytes: impl AsRef<[u8]>) -> FsResult<Self> {
        let module = QuickJsModule::from_bytes_with_default_engine(bytes.as_ref())
            .map_err(|err| FsError::Other(format!("failed to load QuickJS wasm: {err:#}")))?;
        Ok(Self { module })
    }

    /// Loads the workspace-local QuickJS WASM fixture bundled by the engine crate.
    ///
    /// The compiled module is cached on disk under [`bundled_module_cache_dir`]
    /// so repeated cold starts deserialize the prebuilt artifact instead of
    /// Cranelift-compiling the ~1.7 MiB fixture (~550 ms) on every process. The
    /// cache is advisory and the cache directory is verified owner-private
    /// before any artifact is trusted, so a missing or untrusted cache only
    /// forfeits the speedup.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the bundled module cannot be compiled.
    pub fn from_bundled_wasm() -> FsResult<Self> {
        let module = QuickJsModule::from_bytes_with_default_engine_cached(
            rust_wasi_quickjs::QUICKJS_WASM_FIXTURE,
            &bundled_module_cache_dir(),
        )
        .map_err(|err| FsError::Other(format!("failed to load QuickJS wasm: {err:#}")))?;
        Ok(Self { module })
    }
}

/// Returns the WASI crate purpose, proving the intended dependency edge.
#[must_use]
pub fn wasi_contract_purpose() -> &'static str {
    wanix_wasi::CRATE_PURPOSE
}

const CONSOLE_PRELUDE: &str = r#"
globalThis.print = (...args) => __wanix_stdout(args.map(String).join(" ") + "\n");
globalThis.console = {
  log: (...args) => __wanix_stdout(args.map(String).join(" ") + "\n"),
  error: (...args) => __wanix_stderr(args.map(String).join(" ") + "\n"),
};
"#;

fn captured_stdio_config() -> QuickJsHostConfig {
    QuickJsHostConfig::new()
        .with_stdout_capture(true)
        .with_stderr_capture(true)
}

fn captured_stdio_config_for_wasi(config: &WasiConfig) -> QuickJsHostConfig {
    captured_stdio_config().with_clock_time_ns(config.clock_time_ns())
}

fn captured_stdio_options() -> QuickJsCreateOptions {
    create_options_with_config(captured_stdio_config())
}

fn create_options_with_config(config: QuickJsHostConfig) -> QuickJsCreateOptions {
    QuickJsCreateOptions::new().with_host_config(config)
}

fn captured_stdio_options_with_wanix_wasi(
    config: QuickJsWanixConfig,
) -> FsResult<QuickJsCreateOptions> {
    let host_config = captured_stdio_config_for_wasi(config.wasi());
    Ok(create_options_with_config(host_config).with_wasi_host(wanix_wasi_host(config)?))
}

fn wanix_wasi_host(config: QuickJsWanixConfig) -> FsResult<WanixQuickJsWasiHost> {
    WanixQuickJsWasiHost::new(config.wasi).map_err(wanix_wasi_host_error)
}

fn wanix_wasi_host_error(err: wanix_wasi::Errno) -> FsError {
    FsError::Other(format!(
        "failed to create Wanix-backed QuickJS WASI host: {err:?}"
    ))
}

fn uses_module_syntax(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("export ")
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use super::{
        CRATE_PURPOSE, FIRST_DEMO_TARGET, QuickJsHostConfig, QuickJsRunner, QuickJsTaskDriver,
        QuickJsWanixConfig, wasi_contract_purpose,
    };
    use crate::task_command::task_command;
    use crate::task_stdio::task_wasi_config;
    #[cfg(unix)]
    use wanix_fs::LocalFs;
    use wanix_fs::{FileSystem, FileType, FsError, MemFs, NormalizedPath, OpenOptions};
    use wanix_task::{Fd, Task, TaskDriver, TaskId, TaskSpec, TaskTable};
    use wanix_vfs::{BindOptions, Namespace};
    use wanix_wasi::{Errno, WasiConfig, WasiCtx, WasiFd, WasiOpenOptions};

    fn runner() -> Arc<QuickJsRunner> {
        static RUNNER: OnceLock<Result<Arc<QuickJsRunner>, String>> = OnceLock::new();
        match RUNNER.get_or_init(|| {
            QuickJsRunner::from_bundled_wasm()
                .map(Arc::new)
                .map_err(|err| err.to_string())
        }) {
            Ok(runner) => Arc::clone(runner),
            Err(error) => panic!("failed to load bundled QuickJS wasm: {error}"),
        }
    }

    fn read_file(fs: &dyn FileSystem, path: &str) -> Vec<u8> {
        let mut file = fs
            .open(&NormalizedPath::new(path).unwrap(), OpenOptions::read())
            .unwrap();
        let mut out = Vec::new();
        let mut buf = [0; 16];
        loop {
            let n = file.read(&mut buf).unwrap();
            if n == 0 {
                return out;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    #[cfg(unix)]
    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after epoch")
            .as_nanos();
        path.push(format!("{prefix}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn quickjs_wanix_config_wraps_wanix_wasi_settings() {
        let config = QuickJsWanixConfig::new(WasiConfig::default());

        assert_eq!(config.wasi().preopens()[0].guest_path().as_str(), ".");
        assert_eq!(wasi_contract_purpose(), "wanix-backed wasi imports");
        assert!(FIRST_DEMO_TARGET.contains("outside Chrome"));
    }

    #[test]
    fn qjs_and_wanix_wasi_default_clocks_match() {
        assert_eq!(
            QuickJsHostConfig::new().clock_time_ns(),
            wanix_wasi::DEFAULT_CLOCK_TIME_NS
        );
    }

    #[test]
    fn runner_executes_source_and_captures_console_output() {
        let output = runner()
            .run_source(r#"print("hello", 42); console.error("oops");"#)
            .unwrap();

        assert_eq!(output.stdout(), b"hello 42\n");
        assert_eq!(output.stderr(), b"oops\n");
    }

    #[test]
    fn runner_uses_quickjs_wanix_config_for_namespace_access() {
        let mut namespace = wanix_vfs::Namespace::new();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("input.txt", b"from config").unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let config = QuickJsWanixConfig::new(WasiConfig::new(namespace));

        let output = runner()
            .run_source_with_wanix_config(
                r#"
import * as std from "qjs:std";

const text = std.loadFile("input.txt");
std.writeFile("output.txt", text + " / qjs");
"#,
                config,
            )
            .unwrap();

        assert!(output.stdout().is_empty());
        assert_eq!(read_file(&*root, "output.txt"), b"from config / qjs");
    }

    #[test]
    fn runner_wanix_config_mirrors_wasi_clock_into_quickjs_host_config() {
        let mut namespace = wanix_vfs::Namespace::new();
        namespace
            .bind(
                std::sync::Arc::new(MemFs::new()),
                ".",
                ".",
                BindOptions::default(),
            )
            .unwrap();
        let config =
            QuickJsWanixConfig::new(WasiConfig::new(namespace).with_clock_time_ns(12_345_000_000));

        let output = runner()
            .run_source_with_wanix_config(r#"print("date", Date.now());"#, config)
            .unwrap();

        assert_eq!(output.stdout(), b"date 12345\n");
    }

    #[test]
    fn task_runtime_options_mirror_wasi_clock_into_quickjs_host_config() {
        let create_options = crate::task_runtime_attach::task_create_options(
            WasiConfig::default().with_clock_time_ns(12_345_000_000),
            crate::task_context::WanixExitState::default(),
        )
        .unwrap();
        assert_eq!(create_options.host_config().clock_time_ns(), 12_345_000_000);
        let mut runtime = runner()
            .module
            .create_runtime_with_options(create_options)
            .unwrap();

        runtime
            .eval_discard("globalThis.beforeClock = Date.now();")
            .unwrap();
        assert_eq!(runtime.eval_number("Date.now()").unwrap(), 12_345.0);
        let snapshot_bytes = runtime.snapshot().unwrap().try_to_bytes().unwrap();

        let restore_options = crate::task_runtime_attach::task_restore_options(
            WasiConfig::default().with_clock_time_ns(67_890_000_000),
            crate::task_context::WanixExitState::default(),
        )
        .unwrap();
        assert_eq!(
            restore_options.host_config().clock_time_ns(),
            67_890_000_000
        );
        let mut restored = runner()
            .module
            .restore_runtime_from_bytes_with_options(&snapshot_bytes, restore_options)
            .unwrap();

        assert_eq!(
            restored.eval_number("globalThis.beforeClock").unwrap(),
            12_345.0
        );
        assert_eq!(restored.eval_number("Date.now()").unwrap(), 67_890.0);
    }

    #[test]
    fn runner_wanix_config_uses_live_wasi_host_without_flattening_preopens() {
        let mut namespace = wanix_vfs::Namespace::new();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("input.txt", b"from config").unwrap();
        root.write_file("mnt/extra.txt", b"from extra preopen")
            .unwrap();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        let config =
            QuickJsWanixConfig::new(WasiConfig::new(namespace).with_preopen("mnt").unwrap());

        let output = runner()
            .run_source_with_wanix_config(
                r#"
import * as std from "qjs:std";

std.writeFile("observed.txt", std.loadFile("input.txt"));
"#,
                config,
            )
            .unwrap();

        assert!(output.stdout().is_empty());
        assert_eq!(read_file(&*root, "observed.txt"), b"from config");
    }

    #[test]
    fn task_wasi_config_attaches_open_task_standard_fds() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from namespace").unwrap();
        let stdin = std::sync::Arc::new(MemFs::new());
        stdin.write_file("stdin", b"input").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("stdout", b"").unwrap();
        let stderr = std::sync::Arc::new(MemFs::new());
        stderr.write_file("stderr", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDIN,
            stdin
                .open(&NormalizedPath::new("stdin").unwrap(), OpenOptions::read())
                .unwrap(),
            NormalizedPath::new("stdin").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("stdout").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("stdout").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDERR,
            stderr
                .open(
                    &NormalizedPath::new("stderr").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("stderr").unwrap(),
        )
        .unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        assert_eq!(
            task_wasi_config(&task).stdio_fds(),
            vec![WasiFd::STDIN, WasiFd::STDOUT, WasiFd::STDERR]
        );
        let mut buf = [0; 16];
        let count = ctx.fd_read(WasiFd::STDIN, &mut buf).unwrap();
        assert_eq!(&buf[..count], b"input");
        assert_eq!(ctx.fd_write(WasiFd::STDIN, b"nope"), Err(Errno::Notcapable));
        assert_eq!(
            ctx.fd_read(WasiFd::STDOUT, &mut [0; 1]),
            Err(Errno::Notcapable)
        );
        assert_eq!(
            ctx.fd_read(WasiFd::STDERR, &mut [0; 1]),
            Err(Errno::Notcapable)
        );
        assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"out").unwrap(), 3);
        assert_eq!(ctx.fd_write(WasiFd::STDERR, b"err").unwrap(), 3);
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();

        assert_eq!(fd.get(), 4);
        assert_eq!(stdout.read_file("stdout").unwrap(), b"out");
        assert_eq!(stderr.read_file("stderr").unwrap(), b"err");
        assert_eq!(ctx.fd_close(WasiFd::STDOUT), Err(Errno::Badf));
    }

    #[test]
    fn task_wasi_config_leaves_unopened_standard_fds_closed() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from namespace").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));

        assert!(task_wasi_config(&task).stdio_fds().is_empty());
        assert_eq!(ctx.fd_read(WasiFd::STDIN, &mut [0; 1]), Err(Errno::Badf));
        assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"out"), Err(Errno::Badf));
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();
        assert_eq!(fd.get(), 4);
    }

    #[test]
    fn task_wasi_config_flows_task_cmd_and_env_to_wasi_process_state() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        task.set_cmd("main.js --mode test").unwrap();
        task.set_env_lines("MODE=test\nEMPTY=").unwrap();

        let ctx = WasiCtx::new(task_wasi_config(&task));

        assert_eq!(ctx.args(), ["main.js", "--mode", "test"]);
        assert_eq!(ctx.env(), ["MODE=test", "EMPTY="]);
    }

    #[test]
    fn task_spec_preserves_exact_qjs_program_args() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("app/main.js", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js two words").unwrap();
        task.set_dir("app").unwrap();
        let mut spec = TaskSpec::new("main.js").unwrap();
        spec.args = vec!["two words".to_owned(), "".to_owned(), "--flag".to_owned()];
        spec.env.insert("EMPTY".to_owned(), String::new());
        spec.env.insert("MODE".to_owned(), "test".to_owned());
        spec.cwd = NormalizedPath::new("app").unwrap();
        task.set_spec(spec).unwrap();
        task.set_env_lines("RAW=1").unwrap();

        let command = task_command(&task).unwrap();
        let ctx = WasiCtx::new(task_wasi_config(&task));

        assert_eq!(command.raw, "main.js two words");
        assert_eq!(command.program.as_str(), "app/main.js");
        assert_eq!(command.args, ["two words", "", "--flag"]);
        assert_eq!(command.cwd.as_str(), "app");
        assert_eq!(ctx.args(), ["main.js", "two words", "", "--flag"]);
        assert_eq!(ctx.env(), ["EMPTY=", "MODE=test"]);
        assert_eq!(super::task_env_map(&task)["MODE"], "test");
        assert!(!super::task_env_map(&task).contains_key("RAW"));
    }

    #[test]
    fn raw_task_cmd_preserves_quoted_qjs_program_args() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("app/main.js", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js alpha 'two words' '' plain\\ arg")
            .unwrap();
        task.set_env_lines("MODE=test\nEMPTY=").unwrap();
        task.set_dir("app").unwrap();

        let command = task_command(&task).unwrap();
        let ctx = WasiCtx::new(task_wasi_config(&task));

        assert_eq!(command.raw, "main.js alpha 'two words' '' plain\\ arg");
        assert_eq!(command.program.as_str(), "app/main.js");
        assert_eq!(command.args, ["alpha", "two words", "", "plain arg"]);
        assert_eq!(
            ctx.args(),
            ["main.js", "alpha", "two words", "", "plain arg"]
        );
        assert_eq!(ctx.env(), ["MODE=test", "EMPTY="]);
        assert_eq!(super::task_env_map(&task)["MODE"], "test");
    }

    #[test]
    fn task_wasi_config_maps_root_preopen_to_task_spec_cwd() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from root").unwrap();
        root.write_file("app/data.txt", b"from app").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        let mut spec = TaskSpec::new("main.js").unwrap();
        spec.cwd = NormalizedPath::new("app").unwrap();
        task.set_spec(spec).unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();
        let mut buf = [0; 16];
        let count = ctx.fd_read(fd, &mut buf).unwrap();

        assert_eq!(ctx.fd_prestat_get(WasiFd::ROOT).unwrap().dir_name(), "/");
        assert_eq!(&buf[..count], b"from app");
    }

    #[test]
    fn task_wasi_config_ignores_non_standard_task_fds() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from namespace").unwrap();
        let fd3 = std::sync::Arc::new(MemFs::new());
        fd3.write_file("fd3", b"task fd three").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::new(3),
            fd3.open(&NormalizedPath::new("fd3").unwrap(), OpenOptions::read())
                .unwrap(),
            NormalizedPath::new("fd3").unwrap(),
        )
        .unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));

        assert!(task_wasi_config(&task).stdio_fds().is_empty());
        assert_eq!(ctx.fd_read(WasiFd::ROOT, &mut [0; 4]), Err(Errno::Isdir));
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();
        assert_eq!(fd, WasiFd::new(4));
    }

    #[test]
    fn task_wasi_config_mirrors_dynamic_file_fds_into_task_table() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from mirrored fd").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();

        assert_eq!(fd, WasiFd::new(4));
        assert_eq!(task.fd_numbers(), [Fd::new(4)]);
        let mut buf = [0; 32];
        let count = task.read_fd(Fd::new(4), &mut buf).unwrap();
        assert_eq!(&buf[..count], b"from mirrored fd");
        assert_eq!(ctx.fd_tell(fd).unwrap(), 16);

        ctx.fd_close(fd).unwrap();
        assert!(task.fd_numbers().is_empty());
    }

    #[test]
    fn task_wasi_config_skips_existing_task_fd_when_mirroring_dynamic_file_fd() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from mirrored fd").unwrap();
        let existing = std::sync::Arc::new(MemFs::new());
        existing.write_file("existing.txt", b"existing fd").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::new(4),
            existing
                .open(
                    &NormalizedPath::new("existing.txt").unwrap(),
                    OpenOptions::read(),
                )
                .unwrap(),
            NormalizedPath::new("existing.txt").unwrap(),
        )
        .unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();

        assert_eq!(fd, WasiFd::new(5));
        assert_eq!(task.fd_numbers(), [Fd::new(4), Fd::new(5)]);
        assert_eq!(task.fd_path(Fd::new(4)).unwrap().as_str(), "existing.txt");
        assert_eq!(task.fd_path(Fd::new(5)).unwrap().as_str(), "data.txt");
        ctx.fd_close(fd).unwrap();
        assert_eq!(task.fd_numbers(), [Fd::new(4)]);
    }

    #[test]
    fn task_wasi_config_cleans_mirrored_dynamic_fds_on_ctx_drop() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("data.txt", b"from mirrored fd").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();

        {
            let mut ctx = WasiCtx::new(task_wasi_config(&task));
            let fd = ctx
                .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
                .unwrap();
            assert_eq!(fd, WasiFd::new(4));
            assert_eq!(task.fd_numbers(), [Fd::new(4)]);
        }

        assert!(task.fd_numbers().is_empty());
    }

    #[test]
    fn task_wasi_config_tracks_live_task_standard_fd_state() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("stdout", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("stdout").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("stdout").unwrap(),
        )
        .unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        task.close_fd(Fd::STDOUT).unwrap();

        assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"after"), Err(Errno::Badf));
        assert_eq!(stdout.read_file("stdout").unwrap(), b"");
    }

    #[test]
    fn task_wasi_config_tracks_live_task_standard_fd_replacement() {
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        let old_stdout = std::sync::Arc::new(MemFs::new());
        old_stdout.write_file("old", b"").unwrap();
        let new_stdout = std::sync::Arc::new(MemFs::new());
        new_stdout.write_file("new", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            old_stdout
                .open(
                    &NormalizedPath::new("old").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("old").unwrap(),
        )
        .unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        task.insert_fd(
            Fd::STDOUT,
            new_stdout
                .open(
                    &NormalizedPath::new("new").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("new").unwrap(),
        )
        .unwrap();

        assert_eq!(ctx.fd_write(WasiFd::STDOUT, b"after").unwrap(), 5);
        assert_eq!(old_stdout.read_file("old").unwrap(), b"");
        assert_eq!(new_stdout.read_file("new").unwrap(), b"after");
    }

    #[test]
    fn runner_loads_es_modules_from_wanix_namespace() {
        let mut namespace = wanix_vfs::Namespace::new();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "lib.js",
            b"import { suffix } from './nested/suffix.js'; export const message = `from namespace ${suffix}`;",
        )
        .unwrap();
        root.write_file("nested/suffix.js", b"export const suffix = 'modules';")
            .unwrap();
        namespace
            .bind(root, ".", ".", BindOptions::default())
            .unwrap();

        let config = QuickJsWanixConfig::new(WasiConfig::new(namespace));
        let mut runtime = runner().create_runtime_with_wanix_config(config).unwrap();

        runtime
            .eval_module_discard(
                r#"
import { message } from "./lib.js";

globalThis.loadedMessage = message;
"#,
                "main.js",
            )
            .unwrap();

        assert_eq!(
            runtime.eval_string("globalThis.loadedMessage").unwrap(),
            "from namespace modules"
        );
    }

    #[test]
    fn task_driver_loads_script_from_namespace_and_writes_task_fds() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br##"
import * as std from "qjs:std";

const text = std.loadFile("input.txt");
std.writeFile("generated.txt", text + " via task");
std.out.puts("task " + std.loadFile("generated.txt") + "\n");
std.err.puts("stderr line\n");
std.out.flush();
std.err.flush();
"##,
        )
        .unwrap();
        root.write_file("input.txt", b"namespace").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        let stderr = std::sync::Arc::new(MemFs::new());
        stderr.write_file("err", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDERR,
            stderr
                .open(
                    &NormalizedPath::new("err").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("err").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"task namespace via task\n");
        assert_eq!(read_file(&*stderr, "err"), b"stderr line\n");
        assert_eq!(read_file(&*root, "generated.txt"), b"namespace via task");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_releases_fds_on_exit_so_a_piped_consumer_sees_eof() {
        // task-exit-closes-fds for the qjs driver: a qjs producer whose fd 1 is
        // a #pipe write end must drop that writer when it exits, or a pipeline
        // consumer blocks forever waiting for EOF.
        let table = TaskTable::new();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner())))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br##"
import * as std from "qjs:std";
std.out.puts("piped bytes");
std.out.flush();
"##,
        )
        .unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();

        let pipe = wanix_pipe::PipeDevice::new();
        let id = pipe.alloc().unwrap();
        let data = NormalizedPath::new(format!("{id}/data")).unwrap();
        let writer = pipe
            .open(
                &data,
                wanix_fs::OpenOptions {
                    write: true,
                    ..wanix_fs::OpenOptions::default()
                },
            )
            .unwrap();
        task.insert_fd(Fd::STDOUT, writer, data.clone()).unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();
        assert_eq!(task.exit(), "0");
        assert!(
            task.fd_numbers().is_empty(),
            "an exited qjs task holds no descriptors"
        );

        // The reader drains the bytes and then observes EOF — which only
        // happens because the producer's writer fd was released on exit.
        let mut reader = pipe.open(&data, OpenOptions::read()).unwrap();
        let mut collected = Vec::new();
        let mut buf = [0u8; 16];
        loop {
            let n = reader.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            collected.extend_from_slice(&buf[..n]);
        }
        assert_eq!(collected, b"piped bytes");
    }

    #[test]
    fn task_driver_loads_es_modules_from_namespace() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import { message } from "./data/message.js";

print(message);
"#,
        )
        .unwrap();
        root.write_file(
            "data/message.js",
            b"export const message = 'loaded as Wanix module';",
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"loaded as Wanix module\n");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_exposes_task_context_through_wasi_and_service_files() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "app/main.js",
            br##"
import * as std from "qjs:std";

const env = std.getenviron();
std.out.puts("cmd " + std.loadFile("#task/self/cmd").trim() + "\n");
std.out.puts("cwd " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("args " + scriptArgs.slice(1).join("|") + "\n");
std.out.puts("env " + std.getenv("MODE") + " " + (env.EMPTY === "") + " " + String(env.MISSING) + "\n");
const source = std.loadFile("main.js");
std.writeFile("out.txt", "cwd write");
std.out.puts("source " + source.includes("std.loadFile") + "\n");
std.out.puts("id " + std.loadFile("#task/self/id").trim() + "\n");
std.out.flush();
"##,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js alpha beta").unwrap();
        task.set_env_lines("MODE=test\nEMPTY=").unwrap();
        task.set_dir("app").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"cmd main.js alpha beta\ncwd app\nargs alpha|beta\nenv test true undefined\nsource true\nid 1\n"
        );
        assert_eq!(read_file(&*root, "app/out.txt"), b"cwd write");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_maps_quickjs_std_exit_to_task_status() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";

std.out.puts("before std exit\n");
std.out.flush();
std.err.puts("stderr before std exit\n");
std.err.flush();
std.exit(7);
std.out.puts("after std exit\n");
std.err.puts("stderr after std exit\n");
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        let stderr = std::sync::Arc::new(MemFs::new());
        stderr.write_file("err", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDERR,
            stderr
                .open(
                    &NormalizedPath::new("err").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("err").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"before std exit\n");
        assert_eq!(read_file(&*stderr, "err"), b"stderr before std exit\n");
        assert_eq!(task.exit(), "7");
    }

    #[test]
    fn task_driver_quickjs_os_mirrors_dynamic_fds_into_wanix_task_table() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "app/main.js",
            br##"
import * as os from "qjs:os";
import * as std from "qjs:std";

const stringFromBytes = (bytes, count) =>
  Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");

const input = os.open("input.txt", os.O_RDONLY);
std.out.puts("input fd " + input + "\n");
const inputBytes = new Uint8Array(64);
const inputCount = os.read(input, inputBytes.buffer, 0, inputBytes.length);
std.out.puts("read " + stringFromBytes(inputBytes, inputCount) + "\n");
os.close(input);

const output = os.open("created.txt", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
std.out.puts("output fd " + output + "\n");
const outputBytes = Uint8Array.from(Array.from("created via fd").map((char) => char.charCodeAt(0)));
std.out.puts("wrote " + os.write(output, outputBytes.buffer, 0, outputBytes.length) + "\n");
os.close(output);
std.out.flush();
"##,
        )
        .unwrap();
        root.write_file("app/input.txt", b"from fd").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();
        task.set_dir("app").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"input fd 4\nread from fd\noutput fd 5\nwrote 14\n"
        );
        assert_eq!(
            root.read_file("app/created.txt").unwrap(),
            b"created via fd"
        );
        // task-exit-closes-fds: an exited task holds no descriptors, stdio
        // included (its output lives in the fd's backing, read above).
        assert_eq!(task.fd_numbers(), []);
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_std_writes_to_wanix_stdio_fds() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";

std.out.puts("std stdout\n");
std.out.flush();
std.err.puts("std stderr\n");
std.err.flush();
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        let stderr = std::sync::Arc::new(MemFs::new());
        stderr.write_file("err", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDERR,
            stderr
                .open(
                    &NormalizedPath::new("err").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("err").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"std stdout\n");
        assert_eq!(read_file(&*stderr, "err"), b"std stderr\n");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_std_and_os_use_wanix_process_context() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "app/main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

const readStdin = () => {
  const bytes = new Uint8Array(64);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  return Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
};

const env = std.getenviron();
std.out.puts("argv " + scriptArgs.join("|") + "\n");
std.out.puts("mode " + std.getenv("MODE") + "\n");
std.out.puts("env " + env.MODE + " " + (env.EMPTY === "") + " " + String(env.MISSING) + "\n");
std.out.puts("task " + std.loadFile('#task/self/id').trim() + "\n");
std.out.puts("stdin " + readStdin() + "\n");
std.out.puts("source " + std.loadFile("main.js").includes("std.getenv") + "\n");
std.writeFile("created.txt", "made via std cwd");
std.out.puts("created " + std.loadFile("created.txt") + "\n");
std.out.flush();
"#,
        )
        .unwrap();
        let stdin = std::sync::Arc::new(MemFs::new());
        stdin.write_file("in", b"hello from fd0").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDIN,
            stdin
                .open(&NormalizedPath::new("in").unwrap(), OpenOptions::read())
                .unwrap(),
            NormalizedPath::new("in").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js alpha beta").unwrap();
        task.set_env_lines("MODE=test\nEMPTY=").unwrap();
        task.set_dir("app").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"argv main.js|alpha|beta\nmode test\nenv test true undefined\ntask 1\nstdin hello from fd0\nsource true\ncreated made via std cwd\n"
        );
        assert_eq!(
            root.read_file("app/created.txt").unwrap(),
            b"made via std cwd"
        );
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_sleep_completes_timer_poll() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.out.puts("before sleep\n");
os.sleep(0);
std.out.puts("after sleep\n");
std.out.flush();
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"before sleep\nafter sleep\n");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_runs_zero_delay_quickjs_async_timers() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.out.puts("sync\n");
os.sleepAsync(0).then(() => {
  std.out.puts("sleepAsync\n");
  std.out.flush();
});
std.out.flush();
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        let output = String::from_utf8(read_file(&*stdout, "out")).unwrap();
        assert!(output.starts_with("sync\n"), "{output}");
        assert!(output.contains("sleepAsync\n"), "{output}");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_runs_future_quickjs_timers_with_wait_budget() {
        let table = TaskTable::new();
        let runner = runner();
        let driver =
            QuickJsTaskDriver::new(runner).with_event_loop_wait_budget(Duration::from_millis(10));
        table
            .register_driver("qjs", std::sync::Arc::new(driver))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.out.puts("sync\n");
os.setTimeout(() => {
  std.out.puts("timeout\n");
  std.out.flush();
}, 1);
std.out.flush();
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"sync\ntimeout\n");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_runs_cleared_quickjs_intervals_with_wait_budget() {
        let table = TaskTable::new();
        let runner = runner();
        let driver =
            QuickJsTaskDriver::new(runner).with_event_loop_wait_budget(Duration::from_millis(10));
        table
            .register_driver("qjs", std::sync::Arc::new(driver))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

let count = 0;
std.out.puts("sync\n");
const interval = os.setInterval(() => {
  count += 1;
  std.out.puts("tick " + count + "\n");
  if (count >= 3) {
    os.clearInterval(interval);
    std.out.flush();
  }
}, 1);
std.out.flush();
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"sync\ntick 1\ntick 2\ntick 3\n"
        );
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_interrupt_budget_stops_cpu_bound_quickjs() {
        let table = TaskTable::new();
        let runner = runner();
        let driver = QuickJsTaskDriver::new(runner).with_interrupt_poll_budget(0);
        table
            .register_driver("qjs", std::sync::Arc::new(driver))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("main.js", b"while (true) {}").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js").unwrap();

        let err = table
            .start(task.id())
            .expect_err("interrupt budget should stop a CPU-bound task");

        assert!(
            err.to_string().contains("interrupted"),
            "unexpected error: {err}"
        );
        assert_eq!(task.exit(), "1");
    }

    #[test]
    fn task_driver_memory_limit_stops_allocation_heavy_quickjs() {
        let table = TaskTable::new();
        let runner = runner();
        let driver = QuickJsTaskDriver::new(runner).with_memory_limit_bytes(1024 * 1024);
        table
            .register_driver("qjs", std::sync::Arc::new(driver))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
print("before allocation");
globalThis.tooBig = new ArrayBuffer(16 * 1024 * 1024);
print("after allocation");
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        let err = table
            .start(task.id())
            .expect_err("memory limit should stop a large allocation");

        assert!(
            err.to_string().contains("QuickJS exception"),
            "unexpected error: {err}"
        );
        assert_eq!(read_file(&*stdout, "out"), b"before allocation\n");
        assert_eq!(task.exit(), "1");
    }

    #[test]
    fn task_driver_runs_quickjs_read_handler_for_ready_stdin() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.out.puts("sync\n");
os.setReadHandler(0, () => {
  const bytes = new Uint8Array(64);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  const text = Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
  std.out.puts("handler " + text + "\n");
  os.setReadHandler(0, null);
  std.out.flush();
});
std.out.flush();
"#,
        )
        .unwrap();
        let stdin = std::sync::Arc::new(MemFs::new());
        stdin.write_file("in", b"ready stdin").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDIN,
            stdin
                .open(&NormalizedPath::new("in").unwrap(), OpenOptions::read())
                .unwrap(),
            NormalizedPath::new("in").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"sync\nhandler ready stdin\n");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_runs_multiple_ready_io_turns_when_configured() {
        let table = TaskTable::new();
        let runner = runner();
        let driver = QuickJsTaskDriver::new(runner).with_ready_io_turns(2);
        table
            .register_driver("qjs", std::sync::Arc::new(driver))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

let chunks = 0;
const bytes = new Uint8Array(3);

std.out.puts("sync\n");
os.setReadHandler(0, () => {
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  const text = Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
  chunks += 1;
  std.out.puts("chunk " + chunks + " " + text + "\n");
  if (chunks >= 2) {
    os.setReadHandler(0, null);
    std.out.flush();
  }
});
std.out.flush();
"#,
        )
        .unwrap();
        let stdin = std::sync::Arc::new(MemFs::new());
        stdin.write_file("in", b"abcdef").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDIN,
            stdin
                .open(&NormalizedPath::new("in").unwrap(), OpenOptions::read())
                .unwrap(),
            NormalizedPath::new("in").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"sync\nchunk 1 abc\nchunk 2 def\n"
        );
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_std_and_os_read_wanix_namespace_files() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

print("std", std.loadFile("input.txt"));
const fd = os.open("input.txt", os.O_RDONLY);
const bytes = new Uint8Array(64);
const n = os.read(fd, bytes.buffer, 0, bytes.length);
os.close(fd);
const text = Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
print("os", text);
"#,
        )
        .unwrap();
        root.write_file("input.txt", b"from qjs stdlib").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"std from qjs stdlib\nos from qjs stdlib\n"
        );
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_cleans_unclosed_quickjs_os_file_fds_after_run() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as os from "qjs:os";

const fd = os.open("input.txt", os.O_RDONLY);
print("opened", fd);
"#,
        )
        .unwrap();
        root.write_file("input.txt", b"left open by guest").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"opened 4\n");
        // task-exit-closes-fds: an exited task holds no descriptors, stdio
        // included (its output lives in the fd's backing, read above).
        assert_eq!(task.fd_numbers(), []);
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn quickjs_snapshot_restore_reattaches_wanix_wasi_host() {
        let runner = runner();
        let root = Arc::new(MemFs::new());
        root.write_file("input.txt", b"from namespace").unwrap();
        root.write_file("suffix.js", b"export const suffix = 'restored';")
            .unwrap();
        root.write_file("before-stdout", b"").unwrap();
        root.write_file("after-stdout", b"").unwrap();
        let mut namespace = Namespace::new();
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();

        let before_stdout = root
            .open(
                &NormalizedPath::new("before-stdout").unwrap(),
                OpenOptions::read_write(),
            )
            .unwrap();
        let mut runtime = runner
            .create_runtime_with_wanix_config(QuickJsWanixConfig::new(
                WasiConfig::new(namespace.clone()).with_stdout(before_stdout, "before stdout"),
            ))
            .unwrap();

        runtime
            .eval_module_discard(
                r#"
import * as std from "qjs:std";

globalThis.before = std.loadFile("input.txt").trimEnd();
std.out.puts("before " + globalThis.before + "\n");
std.out.flush();
"#,
                "before-snapshot.mjs",
            )
            .unwrap();
        assert_eq!(
            read_file(&*root, "before-stdout"),
            b"before from namespace\n"
        );
        let snapshot_bytes = runtime.snapshot().unwrap().try_to_bytes().unwrap();
        drop(runtime);

        let after_stdout = root
            .open(
                &NormalizedPath::new("after-stdout").unwrap(),
                OpenOptions::read_write(),
            )
            .unwrap();
        let mut restored = runner
            .restore_runtime_from_bytes_with_wanix_config(
                &snapshot_bytes,
                QuickJsWanixConfig::new(
                    WasiConfig::new(namespace).with_stdout(after_stdout, "after stdout"),
                ),
            )
            .unwrap();

        restored
            .eval_module_discard(
                r##"
import * as std from "qjs:std";
import { suffix } from "./suffix.js";

const text = globalThis.before + " -> " + suffix;
std.writeFile("after.txt", text);
std.out.puts("after " + std.loadFile("after.txt") + "\n");
std.out.flush();
"##,
                "after-restore.mjs",
            )
            .unwrap();

        assert_eq!(
            read_file(&*root, "after.txt"),
            b"from namespace -> restored"
        );
        assert_eq!(
            read_file(&*root, "after-stdout"),
            b"after from namespace -> restored\n"
        );
        assert_eq!(
            read_file(&*root, "before-stdout"),
            b"before from namespace\n"
        );
    }

    #[test]
    fn task_runtime_snapshot_restore_reattaches_wanix_task_state() {
        let runner = runner();
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = Arc::new(MemFs::new());
        root.write_file("app/input.txt", b"from task cwd").unwrap();
        root.write_file("app/main.js", b"").unwrap();
        let before_stdout = Arc::new(MemFs::new());
        before_stdout.write_file("stdout", b"").unwrap();
        let after_stdout = Arc::new(MemFs::new());
        after_stdout.write_file("stdout", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            before_stdout
                .open(
                    &NormalizedPath::new("stdout").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("stdout").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js 'two words'").unwrap();
        task.set_env_lines("MODE=before").unwrap();
        task.set_dir("app").unwrap();
        let mut runtime = runner.create_task_runtime(&task).unwrap();

        runtime
            .eval_module_discard(
                r#"
import * as std from "qjs:std";

globalThis.before = std.loadFile("input.txt").trim();
std.out.puts("before " + globalThis.before + " " + scriptArgs.join("|") + "\n");
std.out.flush();
"#,
                "app/main.js",
            )
            .unwrap();
        let snapshot_bytes = runtime.snapshot_bytes().unwrap();
        drop(runtime);

        task.close_fd(Fd::STDOUT).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            after_stdout
                .open(
                    &NormalizedPath::new("stdout").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("stdout").unwrap(),
        )
        .unwrap();
        task.set_env_lines("MODE=after").unwrap();
        root.write_file("app/suffix.js", b"export const suffix = 'restored';")
            .unwrap();
        let mut restored = runner
            .restore_task_runtime_from_bytes(&task, &snapshot_bytes)
            .unwrap();

        restored
            .eval_module_discard(
                r##"
import * as std from "qjs:std";
import { suffix } from "./suffix.js";

std.out.puts(
  "after " + globalThis.before
    + " wasi=" + std.getenv("MODE")
    + " taskenv=" + std.loadFile("#task/self/env").trim()
    + " " + std.loadFile("#task/self/id").trim()
    + " " + suffix
    + "\n"
);
std.writeFile("after.txt", scriptArgs.slice(1).join("|"));
std.out.flush();
std.exit(6);
"##,
                "app/after.mjs",
            )
            .unwrap();
        assert_eq!(restored.exit_code().unwrap(), Some(6));
        restored.finish().unwrap();

        assert_eq!(
            before_stdout.read_file("stdout").unwrap(),
            b"before from task cwd main.js|two words\n"
        );
        assert_eq!(
            after_stdout.read_file("stdout").unwrap(),
            b"after from task cwd wasi=before taskenv=MODE=after 1 restored\n"
        );
        assert_eq!(root.read_file("app/after.txt").unwrap(), b"two words");
        assert_eq!(task.exit(), "6");
    }

    #[test]
    fn task_runtime_snapshot_rejects_open_wanix_wasi_fd() {
        let runner = runner();
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = Arc::new(MemFs::new());
        root.write_file("app/input.txt", b"from task cwd").unwrap();
        root.write_file("app/main.js", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js").unwrap();
        task.set_dir("app").unwrap();
        let mut runtime = runner.create_task_runtime(&task).unwrap();

        runtime
            .eval_module_discard(
                r#"
import * as os from "qjs:os";

globalThis.openFd = os.open("input.txt", os.O_RDONLY);
"#,
                "app/main.js",
            )
            .unwrap();
        let err = runtime
            .snapshot_bytes()
            .expect_err("snapshot should reject open Wanix WASI fds");

        assert!(err.to_string().contains("open dynamic WASI fd"));
    }

    #[test]
    fn task_runtime_snapshot_rejects_after_wasi_exit() {
        let runner = runner();
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = Arc::new(MemFs::new());
        root.write_file("main.js", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js").unwrap();
        let mut runtime = runner.create_task_runtime(&task).unwrap();

        runtime
            .eval_module_discard(
                r#"
import * as std from "qjs:std";

std.exit(5);
"#,
                "main.js",
            )
            .unwrap();
        let err = runtime
            .snapshot_bytes()
            .expect_err("snapshot should reject exited Wanix task runtimes");

        assert!(err.to_string().contains("Wanix process exit"));
    }

    #[test]
    fn task_driver_quickjs_std_reads_task_service_paths_through_wasi() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "app/main.js",
            br##"
import * as std from "qjs:std";

std.out.puts("source " + std.loadFile("main.js").includes("qjs:std") + "\n");
std.out.puts("id " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("cmd " + std.loadFile("#task/self/cmd").trim() + "\n");
std.out.puts("dir " + std.loadFile("#task/self/dir").trim() + "\n");
std.out.puts("env " + std.loadFile("#task/self/env").trim() + "\n");
std.out.flush();
"##,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js --service").unwrap();
        task.set_env_lines("MODE=service").unwrap();
        task.set_dir("app").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"source true\nid 1\ncmd main.js --service\ndir app\nenv MODE=service\n"
        );
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_readdir_lists_task_namespace() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function visible(path) {
  const [entries, err] = os.readdir(path);
  if (err !== 0) {
    throw new Error("readdir " + path + ": " + err);
  }
  return entries.filter((name) => name !== "." && name !== "..").sort().join(",");
}

const rootList = visible(".");
const dirList = visible("dir");
std.out.puts("root " + rootList + "\n");
std.out.puts("dir " + dirList + "\n");
std.out.flush();
"##,
        )
        .unwrap();
        root.write_file("alpha.txt", b"alpha").unwrap();
        root.write_file("dir/beta.txt", b"beta").unwrap();
        root.write_file("dir/gamma.txt", b"gamma").unwrap();
        root.write_file("#hidden", b"hidden").unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(task.exit(), "0");
        assert_eq!(
            read_file(&*stdout, "out"),
            b"root alpha.txt,dir,main.js\ndir beta.txt,gamma.txt\n",
            "task exit {}",
            task.exit()
        );
    }

    #[test]
    fn task_driver_quickjs_os_starts_child_task_through_task_service() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function bytesFromString(text) {
  return new Uint8Array(Array.from(text).map((char) => char.charCodeAt(0)));
}

function readServiceText(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const chunks = [];
  const bytes = new Uint8Array(64);
  while (true) {
    const count = os.read(fd, bytes.buffer, 0, bytes.length);
    if (count < 0) {
      throw new Error("read " + path + ": " + count);
    }
    if (count === 0) {
      break;
    }
    chunks.push(stringFromBytes(bytes, count));
    if (count < bytes.length) {
      break;
    }
  }
  os.close(fd);
  return chunks.join("");
}

function writeServiceText(path, text) {
  const fd = os.open(path, os.O_WRONLY);
  if (fd < 0) {
    throw new Error("open " + path + ": " + fd);
  }
  const bytes = bytesFromString(text);
  const count = os.write(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count !== bytes.length) {
    throw new Error("short write " + path + ": " + count + "/" + bytes.length);
  }
}

const parent = readServiceText("#task/self/id").trim();
const child = readServiceText("#task/new/qjs").trim();
std.writeFile("child-stdin.txt", "stdin from parent\n");
writeServiceText("#task/" + child + "/cmd", "child.js alpha 'two words' '' beta\n");
writeServiceText("#task/" + child + "/env", "MODE=child\n");
writeServiceText("#task/" + child + "/dir", ".\n");
writeServiceText("#task/" + child + "/ctl", "bind child-stdin.txt fd/0\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/1 fd/1\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/2 fd/2\n");
print("parent " + parent);
print("child " + child);
writeServiceText("#task/" + child + "/ctl", "start\n");
print("child exit " + readServiceText("#task/" + child + "/exit").trim());
"##,
        )
        .unwrap();
        root.write_file(
            "child.js",
            br##"
import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts(
  "id " + std.loadFile("#task/self/id").trim()
    + " args " + scriptArgs.join("|")
    + " mode " + std.getenv("MODE")
    + " stdin " + readStdin().trimEnd()
    + "\n"
);
std.out.flush();
std.err.puts("stderr mode " + std.getenv("MODE") + "\n");
std.err.flush();
std.exit(5);
"##,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        let stderr = std::sync::Arc::new(MemFs::new());
        stderr.write_file("err", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.insert_fd(
            Fd::STDERR,
            stderr
                .open(
                    &NormalizedPath::new("err").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("err").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"parent 1\nchild 2\nid 2 args child.js|alpha|two words||beta mode child stdin stdin from parent\nchild exit 5\n"
        );
        assert_eq!(read_file(&*stderr, "err"), b"stderr mode child\n");
        assert_eq!(task.exit(), "0");
        assert_eq!(table.get(TaskId::new(2)).unwrap().exit(), "5");
    }

    #[test]
    fn task_driver_quickjs_std_and_os_write_wanix_namespace_files() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("std-created.txt", "from std write");
const bytes = new Uint8Array([102, 114, 111, 109, 32, 111, 115, 32, 119, 114, 105, 116, 101]);
const fd = os.open("os-created.txt", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
const count = os.write(fd, bytes.buffer, 0, bytes.length);
os.close(fd);
print("std", std.loadFile("std-created.txt"));
print("os", std.loadFile("os-created.txt"));
print("bytes", count);
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"std from std write\nos from os write\nbytes 13\n"
        );
        assert_eq!(
            root.read_file("std-created.txt").unwrap(),
            b"from std write"
        );
        assert_eq!(root.read_file("os-created.txt").unwrap(), b"from os write");
        assert_eq!(task.exit(), "0");
    }

    #[cfg(unix)]
    #[test]
    fn task_driver_quickjs_os_symlink_readlink_and_lstat_reach_wanix_namespace() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let host = temp_dir("wanix-qjs-symlink");
        std::fs::write(host.join("target.txt"), b"from host target").unwrap();
        std::fs::write(
            host.join("main.js"),
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

print("symlink", os.symlink("target.txt", "link.txt"));
const [target, readlinkErr] = os.readlink("link.txt");
print("readlink", readlinkErr, target);
const [linkStat, lstatErr] = os.lstat("link.txt");
print("lstat", lstatErr, (linkStat.mode & os.S_IFMT) === os.S_IFLNK, linkStat.size);
const [targetStat, statErr] = os.stat("link.txt");
print("stat", statErr, (targetStat.mode & os.S_IFMT) === os.S_IFREG, targetStat.size);
print("load", std.loadFile("link.txt"));
"#,
        )
        .unwrap();
        let local = std::sync::Arc::new(LocalFs::new(&host).unwrap());
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(local.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"symlink 0\nreadlink 0 target.txt\nlstat 0 true 10\nstat 0 true 16\nload from host target\n"
        );
        assert_eq!(
            std::fs::read_link(host.join("link.txt")).unwrap(),
            std::path::Path::new("target.txt")
        );
        assert_eq!(task.exit(), "0");
        std::fs::remove_dir_all(host).unwrap();
    }

    #[test]
    fn task_driver_quickjs_os_truncate_and_ftruncate_resize_wanix_files() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("resize.txt", "abcdef");

const fd = os.open("resize.txt", os.O_RDWR);
print("ftruncate", os.ftruncate(fd, 3));
os.close(fd);
print("small", std.loadFile("resize.txt"));

print("truncate", os.truncate("resize.txt", 5));
const fd2 = os.open("resize.txt", os.O_RDONLY);
const bytes = new Uint8Array(8);
const n = os.read(fd2, bytes.buffer, 0, bytes.length);
os.close(fd2);
print("len", n);
print("codes", Array.from(bytes.slice(0, n)).join(","));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"ftruncate 0\nsmall abc\ntruncate 0\nlen 5\ncodes 97,98,99,0,0\n"
        );
        assert_eq!(root.read_file("resize.txt").unwrap(), b"abc\0\0");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_append_mode_writes_at_end_of_wanix_files() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("log.txt", "start");
const fd = os.open("log.txt", os.O_WRONLY | os.O_APPEND);
os.seek(fd, 0, std.SEEK_SET);
const bytes = new Uint8Array([45, 111, 115]);
print("os bytes", os.write(fd, bytes.buffer, 0, bytes.length));
os.close(fd);

const file = std.open("log.txt", "a");
file.puts("-std");
file.close();

const fd2 = os.open("log.txt", os.O_WRONLY);
const file2 = std.fdopen(fd2, "a");
file2.puts("-fdopen");
file2.close();

print("log", std.loadFile("log.txt"));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"os bytes 3\nlog start-os-std-fdopen\n"
        );
        assert_eq!(root.read_file("log.txt").unwrap(), b"start-os-std-fdopen");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_fd_fdstat_set_flags_updates_mirrored_task_fd_append() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br##"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("log.txt", "start");
const fd = os.open("log.txt", os.O_WRONLY);
const file = std.fdopen(fd, "a");
const mirror = os.open("#task/self/fd/" + fd, os.O_WRONLY);
const bytes = new Uint8Array([45, 109, 105, 114, 114, 111, 114]);
print("mirror bytes", os.write(mirror, bytes.buffer, 0, bytes.length));
os.close(mirror);
file.close();
print("log", std.loadFile("log.txt"));
"##,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"mirror bytes 7\nlog start-mirror\n"
        );
        assert_eq!(root.read_file("log.txt").unwrap(), b"start-mirror");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_removes_wanix_namespace_files() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("delete-me.txt", "remove me");
std.writeFile("keep.txt", "keep me");
print("remove result", JSON.stringify(os.remove("delete-me.txt")));

const probe = os.open("delete-me.txt", os.O_RDONLY);
const deleted = probe < 0;
if (probe >= 0) {
  os.close(probe);
}
print("deleted", deleted);
print("keep", std.loadFile("keep.txt"));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"remove result 0\ndeleted true\nkeep keep me\n"
        );
        assert_eq!(
            root.metadata(&NormalizedPath::new("delete-me.txt").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(root.read_file("keep.txt").unwrap(), b"keep me");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_renames_wanix_namespace_paths() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("old.txt", "rename me");
print("rename result", JSON.stringify(os.rename("old.txt", "renamed.txt")));
const oldFd = os.open("old.txt", os.O_RDONLY);
if (oldFd >= 0) {
  os.close(oldFd);
}
print("old missing", oldFd < 0);
print("new", std.loadFile("renamed.txt"));

os.mkdir("dir", 0o777);
std.writeFile("dir/file.txt", "nested");
os.mkdir("empty", 0o777);
print("dir rename", JSON.stringify(os.rename("dir", "empty")));
print("nested", std.loadFile("empty/file.txt"));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"rename result 0\nold missing true\nnew rename me\ndir rename 0\nnested nested\n"
        );
        assert_eq!(
            root.metadata(&NormalizedPath::new("old.txt").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(root.read_file("renamed.txt").unwrap(), b"rename me");
        assert_eq!(
            root.metadata(&NormalizedPath::new("dir").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(root.read_file("empty/file.txt").unwrap(), b"nested");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_utimes_updates_wanix_namespace_metadata() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("stamp.txt", "timestamped");
print("utimes result", JSON.stringify(os.utimes("stamp.txt", new Date(1000), new Date(2000))));
const stat = os.stat("stamp.txt")[0];
print("atime", stat.atime);
print("mtime", stat.mtime);
print("message", std.loadFile("stamp.txt"));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"utimes result 0\natime 1000\nmtime 2000\nmessage timestamped\n"
        );
        let metadata = root
            .metadata(&NormalizedPath::new("stamp.txt").unwrap())
            .unwrap();
        assert_eq!(metadata.accessed_time_ns(), 1_000_000_000);
        assert_eq!(metadata.modified_time_ns(), 2_000_000_000);
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_creates_wanix_namespace_directories() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

print("mkdir result", JSON.stringify(os.mkdir("made", 0o777)));
std.writeFile("made/file.txt", "inside made");
print("made file", std.loadFile("made/file.txt"));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"mkdir result 0\nmade file inside made\n"
        );
        assert_eq!(
            root.metadata(&NormalizedPath::new("made").unwrap())
                .unwrap()
                .file_type(),
            FileType::Directory
        );
        assert_eq!(root.read_file("made/file.txt").unwrap(), b"inside made");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_removes_wanix_namespace_directories() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"
import * as std from "qjs:std";
import * as os from "qjs:os";

os.mkdir("empty", 0o777);
os.mkdir("keep", 0o777);
std.writeFile("keep/file.txt", "kept");
print("remove dir result", JSON.stringify(os.remove("empty")));
const probe = os.open("empty", os.O_RDONLY);
if (probe >= 0) {
  os.close(probe);
}
print("removed", probe < 0);
print("keep", std.loadFile("keep/file.txt"));
"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"remove dir result 0\nremoved true\nkeep kept\n"
        );
        assert_eq!(
            root.metadata(&NormalizedPath::new("empty").unwrap()),
            Err(FsError::NotFound)
        );
        assert_eq!(root.read_file("keep/file.txt").unwrap(), b"kept");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_quickjs_os_preserves_stdio_write_order() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br##"
import * as os from "qjs:os";

const bytes = Uint8Array.from([98, 10]);

print("a");
os.write(1, bytes.buffer, 0, bytes.length);
print("c");
"##,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"a\nb\nc\n");
        // task-exit-closes-fds: an exited task holds no descriptors, stdio
        // included (its output lives in the fd's backing, read above).
        assert_eq!(task.fd_numbers(), []);
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_check_uses_the_first_command_word() {
        let driver = QuickJsTaskDriver::new(runner());
        let task = Task::new(
            TaskId::new(1),
            TaskSpec::new(".").unwrap(),
            wanix_vfs::Namespace::new(),
        );

        task.set_cmd("main.js\t--flag").unwrap();
        assert!(driver.check(&task));

        task.set_cmd("notes.txt main.js").unwrap();
        assert!(!driver.check(&task));
    }

    #[test]
    fn task_driver_check_uses_typed_task_spec_program_when_present() {
        let driver = QuickJsTaskDriver::new(runner());
        let task = Task::new(
            TaskId::new(1),
            TaskSpec::new("notes.txt").unwrap(),
            wanix_vfs::Namespace::new(),
        );

        task.set_cmd("not-js.txt").unwrap();
        assert!(!driver.check(&task));

        let mut spec = TaskSpec::new("main.js").unwrap();
        spec.args = vec!["two words".to_owned()];
        task.set_spec(spec).unwrap();
        assert!(driver.check(&task));
    }

    #[test]
    fn task_driver_sets_nonzero_exit_on_failure() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        task.set_cmd("missing.js").unwrap();

        let err = table.start(task.id()).unwrap_err();

        assert!(matches!(err, wanix_fs::FsError::NotFound));
        assert_eq!(task.exit(), "1");
    }

    #[test]
    fn task_driver_preserves_output_before_javascript_failure() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"print("before failure"); throw new Error("boom");"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        let err = table.start(task.id()).unwrap_err();

        assert!(err.to_string().contains("QuickJS error"));
        assert_eq!(read_file(&*stdout, "out"), b"before failure\n");
        assert_eq!(task.exit(), "1");
    }

    #[test]
    fn task_driver_reports_module_top_level_throw() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file(
            "main.js",
            br#"import * as std from "qjs:std"; print("before failure"); throw new Error("module boom");"#,
        )
        .unwrap();
        let stdout = std::sync::Arc::new(MemFs::new());
        stdout.write_file("out", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.insert_fd(
            Fd::STDOUT,
            stdout
                .open(
                    &NormalizedPath::new("out").unwrap(),
                    OpenOptions::read_write(),
                )
                .unwrap(),
            NormalizedPath::new("out").unwrap(),
        )
        .unwrap();
        task.set_cmd("main.js").unwrap();

        let err = table.start(task.id()).unwrap_err();

        assert!(
            err.to_string().contains("module boom"),
            "module throw should surface the thrown message: {err}"
        );
        assert_eq!(read_file(&*stdout, "out"), b"before failure\n");
        assert_eq!(task.exit(), "1");
    }
}
