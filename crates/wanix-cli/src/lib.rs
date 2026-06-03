//! Native CLI plumbing for Rust Wanix demos.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::OnceLock;

use wanix_fs::{FileSystem, FsError, MemFs, NormalizedPath, OpenOptions};
use wanix_qjs::{QuickJsRunner, QuickJsTaskDriver};
use wanix_task::{Fd, TaskSpec, TaskTable};
use wanix_vfs::BindOptions;

const USAGE: &str = "usage: wanix-rust qjs [--env KEY=VALUE ...] [--cwd DIR] [--stdin TEXT] <script.js> [-- arg ...]\n       wanix-rust --help";
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
        [command, rest @ ..] if command == "qjs" => run_qjs(parse_qjs_command(rest)?),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct QjsCommand {
    script_path: PathBuf,
    args: Vec<String>,
    env: Vec<String>,
    cwd: NormalizedPath,
    stdin: Option<Vec<u8>>,
}

fn run_qjs(command: QjsCommand) -> Result<CliOutput, CliError> {
    let script_path = command.script_path.as_path();
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
    let runner = quickjs_runner()?;
    table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(runner)))?;
    let task = table.allocate_root("qjs")?;
    let task_spec = qjs_task_spec(&command)?;
    let task_cmd = qjs_task_cmd(&command.args);
    let task_env = command.env.join("\n");
    let task_dir = command.cwd.to_string();

    let root = Arc::new(MemFs::new());
    copy_script_directory(script_path, &root, &command.cwd)?;
    let guest_script = guest_path_in_cwd(&command.cwd, QJS_GUEST_SCRIPT)?;
    root.write_file(guest_script.as_str(), script.as_bytes())?;
    task.bind(root, ".", ".", BindOptions::default())?;

    if let Some(stdin_bytes) = command.stdin {
        let stdin = Arc::new(MemFs::new());
        stdin.write_file("stdin", stdin_bytes)?;
        task.insert_fd(
            Fd::STDIN,
            stdin.open(&NormalizedPath::new("stdin")?, OpenOptions::read())?,
            NormalizedPath::new("stdin")?,
        )?;
    }

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
    task.set_spec(task_spec)?;
    task.set_cmd(task_cmd)?;
    task.set_env_lines(task_env)?;
    task.set_dir(task_dir)?;

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

fn parse_qjs_command(args: &[OsString]) -> Result<QjsCommand, CliError> {
    let mut env = Vec::new();
    let mut cwd = NormalizedPath::new(".")?;
    let mut stdin = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--env" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs --env expects KEY=VALUE"))?;
            let value = os_arg_to_string(value, "qjs --env")?;
            validate_env_line(&value)?;
            env.push(value);
            i += 1;
        } else if args[i] == "--cwd" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs --cwd expects a Wanix path"))?;
            cwd = NormalizedPath::new(os_arg_to_string(value, "qjs --cwd")?)?;
            i += 1;
        } else if args[i] == "--stdin" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| CliError::usage("qjs --stdin expects text"))?;
            stdin = Some(os_arg_to_string(value, "qjs --stdin")?.into_bytes());
            i += 1;
        } else if args[i] == "--" {
            i += 1;
            break;
        } else {
            break;
        }
    }

    let script = args
        .get(i)
        .ok_or_else(|| CliError::usage("qjs expects a script path"))?;
    let script_path = PathBuf::from(script);
    i += 1;

    if args.get(i).is_some_and(|arg| arg == "--") {
        i += 1;
    }

    let js_args = args[i..]
        .iter()
        .map(|arg| os_arg_to_string(arg, "qjs script arg"))
        .collect::<Result<Vec<_>, CliError>>()?;

    Ok(QjsCommand {
        script_path,
        args: js_args,
        env,
        cwd,
        stdin,
    })
}

fn os_arg_to_string(arg: &OsString, label: &str) -> Result<String, CliError> {
    arg.clone()
        .into_string()
        .map_err(|_| CliError::usage(format!("{label} must be valid UTF-8")))
}

fn validate_env_line(line: &str) -> Result<(), CliError> {
    let Some((key, _value)) = line.split_once('=') else {
        return Err(CliError::usage("qjs --env expects KEY=VALUE"));
    };
    if key.is_empty() || key.chars().any(char::is_whitespace) || line.contains('\n') {
        return Err(CliError::usage("qjs --env expects KEY=VALUE"));
    }
    Ok(())
}

fn qjs_task_cmd(args: &[String]) -> String {
    std::iter::once(QJS_GUEST_SCRIPT)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn qjs_task_spec(command: &QjsCommand) -> Result<TaskSpec, CliError> {
    let mut spec = TaskSpec::new(QJS_GUEST_SCRIPT)?;
    spec.args = command.args.clone();
    spec.env = env_map(&command.env);
    spec.cwd = command.cwd.clone();
    Ok(spec)
}

fn env_map(lines: &[String]) -> BTreeMap<String, String> {
    lines
        .iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn quickjs_runner() -> Result<Arc<QuickJsRunner>, CliError> {
    match std::env::var_os("WANIX_QJS_WASM") {
        Some(path) => QuickJsRunner::from_wasm_file(PathBuf::from(path)).map(Arc::new),
        None => bundled_quickjs_runner(),
    }
    .map_err(CliError::from)
}

#[cfg(not(test))]
fn bundled_quickjs_runner() -> Result<Arc<QuickJsRunner>, FsError> {
    QuickJsRunner::from_bundled_wasm().map(Arc::new)
}

#[cfg(test)]
fn bundled_quickjs_runner() -> Result<Arc<QuickJsRunner>, FsError> {
    static RUNNER: OnceLock<Result<Arc<QuickJsRunner>, String>> = OnceLock::new();
    match RUNNER.get_or_init(|| {
        QuickJsRunner::from_bundled_wasm()
            .map(Arc::new)
            .map_err(|err| err.to_string())
    }) {
        Ok(runner) => Ok(Arc::clone(runner)),
        Err(error) => Err(FsError::Other(error.clone())),
    }
}

fn copy_script_directory(
    script_path: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
    let base = script_path.parent().unwrap_or_else(|| Path::new("."));
    copy_directory_tree(base, base, root, cwd)
}

fn copy_directory_tree(
    base: &Path,
    dir: &Path,
    root: &MemFs,
    cwd: &NormalizedPath,
) -> Result<(), CliError> {
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
            copy_directory_tree(base, &path, root, cwd)?;
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
        let guest_path = guest_path_in_cwd(cwd, &guest_path)?;
        root.write_file(guest_path, bytes)?;
    }
    Ok(())
}

fn guest_path_in_cwd(cwd: &NormalizedPath, path: &str) -> Result<String, CliError> {
    if cwd.as_str() == "." {
        return Ok(path.to_owned());
    }
    Ok(NormalizedPath::new(format!("{cwd}/{path}"))?.to_string())
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
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::run;

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn help_mentions_qjs_demo_target() {
        let output = run(["--help"]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert!(String::from_utf8_lossy(output.stdout()).contains("wanix-rust qjs"));
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_runs_script_outside_chrome() {
        let script = write_temp_script(
            "demo script.js",
            r##"
import * as std from "qjs:std";
import { runtime } from "./lib.js";

const text = std.loadFile("main.js");
std.writeFile("created.txt", "made inside Wanix");
std.out.puts("task " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts(runtime + "\n");
std.out.puts(text.includes("made inside Wanix") + " " + std.loadFile("created.txt") + "\n");
std.out.flush();
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
    fn qjs_command_runs_quickjs_std_stdout_outside_chrome() {
        let script = write_temp_script(
            "std-demo.js",
            r#"
import * as std from "qjs:std";

std.out.puts("hello std\n");
std.out.flush();
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"hello std\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_reads_script_sibling_with_quickjs_std_load_file() {
        let script = write_temp_script(
            "std-read-demo.js",
            r#"
import * as std from "qjs:std";

print(std.loadFile("input.txt"));
"#,
        );
        fs::write(script.parent().unwrap().join("input.txt"), "hello std file").unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"hello std file\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_reads_task_id_with_quickjs_std_load_file() {
        let script = write_temp_script(
            "std-task-id-demo.js",
            r##"
import * as std from "qjs:std";

print("source", std.loadFile("main.js").includes("qjs:std"));
print("id", std.loadFile("#task/self/id").trim());
"##,
        );

        let output = run([
            "qjs".into(),
            "--cwd".into(),
            "app".into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"source true\nid 1\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_writes_script_sibling_with_quickjs_std_write_file() {
        let script = write_temp_script(
            "std-write-demo.js",
            r#"
import * as std from "qjs:std";

std.writeFile("created.txt", "hello from std write");
print(std.loadFile("created.txt"));
"#,
        );

        let output = run(["qjs".into(), script.clone().into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(output.stdout(), b"hello from std write\n");
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_uses_quickjs_std_and_os_for_process_context() {
        let script = write_temp_script(
            "std-process-demo.js",
            r#"
import * as std from "qjs:std";
import * as os from "qjs:os";

const readStdin = () => {
  const bytes = new Uint8Array(64);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  return Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
};

const env = std.getenviron();
std.out.puts("argv " + scriptArgs.join("/") + "\n");
std.out.puts("mode " + std.getenv("MODE") + "\n");
std.out.puts("env " + env.MODE + " " + (env.EMPTY === "") + " " + String(env.MISSING) + "\n");
std.out.puts("stdin " + readStdin() + "\n");
std.out.puts("source " + std.loadFile("main.js").includes("std.getenv") + "\n");
std.writeFile("created.txt", "made via std cwd");
std.out.puts("created " + std.loadFile("created.txt") + "\n");
std.out.flush();
"#,
        );

        let output = run([
            "qjs".into(),
            "--env".into(),
            "MODE=test".into(),
            "--env".into(),
            "EMPTY=".into(),
            "--cwd".into(),
            "app".into(),
            "--stdin".into(),
            "hello from fd0".into(),
            script.into_os_string(),
            "--".into(),
            "alpha".into(),
            "two words".into(),
            "beta".into(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"argv main.js/alpha/two words/beta\nmode test\nenv test true undefined\nstdin hello from fd0\nsource true\ncreated made via std cwd\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_starts_child_qjs_task_through_task_service() {
        let script = write_temp_script(
            "spawn-parent.js",
            r##"
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
  const bytes = new Uint8Array(64);
  const count = os.read(fd, bytes.buffer, 0, bytes.length);
  os.close(fd);
  if (count < 0) {
    throw new Error("read " + path + ": " + count);
  }
  return stringFromBytes(bytes, count);
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
writeServiceText("#task/" + child + "/cmd", "spawn-child.js alpha beta\n");
writeServiceText("#task/" + child + "/env", "MODE=child\n");
writeServiceText("#task/" + child + "/dir", ".\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/1 fd/1\n");
writeServiceText("#task/" + child + "/ctl", "bind #task/" + parent + "/fd/2 fd/2\n");
print("parent " + parent);
print("child " + child);
writeServiceText("#task/" + child + "/ctl", "start\n");
print("child exit " + readServiceText("#task/" + child + "/exit").trim());
"##,
        );
        fs::write(
            script.parent().unwrap().join("spawn-child.js"),
            r##"
import * as std from "qjs:std";

std.out.puts(
  "id " + std.loadFile("#task/self/id").trim()
    + " args " + scriptArgs.join("|")
    + " mode " + std.getenv("MODE")
    + "\n"
);
std.out.flush();
std.err.puts("stderr mode " + std.getenv("MODE") + "\n");
std.err.flush();
std.exit(5);
"##,
        )
        .unwrap();

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"parent 1\nchild 2\nid 2 args spawn-child.js|alpha|beta mode child\nchild exit 5\n"
        );
        assert_eq!(output.stderr(), b"stderr mode child\n");
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

    #[test]
    fn qjs_command_uses_exit_status_requested_by_javascript() {
        let script = write_temp_script(
            "exit.js",
            r#"
print("before exit");
console.error("stderr before exit");
Wanix.exit(9);
print("after exit");
console.error("stderr after exit");
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 9);
        assert_eq!(output.stdout(), b"before exit\n");
        assert_eq!(output.stderr(), b"stderr before exit\n");
    }

    #[test]
    fn qjs_command_uses_exit_status_requested_by_quickjs_std() {
        let script = write_temp_script(
            "std-exit.js",
            r#"
import * as std from "qjs:std";

std.out.puts("before std exit\n");
std.out.flush();
std.err.puts("stderr before std exit\n");
std.err.flush();
std.exit(9);
std.out.puts("after std exit\n");
std.err.puts("stderr after std exit\n");
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 9);
        assert_eq!(output.stdout(), b"before std exit\n");
        assert_eq!(output.stderr(), b"stderr before std exit\n");
    }

    #[test]
    fn qjs_command_runs_fd_demo_through_wanix_task_fds() {
        let script = write_temp_script(
            "fd-demo.js",
            r#"
const input = Wanix.open("main.js", "r");
print("read fd", input);
print("saw api", Wanix.readFd(input, 80).includes("Wanix.open"));
Wanix.closeFd(input);

const output = Wanix.open("fd-output.txt", "w+");
print("write fd", output);
print("bytes", Wanix.writeFd(output, "via cli fd"));
Wanix.closeFd(output);
print(Wanix.readText("fd-output.txt"));
"#,
        );

        let output = run(["qjs".into(), script.into_os_string()]).unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"read fd 3\nsaw api true\nwrite fd 4\nbytes 10\nvia cli fd\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_attaches_stdin_as_wanix_task_fd_zero() {
        let script = write_temp_script(
            "stdin-demo.js",
            r##"
print("stdin", Wanix.readFd(0, 1024));
print("again", JSON.stringify(Wanix.readFd(0, 1024)));
print("task", Wanix.readText("#task/self/id").trim());
"##,
        );

        let output = run([
            "qjs".into(),
            "--stdin".into(),
            "hello from fd0".into(),
            script.into_os_string(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"stdin hello from fd0\nagain \"\"\ntask 1\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_flows_env_cwd_and_args_from_wanix_task_state() {
        let script = write_temp_script(
            "context.js",
            r##"
print("cmd", Wanix.cmd());
print("cwd", Wanix.cwd());
print("args", Wanix.args().join("/"));
print("mode", Wanix.env("MODE"));
print("all", Wanix.env().MODE);
print("source", Wanix.readText("main.js").includes("Wanix.cwd"));
Wanix.writeText("created.txt", "made in cwd");
print("created", Wanix.readText("created.txt"));
print("id", Wanix.readText("#task/self/id").trim());
"##,
        );

        let output = run([
            "qjs".into(),
            "--env".into(),
            "MODE=test".into(),
            "--cwd".into(),
            "app".into(),
            script.into_os_string(),
            "--".into(),
            "alpha".into(),
            "two words".into(),
            "beta".into(),
        ])
        .unwrap();

        assert_eq!(output.exit_code(), 0);
        assert_eq!(
            output.stdout(),
            b"cmd main.js alpha two words beta\ncwd app\nargs alpha/two words/beta\nmode test\nall test\nsource true\ncreated made in cwd\nid 1\n"
        );
        assert!(output.stderr().is_empty());
    }

    #[test]
    fn qjs_command_rejects_invalid_env_keys_and_missing_stdin_text() {
        let script = write_temp_script("context.js", "print('unused');");

        let env_error = run([
            "qjs".into(),
            "--env".into(),
            "BAD KEY=value".into(),
            script.clone().into_os_string(),
        ])
        .unwrap_err();
        assert_eq!(env_error.exit_code(), 2);
        assert!(env_error.to_string().contains("KEY=VALUE"));

        let stdin_error = run(["qjs", "--stdin"]).unwrap_err();
        assert_eq!(stdin_error.exit_code(), 2);
        assert!(stdin_error.to_string().contains("--stdin expects text"));
    }

    fn write_temp_script(name: &str, source: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        path.push(format!("wanix-cli-test-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path.push(name);
        fs::write(&path, source).unwrap();
        path
    }
}
