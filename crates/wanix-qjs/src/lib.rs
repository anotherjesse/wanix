//! QuickJS/WASI task driver for Rust Wanix.
//!
//! This crate adapts the `rust-wasi-quickjs` prototype into a Wanix task
//! driver. QuickJS runs inside a WASI reactor hosted by Wasmtime; Wanix supplies
//! the namespace, fds, host callbacks, module loading policy, and snapshot
//! reattachment policy.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rust_wasi_quickjs::{
    QuickJsCreateOptions, QuickJsHostConfig, QuickJsModule, QuickJsRestoreOptions, QuickJsRuntime,
};
use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath};
use wanix_task::{Fd, Task, TaskSpec, quote_cmd_argv};
use wanix_wasi::WasiConfig;
use wasmtime::Engine;

mod driver;
mod fd_api;
mod host_api;
mod task_context;
mod task_runtime;
mod task_stdio;
mod virtual_wasi;
mod wasi_host;

pub use driver::QuickJsTaskDriver;
pub use task_runtime::QuickJsTaskRuntime;

use fd_api::define_fd_output_callback;
use host_api::{
    define_output_callback, define_output_callback_with_exit_state, define_wanix_host_api,
    define_wanix_module_loader, define_wanix_namespace_api, qjs_error, read_namespace_file,
    take_buffer,
};
use task_context::{WanixExitState, WanixTaskContext};
use task_stdio::task_wasi_config;
use virtual_wasi::with_namespace_read_only_files;
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

/// Result of running QuickJS source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl RunOutput {
    fn empty() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// Returns captured stdout bytes.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns captured stderr bytes.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

/// QuickJS runtime runner backed by the `rust-wasi-quickjs` prototype.
pub struct QuickJsRunner {
    _engine: Engine,
    module: QuickJsModule,
}

#[derive(Debug)]
struct RunFailure {
    error: FsError,
    output: RunOutput,
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
        let engine = Engine::default();
        let module = QuickJsModule::from_file(&engine, path)
            .map_err(|err| FsError::Other(format!("failed to load QuickJS wasm: {err:#}")))?;
        Ok(Self {
            _engine: engine,
            module,
        })
    }

    /// Compiles a QuickJS WASM module from in-memory bytes.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the module cannot be compiled.
    pub fn from_wasm_bytes(bytes: impl AsRef<[u8]>) -> FsResult<Self> {
        let engine = Engine::default();
        let module = QuickJsModule::from_bytes(&engine, bytes.as_ref())
            .map_err(|err| FsError::Other(format!("failed to load QuickJS wasm: {err:#}")))?;
        Ok(Self {
            _engine: engine,
            module,
        })
    }

    /// Loads the workspace-local QuickJS WASM fixture bundled by the engine crate.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the bundled module cannot be compiled.
    pub fn from_bundled_wasm() -> FsResult<Self> {
        Self::from_wasm_bytes(rust_wasi_quickjs::QUICKJS_WASM_FIXTURE)
    }

    /// Runs JavaScript source outside Chrome and captures `print`/`console` output.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS creation, host callback setup,
    /// or JavaScript evaluation fails.
    pub fn run_source(&self, source: &str) -> FsResult<RunOutput> {
        self.run_source_with_setup(source, captured_stdio_options(), |_| Ok(()))
            .map_err(|failure| failure.error)
    }

    /// Runs JavaScript source with access to a Wanix namespace host API.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS setup, namespace callbacks, or
    /// JavaScript evaluation fails.
    pub fn run_source_with_namespace(
        &self,
        source: &str,
        namespace: impl FileSystem + Clone + 'static,
    ) -> FsResult<RunOutput> {
        let config = with_namespace_read_only_files(captured_stdio_config(), &namespace)?;
        self.run_source_with_setup(source, create_options_with_config(config), move |runtime| {
            define_wanix_module_loader(runtime, namespace.clone())?;
            define_wanix_namespace_api(runtime, namespace)
        })
        .map_err(|failure| failure.error)
    }

    /// Runs JavaScript source with Wanix-backed QuickJS configuration.
    ///
    /// Wanix-owned WASI settings are attached as a live host provider while the
    /// interim writable `Wanix` host object remains available for behavior not
    /// yet exercised by the bundled QuickJS WASI fixture.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when WASI host setup, QuickJS setup,
    /// namespace callbacks, or JavaScript evaluation fails.
    pub fn run_source_with_wanix_config(
        &self,
        source: &str,
        config: QuickJsWanixConfig,
    ) -> FsResult<RunOutput> {
        let namespace = config.wasi().namespace().clone();
        let create_options = captured_stdio_options_with_wanix_wasi(config)?;
        self.run_source_with_setup(source, create_options, move |runtime| {
            define_wanix_module_loader(runtime, namespace.clone())?;
            define_wanix_namespace_api(runtime, namespace)
        })
        .map_err(|failure| failure.error)
    }

    /// Creates a QuickJS runtime with live Wanix-backed WASI imports.
    ///
    /// The returned runtime can be snapshotted through the engine API. Use
    /// [`Self::restore_runtime_from_bytes_with_wanix_config`] to resume that VM
    /// image with fresh Wanix host resources. This lifecycle helper intentionally
    /// installs only the live WASI provider and namespace module loader; it does
    /// not install the interim `Wanix` global or `print`/`console` callbacks,
    /// because those are host callback objects that require separate restore-time
    /// reattachment. Guest `qjs:std` stdout and stderr writes go through the
    /// `WasiConfig` fd attachments supplied by `config`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the Wanix WASI host cannot be created, the
    /// QuickJS runtime cannot be instantiated, or the namespace module loader
    /// cannot be attached.
    pub fn create_runtime_with_wanix_config(
        &self,
        config: QuickJsWanixConfig,
    ) -> FsResult<QuickJsRuntime> {
        let namespace = config.wasi().namespace().clone();
        let create_options = create_options_with_wanix_wasi(config)?;
        let mut runtime = self
            .module
            .create_runtime_with_options(create_options)
            .map_err(qjs_error)?;
        define_wanix_module_loader(&mut runtime, namespace)?;
        Ok(runtime)
    }

    /// Restores a QuickJS VM image with live Wanix-backed WASI imports.
    ///
    /// Snapshot bytes remain a QuickJS VM image. The supplied Wanix config
    /// reattaches namespace, preopen, fd, argv/env, and stdio host resources for
    /// the restored runtime. Guest `qjs:std` stdout and stderr writes go through
    /// the `WasiConfig` fd attachments supplied by `config`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the snapshot bytes are invalid or
    /// incompatible with this runner's QuickJS module, the Wanix WASI host cannot
    /// be created, the runtime cannot be restored, or the namespace module loader
    /// cannot be attached.
    pub fn restore_runtime_from_bytes_with_wanix_config(
        &self,
        bytes: &[u8],
        config: QuickJsWanixConfig,
    ) -> FsResult<QuickJsRuntime> {
        let namespace = config.wasi().namespace().clone();
        let restore_options = restore_options_with_wanix_wasi(config)?;
        let mut runtime = self
            .module
            .restore_runtime_from_bytes_with_options(bytes, restore_options)
            .map_err(qjs_error)?;
        define_wanix_module_loader(&mut runtime, namespace)?;
        Ok(runtime)
    }

    /// Runs JavaScript as an ES module with access to a Wanix namespace.
    ///
    /// The namespace is copied into QuickJS's read-only WASI virtual filesystem
    /// before execution. The interim `Wanix` host API is also installed so this
    /// can coexist with the first vertical slice while WASI write support lands.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when namespace projection, QuickJS setup, or
    /// JavaScript module evaluation fails.
    pub fn run_module_with_namespace(
        &self,
        source: &str,
        filename: &str,
        namespace: impl FileSystem + Clone + 'static,
    ) -> FsResult<RunOutput> {
        let config = with_namespace_read_only_files(captured_stdio_config(), &namespace)?;
        self.run_with_setup(
            source,
            create_options_with_config(config),
            move |runtime| {
                define_wanix_module_loader(runtime, namespace.clone())?;
                define_wanix_namespace_api(runtime, namespace)
            },
            |runtime, source| {
                runtime
                    .eval_module_discard(source, filename)
                    .map_err(qjs_error)
            },
        )
        .map_err(|failure| failure.error)
    }

    fn run_source_with_setup(
        &self,
        source: &str,
        create_options: QuickJsCreateOptions,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        self.run_with_setup(source, create_options, setup, |runtime, source| {
            runtime.eval_discard(source).map_err(qjs_error)
        })
    }

    fn run_with_setup(
        &self,
        source: &str,
        create_options: QuickJsCreateOptions,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
        eval: impl FnOnce(&mut QuickJsRuntime, &str) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        self.run_with_setup_control(source, create_options, None, None, setup, eval)
    }

    fn run_with_setup_control(
        &self,
        source: &str,
        create_options: QuickJsCreateOptions,
        exit_state: Option<WanixExitState>,
        output_task: Option<Task>,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
        eval: impl FnOnce(&mut QuickJsRuntime, &str) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        let mut runtime = self
            .module
            .create_runtime_with_options(create_options)
            .map_err(|err| RunFailure {
                error: qjs_error(err),
                output: RunOutput::empty(),
            })?;

        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let result = (|| -> FsResult<()> {
            define_task_output_callback(
                &mut runtime,
                "__wanix_stdout",
                Arc::clone(&stdout),
                exit_state.clone(),
                output_task.clone().map(|task| (task, Fd::STDOUT)),
            )?;
            define_task_output_callback(
                &mut runtime,
                "__wanix_stderr",
                Arc::clone(&stderr),
                exit_state.clone(),
                output_task.clone().map(|task| (task, Fd::STDERR)),
            )?;
            if let Some(exit_state) = exit_state.clone() {
                runtime
                    .set_interrupt_handler(move || {
                        exit_state.code().map(|code| code.is_some()).unwrap_or(true)
                    })
                    .map_err(qjs_error)?;
            }
            setup(&mut runtime)?;
            runtime.eval_discard(CONSOLE_PRELUDE).map_err(qjs_error)?;
            eval(&mut runtime, source)?;
            if !exit_requested(&exit_state)? {
                runtime
                    .execute_pending_jobs_with_limit(1024)
                    .map_err(qjs_error)?;
            }
            Ok(())
        })();

        let mut stdout = take_buffer(stdout).map_err(|error| RunFailure {
            error,
            output: RunOutput::empty(),
        })?;
        stdout.extend_from_slice(&runtime.take_captured_stdout());
        let mut stderr = take_buffer(stderr).map_err(|error| RunFailure {
            error,
            output: RunOutput::empty(),
        })?;
        stderr.extend_from_slice(&runtime.take_captured_stderr());
        let output = RunOutput { stdout, stderr };
        match result {
            Ok(()) => Ok(output),
            Err(error) => Err(RunFailure { error, output }),
        }
    }

    /// Runs the script named in `task.cmd()` and writes output through task fds.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the command is missing, the script cannot
    /// be read, QuickJS fails, or task fd writes fail.
    pub fn run_task(&self, task: &Task) -> FsResult<RunOutput> {
        let command = task_command(task)?;
        let script_path = command.program.clone();
        let script_filename = script_path.to_string();
        let namespace = task.namespace();
        let source = read_namespace_file(&namespace, &script_path)?;
        let host = QuickJsWanixConfig::new(task_wasi_config(task));
        let exit_state = WanixExitState::default();
        let wasi_host =
            WanixQuickJsWasiHost::new_with_exit_state(host.wasi().clone(), exit_state.clone())
                .map_err(|err| {
                    FsError::Other(format!(
                        "failed to create Wanix-backed QuickJS WASI host: {err:?}"
                    ))
                })?;
        let create_options = captured_stdio_options().with_wasi_host(wasi_host);
        let run_as_module = uses_module_syntax(&source);
        let api_exit_state = exit_state.clone();
        let api_task = task.clone();
        let script_args = task_wasi_argv(task);
        let context = WanixTaskContext::new(
            command.raw,
            script_args,
            command.args,
            task_env_map(task),
            command.cwd.clone(),
        );
        match self.run_with_setup_control(
            &source,
            create_options,
            Some(exit_state.clone()),
            Some(task.clone()),
            move |runtime| {
                define_wanix_module_loader(runtime, namespace.clone())?;
                define_wanix_host_api(
                    runtime,
                    namespace,
                    context,
                    Some(api_exit_state.clone()),
                    Some(api_task.clone()),
                )
            },
            |runtime, source| {
                if run_as_module {
                    runtime
                        .eval_module_discard(source, &script_filename)
                        .map_err(qjs_error)
                } else {
                    runtime.eval_discard(source).map_err(qjs_error)
                }
            },
        ) {
            Ok(output) => {
                write_task_output(task, &output)?;
                let exit_code = exit_state.code()?.unwrap_or(0);
                task.set_exit(exit_code.to_string())?;
                Ok(output)
            }
            Err(failure) => {
                write_task_output(task, &failure.output)?;
                if let Some(exit_code) = exit_state.code()? {
                    task.set_exit(exit_code.to_string())?;
                    return Ok(failure.output);
                }
                Err(failure.error)
            }
        }
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

fn captured_stdio_options() -> QuickJsCreateOptions {
    create_options_with_config(captured_stdio_config())
}

fn create_options_with_config(config: QuickJsHostConfig) -> QuickJsCreateOptions {
    QuickJsCreateOptions::new().with_host_config(config)
}

fn captured_stdio_options_with_wanix_wasi(
    config: QuickJsWanixConfig,
) -> FsResult<QuickJsCreateOptions> {
    Ok(captured_stdio_options().with_wasi_host(wanix_wasi_host(config)?))
}

fn create_options_with_wanix_wasi(config: QuickJsWanixConfig) -> FsResult<QuickJsCreateOptions> {
    Ok(QuickJsCreateOptions::new().with_wasi_host(wanix_wasi_host(config)?))
}

fn restore_options_with_wanix_wasi(config: QuickJsWanixConfig) -> FsResult<QuickJsRestoreOptions> {
    Ok(QuickJsRestoreOptions::new().with_wasi_host(wanix_wasi_host(config)?))
}

fn wanix_wasi_host(config: QuickJsWanixConfig) -> FsResult<WanixQuickJsWasiHost> {
    WanixQuickJsWasiHost::new(config.wasi).map_err(wanix_wasi_host_error)
}

fn wanix_wasi_host_error(err: wanix_wasi::Errno) -> FsError {
    FsError::Other(format!(
        "failed to create Wanix-backed QuickJS WASI host: {err:?}"
    ))
}

fn define_task_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
    exit_state: Option<WanixExitState>,
    output_fd: Option<(Task, Fd)>,
) -> FsResult<()> {
    match (exit_state, output_fd) {
        (Some(exit_state), Some((task, fd))) => {
            define_fd_output_callback(runtime, name, task, fd, exit_state)
        }
        (Some(exit_state), None) => {
            define_output_callback_with_exit_state(runtime, name, output, exit_state)
        }
        (None, _) => define_output_callback(runtime, name, output),
    }
}

fn exit_requested(exit_state: &Option<WanixExitState>) -> FsResult<bool> {
    match exit_state {
        Some(exit_state) => exit_state.code().map(|code| code.is_some()),
        None => Ok(false),
    }
}

fn uses_module_syntax(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("export ")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskCommand {
    raw: String,
    program: NormalizedPath,
    args: Vec<String>,
    cwd: NormalizedPath,
}

fn task_command(task: &Task) -> FsResult<TaskCommand> {
    let raw = task.cmd();
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        let program = resolve_from_cwd(&spec.cwd, &spec.program)?;
        let raw = if raw.is_empty() {
            raw_command(&spec.program, &spec.args)
        } else {
            raw
        };
        return Ok(TaskCommand {
            raw,
            program,
            args: spec.args,
            cwd: spec.cwd,
        });
    }

    let cwd = task.dir();
    let argv = task
        .cmd_argv()
        .ok_or_else(|| FsError::Other("qjs task cmd is empty".to_owned()))?;
    let (script, args) = argv
        .split_first()
        .ok_or_else(|| FsError::Other("qjs task cmd is empty".to_owned()))?;
    let program = resolve_from_cwd(&cwd, &NormalizedPath::new(script)?)?;
    Ok(TaskCommand {
        raw,
        program,
        args: args.to_vec(),
        cwd,
    })
}

pub(crate) fn task_wasi_argv(task: &Task) -> Vec<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return std::iter::once(spec.program.to_string())
            .chain(spec.args)
            .collect();
    }
    task.cmd_argv().unwrap_or_default()
}

pub(crate) fn task_wasi_env(task: &Task) -> Vec<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return spec
            .env
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
    }
    task.env()
}

pub(crate) fn task_wasi_cwd(task: &Task) -> NormalizedPath {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return spec.cwd;
    }
    task.dir()
}

pub(crate) fn task_program_for_check(task: &Task) -> Option<String> {
    let spec = task.spec();
    if task_spec_is_set(&spec) {
        return Some(spec.program.to_string());
    }
    task.cmd_argv()
        .and_then(|argv| argv.first().map(ToOwned::to_owned))
}

fn task_spec_is_set(spec: &TaskSpec) -> bool {
    spec.program.as_str() != "."
}

fn raw_command(program: &NormalizedPath, args: &[String]) -> String {
    quote_cmd_argv(std::iter::once(program.as_str()).chain(args.iter().map(String::as_str)))
}

fn resolve_from_cwd(cwd: &NormalizedPath, path: &NormalizedPath) -> FsResult<NormalizedPath> {
    if cwd.as_str() == "." {
        return Ok(path.clone());
    }
    if path.as_str() == "." {
        return Ok(cwd.clone());
    }
    NormalizedPath::new(format!("{cwd}/{path}"))
}

fn task_env_map(task: &Task) -> BTreeMap<String, String> {
    task_wasi_env(task)
        .into_iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.is_empty() {
                return None;
            }
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn write_task_output(task: &Task, output: &RunOutput) -> FsResult<()> {
    if !output.stdout.is_empty() {
        task.write_fd(Fd::STDOUT, &output.stdout)?;
    }
    if !output.stderr.is_empty() {
        task.write_fd(Fd::STDERR, &output.stderr)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use super::{
        CRATE_PURPOSE, FIRST_DEMO_TARGET, QuickJsRunner, QuickJsTaskDriver, QuickJsWanixConfig,
        wasi_contract_purpose,
    };
    use crate::task_stdio::task_wasi_config;
    use wanix_fs::{FileSystem, FsError, MemFs, NormalizedPath, OpenOptions};
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
    fn runner_executes_source_and_captures_console_output() {
        let output = runner()
            .run_source(r#"print("hello", 42); console.error("oops");"#)
            .unwrap();

        assert_eq!(output.stdout(), b"hello 42\n");
        assert_eq!(output.stderr(), b"oops\n");
    }

    #[test]
    fn runner_exposes_wanix_namespace_to_javascript() {
        let namespace = wanix_vfs::Namespace::new();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("input.txt", b"from wanix").unwrap();
        let mut namespace = namespace;
        namespace
            .bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();

        let output = runner()
            .run_source_with_namespace(
                r#"
const text = Wanix.readText("input.txt");
Wanix.writeText("output.txt", text + " / qjs");
print(Wanix.readText("output.txt"));
print("exit api", typeof Wanix.exit);
print("fd api", typeof Wanix.open);
"#,
                namespace,
            )
            .unwrap();

        assert_eq!(
            output.stdout(),
            b"from wanix / qjs\nexit api undefined\nfd api undefined\n"
        );
        assert_eq!(read_file(&*root, "output.txt"), b"from wanix / qjs");
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
const text = Wanix.readText("input.txt");
Wanix.writeText("output.txt", text + " / qjs");
print(Wanix.readText("output.txt"));
"#,
                config,
            )
            .unwrap();

        assert_eq!(output.stdout(), b"from config / qjs\n");
        assert_eq!(read_file(&*root, "output.txt"), b"from config / qjs");
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
print(Wanix.readText("input.txt"));
"#,
                config,
            )
            .unwrap();

        assert_eq!(output.stdout(), b"from config\n");
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

        let command = super::task_command(&task).unwrap();
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

        let command = super::task_command(&task).unwrap();
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

        let output = runner()
            .run_module_with_namespace(
                r#"
import { message } from "./lib.js";

print(message);
"#,
                "main.js",
                namespace,
            )
            .unwrap();

        assert_eq!(output.stdout(), b"from namespace modules\n");
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
            br#"
const text = Wanix.readText("input.txt");
Wanix.writeText("generated.txt", text + " via task");
print("task", Wanix.readText("generated.txt"));
console.error("stderr", "line");
"#,
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
    fn task_driver_exposes_wanix_task_context_to_javascript() {
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
print("cmd", Wanix.cmd());
print("cwd", Wanix.cwd());
print("args", Wanix.args().join("|"));
print("env", Wanix.env("MODE"), Wanix.env().EMPTY === "", String(Wanix.env("MISSING")));
const source = Wanix.readText("main.js");
Wanix.writeText("out.txt", "cwd write");
print("source", source.includes("Wanix.readText"));
print("id", Wanix.readText("#task/self/id").trim());
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
        task.set_env_lines("MODE=test\nEMPTY=\nBROKEN\nMODE=override")
            .unwrap();
        task.set_dir("app").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(
            read_file(&*stdout, "out"),
            b"cmd main.js alpha beta\ncwd app\nargs alpha|beta\nenv override true undefined\nsource true\nid 1\n"
        );
        assert_eq!(read_file(&*root, "app/out.txt"), b"cwd write");
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_maps_wanix_exit_to_task_status() {
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
print("before exit");
console.error("stderr before exit");
Promise.resolve().then(() => print("after queued job"));
try {
  Wanix.exit(7);
} catch (err) {
  print("after exit");
  console.error("stderr after exit");
  Wanix.writeText("after.txt", "should not persist");
  const fd = Wanix.open("fd-after.txt", "w+");
  Wanix.writeFd(fd, "should not persist");
  Wanix.closeFd(fd);
  Promise.resolve().then(() => print("after job"));
}
"#,
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

        assert_eq!(read_file(&*stdout, "out"), b"before exit\n");
        assert_eq!(read_file(&*stderr, "err"), b"stderr before exit\n");
        assert!(matches!(
            root.read_file("after.txt"),
            Err(wanix_fs::FsError::NotFound)
        ));
        assert!(matches!(
            root.read_file("fd-after.txt"),
            Err(wanix_fs::FsError::NotFound)
        ));
        assert_eq!(task.exit(), "7");
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
    fn task_driver_exposes_wanix_owned_fd_table_to_javascript() {
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
const input = Wanix.open("input.txt", "r");
print("input fd", input);
print("read", Wanix.readFd(input, 64));
Wanix.closeFd(input);

const output = Wanix.open("created.txt", "w+");
print("output fd", output);
print("wrote", Wanix.writeFd(output, "created via fd"));
Wanix.closeFd(output);
"#,
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
            b"input fd 3\nread from fd\noutput fd 4\nwrote 14\n"
        );
        assert_eq!(
            root.read_file("app/created.txt").unwrap(),
            b"created via fd"
        );
        assert_eq!(task.fd_numbers(), [Fd::STDOUT]);
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_fd_api_writes_to_wanix_stdio_fds() {
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
Wanix.writeFd(1, "direct stdout\n");
Wanix.writeFd(2, "direct stderr\n");
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

        assert_eq!(read_file(&*stdout, "out"), b"direct stdout\n");
        assert_eq!(read_file(&*stderr, "err"), b"direct stderr\n");
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
            b"argv main.js|alpha|beta\nmode test\nenv test true undefined\nstdin hello from fd0\nsource true\ncreated made via std cwd\n"
        );
        assert_eq!(
            root.read_file("app/created.txt").unwrap(),
            b"made via std cwd"
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
        assert_eq!(task.fd_numbers(), [Fd::STDOUT]);
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
    + " wanix=" + Wanix.env("MODE")
    + " " + std.loadFile("#task/self/id").trim()
    + " " + suffix
    + "\n"
);
Wanix.writeText("after.txt", Wanix.args().join("|"));
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
            b"after from task cwd wasi=before wanix=after 1 restored\n"
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
    fn task_runtime_snapshot_rejects_after_wanix_exit() {
        let runner = runner();
        let table = TaskTable::new();
        table.register_noop_driver("qjs").unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = Arc::new(MemFs::new());
        root.write_file("main.js", b"").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js").unwrap();
        let mut runtime = runner.create_task_runtime(&task).unwrap();

        runtime.eval_discard("Wanix.exit(5);").unwrap();
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

print("source", std.loadFile("main.js").includes("qjs:std"));
print("id", std.loadFile("#task/self/id").trim());
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
        task.set_dir("app").unwrap();

        table.start(task.id()).unwrap();

        assert_eq!(read_file(&*stdout, "out"), b"source true\nid 1\n");
        assert_eq!(task.exit(), "0");
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
    fn task_driver_fd_api_preserves_stdio_order_and_close() {
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
print("a");
Wanix.writeFd(1, "b\n");
print("c");
Wanix.closeFd(1);
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

        assert_eq!(read_file(&*stdout, "out"), b"a\nb\nc\n");
        assert!(task.fd_numbers().is_empty());
        assert_eq!(task.exit(), "0");
    }

    #[test]
    fn task_driver_rejects_invalid_wanix_fd_calls() {
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
try {
  Wanix.open("input.txt", "bad");
} catch (err) {
  Wanix.writeText("bad-mode.txt", "caught");
}
try {
  Wanix.readFd(1.5, 1);
} catch (err) {
  Wanix.writeText("bad-fd.txt", "caught");
}
try {
  Wanix.readFd(1, 1048577);
} catch (err) {
  Wanix.writeText("bad-len.txt", "caught");
}
const utf8 = Wanix.open("utf8.txt", "r");
try {
  Wanix.readFd(utf8, 1);
} catch (err) {
  Wanix.writeText("split-utf8.txt", "caught");
} finally {
  Wanix.closeFd(utf8);
}
const fd = Wanix.open("input.txt", "r");
Wanix.closeFd(fd);
Wanix.readFd(fd, 1);
"#,
        )
        .unwrap();
        root.write_file("input.txt", b"from fd").unwrap();
        root.write_file("utf8.txt", [0xc3, 0xa9]).unwrap();
        task.bind(root.clone(), ".", ".", BindOptions::default())
            .unwrap();
        task.set_cmd("main.js").unwrap();

        let err = table.start(task.id()).unwrap_err();

        assert!(err.to_string().contains("Wanix.readFd"));
        assert_eq!(root.read_file("bad-mode.txt").unwrap(), b"caught");
        assert_eq!(root.read_file("bad-fd.txt").unwrap(), b"caught");
        assert_eq!(root.read_file("bad-len.txt").unwrap(), b"caught");
        assert_eq!(root.read_file("split-utf8.txt").unwrap(), b"caught");
        assert_eq!(task.exit(), "1");
    }

    #[test]
    fn task_driver_rejects_invalid_wanix_exit_status() {
        let table = TaskTable::new();
        let runner = runner();
        table
            .register_driver("qjs", std::sync::Arc::new(QuickJsTaskDriver::new(runner)))
            .unwrap();
        let task = table.allocate_root("qjs").unwrap();
        let root = std::sync::Arc::new(MemFs::new());
        root.write_file("main.js", br#"Wanix.exit(999);"#).unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task.set_cmd("main.js").unwrap();

        let err = table.start(task.id()).unwrap_err();

        assert!(err.to_string().contains("Wanix.exit expects"));
        assert_eq!(task.exit(), "1");
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
}
