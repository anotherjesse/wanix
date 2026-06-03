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

use rust_wasi_quickjs::{QuickJsHostConfig, QuickJsModule, QuickJsRuntime};
use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath};
use wanix_task::{Fd, Task};
use wanix_wasi::WasiConfig;
use wasmtime::Engine;

mod driver;
mod host_api;
mod task_context;
mod virtual_wasi;

pub use driver::QuickJsTaskDriver;

use host_api::{
    define_output_callback, define_output_callback_with_exit_state, define_wanix_host_api,
    define_wanix_module_loader, define_wanix_namespace_api, qjs_error, read_namespace_file,
    take_buffer,
};
use task_context::{WanixExitState, WanixTaskContext};
use virtual_wasi::with_namespace_read_only_files;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "quickjs wasi task driver";

/// First externally visible demo target for this crate.
pub const FIRST_DEMO_TARGET: &str =
    "run JavaScript outside Chrome with access to a Wanix namespace";

/// Host configuration for QuickJS/WASI task execution.
#[derive(Debug, Clone)]
pub struct HostConfig {
    wasi: WasiConfig,
}

impl HostConfig {
    /// Creates a host config from Wanix-backed WASI settings.
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

    /// Runs JavaScript source outside Chrome and captures `print`/`console` output.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS creation, host callback setup,
    /// or JavaScript evaluation fails.
    pub fn run_source(&self, source: &str) -> FsResult<RunOutput> {
        self.run_source_with_setup(source, None, |_| Ok(()))
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
        self.run_source_with_setup(source, Some(config), move |runtime| {
            define_wanix_module_loader(runtime, namespace.clone())?;
            define_wanix_namespace_api(runtime, namespace)
        })
        .map_err(|failure| failure.error)
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
            Some(config),
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
        config: Option<QuickJsHostConfig>,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        self.run_with_setup(source, config, setup, |runtime, source| {
            runtime.eval_discard(source).map_err(qjs_error)
        })
    }

    fn run_with_setup(
        &self,
        source: &str,
        config: Option<QuickJsHostConfig>,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
        eval: impl FnOnce(&mut QuickJsRuntime, &str) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        self.run_with_setup_control(source, config, None, setup, eval)
    }

    fn run_with_setup_control(
        &self,
        source: &str,
        config: Option<QuickJsHostConfig>,
        exit_state: Option<WanixExitState>,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
        eval: impl FnOnce(&mut QuickJsRuntime, &str) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        let mut runtime = self
            .module
            .create_runtime_with_host_config(config.unwrap_or_else(captured_stdio_config))
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
            )?;
            define_task_output_callback(
                &mut runtime,
                "__wanix_stderr",
                Arc::clone(&stderr),
                exit_state.clone(),
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
        let config = with_namespace_read_only_files(captured_stdio_config(), &namespace)?;
        let run_as_module = uses_module_syntax(&source);
        let exit_state = WanixExitState::default();
        let api_exit_state = exit_state.clone();
        let context =
            WanixTaskContext::new(command.raw, command.args, task_env_map(task), task.dir());
        match self.run_with_setup_control(
            &source,
            Some(config),
            Some(exit_state.clone()),
            move |runtime| {
                define_wanix_module_loader(runtime, namespace.clone())?;
                define_wanix_host_api(runtime, namespace, context, Some(api_exit_state.clone()))
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

fn define_task_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
    exit_state: Option<WanixExitState>,
) -> FsResult<()> {
    match exit_state {
        Some(exit_state) => {
            define_output_callback_with_exit_state(runtime, name, output, exit_state)
        }
        None => define_output_callback(runtime, name, output),
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
}

fn task_command(task: &Task) -> FsResult<TaskCommand> {
    let raw = task.cmd();
    let mut parts = raw.split_whitespace();
    let script = parts
        .next()
        .ok_or_else(|| FsError::Other("qjs task cmd is empty".to_owned()))?;
    let program = resolve_from_cwd(&task.dir(), &NormalizedPath::new(script)?)?;
    let args = parts.map(str::to_owned).collect();
    Ok(TaskCommand { raw, program, args })
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
    task.env()
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
    use std::path::PathBuf;

    use super::{
        CRATE_PURPOSE, FIRST_DEMO_TARGET, HostConfig, QuickJsRunner, QuickJsTaskDriver,
        wasi_contract_purpose,
    };
    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_task::{Fd, Task, TaskDriver, TaskId, TaskSpec, TaskTable};
    use wanix_vfs::BindOptions;
    use wanix_wasi::WasiConfig;

    fn prototype_wasm() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust-wasi-quickjs/fixtures/quickjs.wasm")
    }

    fn runner() -> QuickJsRunner {
        QuickJsRunner::from_wasm_file(prototype_wasm()).unwrap()
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
    fn host_config_wraps_wanix_wasi_settings() {
        let config = HostConfig::new(WasiConfig::default());

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
"#,
                namespace,
            )
            .unwrap();

        assert_eq!(output.stdout(), b"from wanix / qjs\nexit api undefined\n");
        assert_eq!(read_file(&*root, "output.txt"), b"from wanix / qjs");
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
        let runner = std::sync::Arc::new(runner());
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
        let runner = std::sync::Arc::new(runner());
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
        let runner = std::sync::Arc::new(runner());
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
        let runner = std::sync::Arc::new(runner());
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
        assert_eq!(task.exit(), "7");
    }

    #[test]
    fn task_driver_rejects_invalid_wanix_exit_status() {
        let table = TaskTable::new();
        let runner = std::sync::Arc::new(runner());
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
        let driver = QuickJsTaskDriver::new(std::sync::Arc::new(runner()));
        let task = Task::new(
            TaskId::new(1),
            TaskSpec::new("main.js").unwrap(),
            wanix_vfs::Namespace::new(),
        );

        task.set_cmd("main.js\t--flag").unwrap();
        assert!(driver.check(&task));

        task.set_cmd("notes.txt main.js").unwrap();
        assert!(!driver.check(&task));
    }

    #[test]
    fn task_driver_sets_nonzero_exit_on_failure() {
        let table = TaskTable::new();
        let runner = std::sync::Arc::new(runner());
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
        let runner = std::sync::Arc::new(runner());
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
