//! The `wanix-sh` shell as a `wasm32-wasip1` command guest.
//!
//! It backs the shell's [`NamespaceOps`] surface with WASI: standard I/O for the
//! shell's own output, the `#task` device for launching child commands, and
//! `#pipe` for byte channels between pipeline stages. The Wanix `wanix-wasm`
//! driver wires this guest's fd 0/1/2 to the task's stdio.

use std::io::{Read, Write};

use wanix_sh::{InputSource, NamespaceOps, OutputSink, ShellError, ShellResult, SpawnSpec, run_shell};

struct WasiNamespace;

impl NamespaceOps for WasiNamespace {
    fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()> {
        // Flush explicitly: std's stdout is line-buffered, and interactive
        // prompts and byte-wise echo carry no trailing newline.
        let mut stdout = std::io::stdout();
        stdout
            .write_all(bytes)
            .and_then(|()| stdout.flush())
            .map_err(|err| ShellError::Io(err.to_string()))
    }

    fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()> {
        std::io::stderr()
            .write_all(bytes)
            .map_err(|err| ShellError::Io(err.to_string()))
    }

    fn read_stdin(&mut self, buf: &mut [u8]) -> ShellResult<usize> {
        // WASI fd_read on fd 0; the host parks until the backing device
        // (e.g. #term/<id>/program) has bytes, so this is a true blocking read.
        std::io::stdin()
            .read(buf)
            .map_err(|err| ShellError::Io(err.to_string()))
    }

    fn exists(&self, path: &str) -> ShellResult<bool> {
        match std::fs::metadata(path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(ShellError::Io(format!("{path}: {err}"))),
        }
    }

    fn pipe_new(&mut self) -> ShellResult<String> {
        read_service("#pipe/new")
    }

    fn pipe_read_all(&mut self, id: &str) -> ShellResult<Vec<u8>> {
        let mut file = std::fs::File::open(format!("#pipe/{id}/data"))
            .map_err(|err| ShellError::Io(format!("#pipe/{id}/data: {err}")))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|err| ShellError::Io(format!("#pipe/{id}/data: {err}")))?;
        Ok(bytes)
    }

    fn pipe_write_all_and_close(&mut self, id: &str, bytes: &[u8]) -> ShellResult<()> {
        // Open the write end only; dropping it closes the writer so the reader
        // observes EOF.
        let mut writer = std::fs::OpenOptions::new()
            .write(true)
            .open(format!("#pipe/{id}/data"))
            .map_err(|err| ShellError::Io(format!("#pipe/{id}/data: {err}")))?;
        writer
            .write_all(bytes)
            .map_err(|err| ShellError::Io(format!("#pipe/{id}/data: {err}")))
    }

    fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>> {
        std::fs::read(path).map_err(|err| ShellError::Io(format!("{path}: {err}")))
    }

    fn write_file(&mut self, path: &str, bytes: &[u8], append: bool) -> ShellResult<()> {
        let result = if append {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(bytes))
        } else {
            std::fs::write(path, bytes)
        };
        result.map_err(|err| ShellError::Io(format!("{path}: {err}")))
    }

    fn spawn(&mut self, spec: &SpawnSpec) -> ShellResult<i32> {
        // Allocate a child task whose kind is auto-selected by the program's
        // extension (`.wasm` -> wasm driver, `.js` -> qjs driver, …).
        let self_id = read_service("#task/self/id")?;
        let child = read_service("#task/new/auto")?;
        let base = format!("#task/{child}");

        write_service(&format!("{base}/cmd"), &command_line(spec))?;
        // A `>` to a file: pre-truncate so the child writing from offset 0 leaves
        // no stale tail. (Append to a file for externals is rejected upstream.)
        if let OutputSink::File { path, .. } = &spec.stdout {
            let path = path.clone();
            self.write_file(&path, b"", false)?;
        }
        if !spec.env.is_empty() {
            let env_lines = spec
                .env
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("\n");
            write_service(&format!("{base}/env"), &env_lines)?;
        }
        bind_fd(&base, 0, &input_bind(&spec.stdin, &self_id))?;
        bind_fd(&base, 1, &output_bind(&spec.stdout, &self_id))?;
        // stderr is always inherited.
        bind_fd(&base, 2, &(format!("#task/{self_id}/fd/2"), None))?;
        write_service(&format!("{base}/ctl"), "start")?;

        let exit = read_service(&format!("{base}/exit"))?;
        exit.parse::<i32>()
            .map_err(|_| ShellError::Io(format!("invalid exit status {exit:?}")))
    }
}

/// The (source path, open-mode) a child's stdin should bind to.
fn input_bind(stdin: &InputSource, self_id: &str) -> (String, Option<&'static str>) {
    match stdin {
        InputSource::Inherit => (format!("#task/{self_id}/fd/0"), None),
        InputSource::Pipe(id) => (format!("#pipe/{id}/data"), Some("r")),
        InputSource::File(path) => (path.clone(), Some("r")),
    }
}

/// The (sink path, open-mode) a child's stdout should bind to.
///
/// A `#pipe` write end (and a file fd 1) must open write-only — a pipe rejects a
/// read-write open — so these carry an explicit `w` mode.
fn output_bind(stdout: &OutputSink, self_id: &str) -> (String, Option<&'static str>) {
    match stdout {
        OutputSink::Inherit => (format!("#task/{self_id}/fd/1"), None),
        OutputSink::Pipe(id) => (format!("#pipe/{id}/data"), Some("w")),
        OutputSink::File { path, .. } => (path.clone(), Some("w")),
    }
}

fn bind_fd(base: &str, fd: u32, target: &(String, Option<&str>)) -> ShellResult<()> {
    let (path, mode) = target;
    let line = match mode {
        Some(mode) => format!("bind {} fd/{fd} {mode}", quote(path)),
        None => format!("bind {} fd/{fd}", quote(path)),
    };
    write_service(&format!("{base}/ctl"), &line)
}

/// Builds a shell-quoted command line from a spawn spec.
fn command_line(spec: &SpawnSpec) -> String {
    let mut line = quote(&spec.program);
    for arg in &spec.args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    line
}

/// Single-quotes a word so the `#task` cmd parser keeps it as one argument.
fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/#".contains(&b))
    {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn read_service(path: &str) -> ShellResult<String> {
    std::fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .map_err(|err| ShellError::Io(format!("{path}: {err}")))
}

fn write_service(path: &str, value: &str) -> ShellResult<()> {
    // Service files already exist; open write-only without create/truncate
    // (std::fs::write would set O_CREAT|O_TRUNC, which the host rejects with
    // EEXIST on a device file).
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|mut file| file.write_all(value.as_bytes()))
        .map_err(|err| ShellError::Io(format!("{path}: {err}")))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut ns = WasiNamespace;
    let code = run_shell(&args, &mut ns);
    std::process::exit(code);
}
