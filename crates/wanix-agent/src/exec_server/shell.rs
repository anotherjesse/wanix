//! A deliberately small shell interpreter for `process/start`. Agents send
//! commands as `["/bin/zsh","-lc","<source>"]`; we run a minimal command set
//! (`echo`/`printf`/`cat`/`ls`, with one `>` redirect) against the Wanix
//! filesystem. Anything else returns a clear "unsupported" error so the agent
//! can fall back to the `fs/*` methods.

use super::{CompletedProcess, ExecServer};

pub(super) fn run(server: &ExecServer, argv: &[String], cwd: &str) -> CompletedProcess {
    interpret(server, &shell_source(argv), cwd)
}

fn shell_source(argv: &[String]) -> String {
    if argv.len() >= 2 {
        let flag = &argv[argv.len() - 2];
        if flag == "-lc" || flag == "-c" {
            return argv[argv.len() - 1].clone();
        }
    }
    argv.join(" ")
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

fn interpret(server: &ExecServer, source: &str, cwd: &str) -> CompletedProcess {
    let (command, redirect) = match source.trim().split_once('>') {
        Some((command, file)) => (command.trim(), Some(unquote(file.trim()))),
        None => (source.trim(), None),
    };
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let Some((&verb, args)) = tokens.split_first() else {
        return fail("wanix exec-server: empty command");
    };
    match verb {
        "echo" | "printf" => {
            let mut text = args
                .iter()
                .map(|t| unquote(t))
                .collect::<Vec<_>>()
                .join(" ");
            if verb == "echo" {
                text.push('\n');
            }
            match redirect {
                Some(file) => match server.write_bytes(&resolve(cwd, file), text.as_bytes()) {
                    Ok(()) => ok(Vec::new()),
                    Err(error) => fail(&format!("write: {error:?}")),
                },
                None => ok(text.into_bytes()),
            }
        }
        "cat" => match args.first() {
            Some(file) => match server.read_bytes(&resolve(cwd, file)) {
                Ok(bytes) => ok(bytes),
                Err(error) => fail(&format!("cat: {error:?}")),
            },
            None => fail("cat: missing file"),
        },
        "ls" => {
            let dir = args.first().copied().unwrap_or(".");
            match server.list(&resolve(cwd, dir)) {
                Ok(mut names) => {
                    names.sort();
                    ok(format!("{}\n", names.join("\n")).into_bytes())
                }
                Err(error) => fail(&format!("ls: {error:?}")),
            }
        }
        other => fail(&format!("wanix exec-server: unsupported command `{other}`")),
    }
}
