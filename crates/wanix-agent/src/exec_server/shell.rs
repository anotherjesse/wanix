//! A small real shell for `process/start`, run against the Wanix filesystem.
//! Agents send commands as `["/bin/zsh","-lc","<source>"]` and frequently nest
//! `sh -c '...'`; we unwrap those, split on `&&`, and run a Unix-ish command set
//! (`echo`/`printf`/`cat`/`ls`/`mkdir`/`rm`/`touch`/`head`/`wc`, with `>`/`>>`
//! redirects) so the agent's shell work lands in the bound Wanix world. Anything
//! unsupported returns a clear error so the agent falls back to the `fs/*`
//! methods.

use super::{CompletedProcess, ExecServer};

/// Unwraps an argv into a shell source string (peels nested `sh -c '...'`).
pub(super) fn unwrap_source(argv: &[String]) -> String {
    shell_source(argv)
}

/// Runs an already-unwrapped shell source against the Wanix filesystem.
pub(super) fn run_source(server: &ExecServer, source: &str, cwd: &str) -> CompletedProcess {
    interpret_sequence(server, source, cwd)
}

fn shell_source(argv: &[String]) -> String {
    let mut source = if argv.len() >= 2 && is_command_flag(&argv[argv.len() - 2]) {
        argv[argv.len() - 1].clone()
    } else {
        argv.join(" ")
    };
    while let Some(inner) = unwrap_shell(&source) {
        source = inner;
    }
    source
}

fn is_command_flag(flag: &str) -> bool {
    matches!(flag, "-c" | "-lc" | "-ic" | "-i")
}

/// Unwraps one layer of `[/path/]sh -c '<inner>'`.
fn unwrap_shell(source: &str) -> Option<String> {
    let trimmed = source.trim();
    let mut parts = trimmed.splitn(3, char::is_whitespace);
    let program = parts.next()?;
    let flag = parts.next()?;
    let rest = parts.next()?.trim();
    let program_is_shell = program
        .rsplit('/')
        .next()
        .is_some_and(|name| name.ends_with("sh"));
    if !program_is_shell || !is_command_flag(flag) {
        return None;
    }
    let inner = if (rest.starts_with('\'') && rest.ends_with('\'') && rest.len() >= 2)
        || (rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2)
    {
        &rest[1..rest.len() - 1]
    } else {
        rest
    };
    Some(inner.to_owned())
}

fn ok(stdout: Vec<u8>) -> CompletedProcess {
    CompletedProcess {
        stdout,
        stderr: Vec::new(),
        exit_code: 0,
    }
}

fn fail(message: &str) -> CompletedProcess {
    CompletedProcess {
        stdout: Vec::new(),
        stderr: message.as_bytes().to_vec(),
        exit_code: 1,
    }
}

fn resolve(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), path)
    }
}

fn unquote(token: &str) -> &str {
    token.trim_matches('"').trim_matches('\'')
}

/// Parses a chmod mode: a symbolic `+x` (→ 0o755) or an octal literal.
fn parse_mode(arg: &str) -> u32 {
    if arg.contains('+') {
        0o755
    } else {
        u32::from_str_radix(arg, 8).unwrap_or(0o644)
    }
}

fn interpret_sequence(server: &ExecServer, source: &str, cwd: &str) -> CompletedProcess {
    let mut stdout = Vec::new();
    for stage in source.split("&&") {
        let result = interpret_one(server, stage.trim(), cwd);
        stdout.extend_from_slice(&result.stdout);
        if result.exit_code != 0 {
            return CompletedProcess {
                stdout,
                stderr: result.stderr,
                exit_code: result.exit_code,
            };
        }
    }
    ok(stdout)
}

enum Redirect {
    Truncate,
    Append,
}

fn interpret_one(server: &ExecServer, source: &str, cwd: &str) -> CompletedProcess {
    let (command, redirect) = if let Some((command, file)) = source.split_once(">>") {
        (
            command.trim(),
            Some((Redirect::Append, unquote(file.trim()))),
        )
    } else if let Some((command, file)) = source.split_once('>') {
        (
            command.trim(),
            Some((Redirect::Truncate, unquote(file.trim()))),
        )
    } else {
        (source.trim(), None)
    };
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let Some((&verb, args)) = tokens.split_first() else {
        return ok(Vec::new());
    };
    let produced = match verb {
        "echo" | "printf" => {
            let mut text = args
                .iter()
                .map(|t| unquote(t))
                .collect::<Vec<_>>()
                .join(" ");
            if verb == "echo" {
                text.push('\n');
            }
            Ok(text.into_bytes())
        }
        "cat" => match args.first() {
            Some(file) => server
                .read_bytes(&resolve(cwd, file))
                .map_err(|e| format!("cat: {e:?}")),
            None => Err("cat: missing file".to_owned()),
        },
        "head" => match args.last() {
            Some(file) => server
                .read_bytes(&resolve(cwd, file))
                .map(|bytes| head_lines(&bytes, 10))
                .map_err(|e| format!("head: {e:?}")),
            None => Err("head: missing file".to_owned()),
        },
        "wc" => match args.last() {
            Some(file) => server
                .read_bytes(&resolve(cwd, file))
                .map(|bytes| word_count(&bytes))
                .map_err(|e| format!("wc: {e:?}")),
            None => Err("wc: missing file".to_owned()),
        },
        "ls" => {
            let dir = args
                .iter()
                .find(|a| !a.starts_with('-'))
                .copied()
                .unwrap_or(".");
            server
                .list(&resolve(cwd, dir))
                .map(|mut names| {
                    names.sort();
                    format!("{}\n", names.join("\n")).into_bytes()
                })
                .map_err(|e| format!("ls: {e:?}"))
        }
        "mkdir" => match args.iter().find(|a| !a.starts_with('-')) {
            Some(dir) => server
                .make_dir(&resolve(cwd, dir))
                .map(|()| Vec::new())
                .map_err(|e| format!("mkdir: {e:?}")),
            None => Err("mkdir: missing operand".to_owned()),
        },
        "rm" => match args.iter().find(|a| !a.starts_with('-')) {
            Some(file) => server
                .remove(&resolve(cwd, file))
                .map(|()| Vec::new())
                .map_err(|e| format!("rm: {e:?}")),
            None => Err("rm: missing operand".to_owned()),
        },
        "touch" => match args.first() {
            Some(file) => server
                .write_bytes(&resolve(cwd, file), b"")
                .map(|()| Vec::new())
                .map_err(|e| format!("touch: {e:?}")),
            None => Err("touch: missing file".to_owned()),
        },
        "chmod" => {
            let operands: Vec<&str> = args
                .iter()
                .filter(|a| !a.starts_with('-'))
                .copied()
                .collect();
            match (operands.first(), operands.get(1)) {
                (Some(mode), Some(file)) => server
                    .chmod(&resolve(cwd, file), parse_mode(mode))
                    .map(|()| Vec::new())
                    .map_err(|e| format!("chmod: {e:?}")),
                _ => Err("chmod: usage: chmod MODE FILE".to_owned()),
            }
        }
        "true" | ":" => Ok(Vec::new()),
        other => Err(format!("wanix exec-server: unsupported command `{other}`")),
    };
    match produced {
        Ok(bytes) => apply_redirect(server, cwd, redirect, bytes),
        Err(message) => fail(&message),
    }
}

fn apply_redirect(
    server: &ExecServer,
    cwd: &str,
    redirect: Option<(Redirect, &str)>,
    bytes: Vec<u8>,
) -> CompletedProcess {
    let Some((mode, file)) = redirect else {
        return ok(bytes);
    };
    let path = resolve(cwd, file);
    let payload = match mode {
        Redirect::Truncate => bytes,
        Redirect::Append => {
            let mut existing = server.read_bytes(&path).unwrap_or_default();
            existing.extend_from_slice(&bytes);
            existing
        }
    };
    match server.write_bytes(&path, &payload) {
        Ok(()) => ok(Vec::new()),
        Err(error) => fail(&format!("write: {error:?}")),
    }
}

fn head_lines(bytes: &[u8], count: usize) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .take(count)
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn word_count(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let lines = text.lines().count();
    let words = text.split_whitespace().count();
    format!("{lines} {words} {}\n", bytes.len()).into_bytes()
}
