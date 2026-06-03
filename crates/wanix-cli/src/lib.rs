//! Native CLI plumbing for Rust Wanix demos.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wanix_fs::{FileSystem, FsError, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver};
use wanix_task::{Fd, TaskTable};
use wanix_vfs::BindOptions;

const USAGE: &str = "usage: wanix-rust qjs <script.js>\n       wanix-rust --help";
const QJS_GUEST_SCRIPT: &str = "main.js";

/// Captured native CLI output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

impl CliOutput {
    fn new(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    /// Returns stdout bytes that should be written to the native process.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns stderr bytes that should be written to the native process.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Returns the native process exit code.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

/// CLI execution error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    message: String,
    exit_code: i32,
}

impl CliError {
    fn new(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }

    fn usage(message: impl AsRef<str>) -> Self {
        Self::new(format!("{}\n\n{USAGE}", message.as_ref()), 2)
    }

    /// Returns the native process exit code for this error.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<FsError> for CliError {
    fn from(error: FsError) -> Self {
        Self::new(error.to_string(), 1)
    }
}

/// Runs the native CLI command and returns captured process output.
///
/// # Errors
///
/// Returns a CLI error when arguments are invalid, files cannot be read, or the
/// selected Wanix runtime cannot be initialized.
pub fn run<I, S>(args: I) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    match args.as_slice() {
        [] => Ok(help_output()),
        [help] if help == "--help" || help == "-h" => Ok(help_output()),
        [command, script] if command == "qjs" => run_qjs(Path::new(script)),
        [command, ..] if command == "qjs" => Err(CliError::usage("qjs expects one script path")),
        [command, ..] => Err(CliError::usage(format!(
            "unknown wanix-rust command: {}",
            command.to_string_lossy()
        ))),
    }
}

fn help_output() -> CliOutput {
    CliOutput::new(
        format!("wanix-rust: {}\n{USAGE}\n", wanix_qjs::FIRST_DEMO_TARGET).into_bytes(),
        Vec::new(),
        0,
    )
}

fn run_qjs(script_path: &Path) -> Result<CliOutput, CliError> {
    let script = std::fs::read(script_path).map_err(|error| {
        CliError::new(
            format!("failed to read {}: {error}", script_path.display()),
            1,
        )
    })?;
    let script = String::from_utf8(script).map_err(|error| {
        CliError::new(
            format!(
                "script {} is not valid UTF-8: {error}",
                script_path.display()
            ),
            1,
        )
    })?;

    let table = TaskTable::new();
    let runner = Arc::new(QuickJsRunner::from_wasm_file(quickjs_wasm_path())?);
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(runner)))?;
    let task = table.allocate_root("qjs")?;

    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root)?;
    root.write_file(QJS_GUEST_SCRIPT, script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;

    let stdout = Arc::new(MemFs::new());
    stdout.write_file("stdout", b"")?;
    task.insert_fd(
        Fd::STDOUT,
        stdout.open(&NormalizedPath::new("stdout")?, OpenOptions::read_write())?,
        NormalizedPath::new("stdout")?,
    )?;
    let stderr = Arc::new(MemFs::new());
    stderr.write_file("stderr", b"")?;
    task.insert_fd(
        Fd::STDERR,
        stderr.open(&NormalizedPath::new("stderr")?, OpenOptions::read_write())?,
        NormalizedPath::new("stderr")?,
    )?;
    task.set_cmd(QJS_GUEST_SCRIPT)?;

    let start_result = table.start(task.id());
    let stdout = read_file(&*stdout, "stdout")?;
    let mut stderr = read_file(&*stderr, "stderr")?;
    match start_result {
        Ok(()) => Ok(CliOutput::new(stdout, stderr, parse_exit(&task.exit()))),
        Err(error) => {
            if !stderr.is_empty() && !stderr.ends_with(b"\n") {
                stderr.push(b'\n');
            }
            stderr.extend_from_slice(format!("wanix-rust qjs: {error}\n").as_bytes());
            Ok(CliOutput::new(stdout, stderr, 1))
        }
    }
}

fn quickjs_wasm_path() -> PathBuf {
    std::env::var_os("WANIX_QJS_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../rust-wasi-quickjs/fixtures/quickjs.wasm")
        })
}

fn copy_script_directory(script_path: &Path, root: &MemFs) -> Result<(), CliError> {
    let base = script_path.parent().unwrap_or_else(|| Path::new("."));
    copy_directory_tree(base, base, root)
}

fn copy_directory_tree(base: &Path, dir: &Path, root: &MemFs) -> Result<(), CliError> {
    for entry in std::fs::read_dir(dir).map_err(|error| {
        CliError::new(
            format!("failed to read directory {}: {error}", dir.display()),
            1,
        )
    })? {
        let entry = entry.map_err(|error| {
            CliError::new(
                format!(
                    "failed to read directory entry in {}: {error}",
                    dir.display()
                ),
                1,
            )
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            CliError::new(format!("failed to stat {}: {error}", path.display()), 1)
        })?;
        if file_type.is_dir() {
            copy_directory_tree(base, &path, root)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(guest_path) = guest_path_for_host_file(base, &path)? else {
            continue;
        };
        let bytes = std::fs::read(&path).map_err(|error| {
            CliError::new(format!("failed to read {}: {error}", path.display()), 1)
        })?;
        root.write_file(guest_path, bytes)?;
    }
    Ok(())
}

fn guest_path_for_host_file(base: &Path, path: &Path) -> Result<Option<String>, CliError> {
    let relative = path.strip_prefix(base).map_err(|error| {
        CliError::new(
            format!(
                "failed to map {} under {}: {error}",
                path.display(),
                base.display()
            ),
            1,
        )
    })?;
    let Some(path) = relative.to_str() else {
        return Ok(None);
    };
    let path = path.replace(std::path::MAIN_SEPARATOR, "/");
    if path.is_empty() || NormalizedPath::new(&path).is_err() {
        return Ok(None);
    }
    Ok(Some(path))
}

fn read_file(fs: &dyn FileSystem, path: &str) -> Result<Vec<u8>, CliError> {
    let mut file = fs.open(&NormalizedPath::new(path)?, OpenOptions::read())?;
    let mut out = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn parse_exit(exit: &str) -> i32 {
    exit.trim().parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::run;

    #[test]
    fn help_mentions_qjs_demo_target() {
        let output = run(["--help"]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs <script.js>"));
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_runs_script_outside_chrome() {
        let script = write_temp_script(
            "demo script.js",
            r##"
import { runtime } from "./lib.js";

const text = Wanix.readText("main.js");
Wanix.writeText("created.txt", "made inside Wanix");
print("task", Wanix.readText("#task/self/id").trim());
print(runtime);
print(text.includes("made inside Wanix"), Wanix.readText("created.txt"));
"##,
        );
        fs::write(
            script.parent().unwrap().join("lib.js"),
            "export const runtime = 'Wanix ES module loader';",
        )
        .unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"task 1\nWanix ES module loader\ntrue made inside Wanix\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_reports_failure_and_preserves_stdout() {
        let script = write_temp_script(
            "boom.js",
            r#"print("before failure"); throw new Error("boom");"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 1);
        assert_eq!(output.stdout(), b"before failure\n");
        assert!(String::from_utf8_lossy(output.stderr()).contains("QuickJS error"));
    }

    fn write_temp_script(name: &str, source: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("wanix-cli-test-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        path.push(name);
        fs::write(&path, source).unwrap();
        path
    }
}
