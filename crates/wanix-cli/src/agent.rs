//! `wanix agent` — drive the `#agent` device from the CLI: allocate a session,
//! submit a prompt, and stream the normalized event log. Uses the real
//! `codex app-server` engine by default, or a deterministic fake with `--fake`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use wanix_agent::{AgentDevice, AgentEngine, CodexEngine, FakeEngine};
use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};

use crate::{CliError, CliOutput};

#[cfg(test)]
mod tests;

/// A parsed `agent` command.
#[derive(Debug)]
pub(super) struct AgentCommand {
    fake: bool,
    cwd: String,
    prompt: String,
}

/// Parses `agent [--fake] [--cwd DIR] <prompt...>`.
///
/// # Errors
///
/// Returns a usage error when the prompt is missing or an option is malformed.
pub(super) fn parse_agent_command(args: &[OsString]) -> Result<AgentCommand, CliError> {
    let mut fake = false;
    let mut cwd = ".".to_owned();
    let mut prompt_parts: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| CliError::usage("agent: arguments must be valid UTF-8"))?;
        match arg {
            "--fake" => fake = true,
            "--cwd" => {
                cwd = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| CliError::usage("agent: --cwd requires a directory"))?
                    .to_owned();
            }
            other if other.starts_with("--") => {
                return Err(CliError::usage(format!("agent: unknown option {other}")));
            }
            other => prompt_parts.push(other.to_owned()),
        }
    }
    if prompt_parts.is_empty() {
        return Err(CliError::usage("agent: missing prompt"));
    }
    Ok(AgentCommand {
        fake,
        cwd,
        prompt: prompt_parts.join(" "),
    })
}

/// Runs one agent turn and returns the streamed event log.
///
/// # Errors
///
/// Returns an error when the engine cannot start, the device files cannot be
/// driven, or the turn fails.
pub(super) fn run_agent_command(command: AgentCommand) -> Result<CliOutput, CliError> {
    let engine: Arc<dyn AgentEngine> = if command.fake {
        Arc::new(FakeEngine)
    } else {
        Arc::new(CodexEngine::new(PathBuf::from("codex"), None, command.cwd))
    };
    let device = AgentDevice::new(engine);

    let id = read_file(&device, "new")?.trim().to_owned();
    write_file(&device, &format!("{id}/prompt"), &command.prompt)?;
    let log = stream_events(&device, &id)?;
    Ok(CliOutput::new(log.into_bytes(), Vec::new(), 0))
}

fn path(raw: &str) -> Result<NormalizedPath, CliError> {
    NormalizedPath::new(raw)
        .map_err(|error| CliError::new(format!("agent: bad path {raw}: {error}"), 1))
}

fn read_file(device: &AgentDevice, raw: &str) -> Result<String, CliError> {
    let mut file = device
        .open(&path(raw)?, OpenOptions::read())
        .map_err(|error| CliError::new(format!("agent: open {raw}: {error}"), 1))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|error| CliError::new(format!("agent: read {raw}: {error}"), 1))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(out).map_err(|_| CliError::new("agent: non-utf8 output".to_owned(), 1))
}

fn write_file(device: &AgentDevice, raw: &str, text: &str) -> Result<(), CliError> {
    let mut file = device
        .open(&path(raw)?, OpenOptions::read_write())
        .map_err(|error| CliError::new(format!("agent: open {raw}: {error}"), 1))?;
    file.write(text.as_bytes())
        .map(|_| ())
        .map_err(|error| CliError::new(format!("agent: write {raw}: {error}"), 1))
}

fn stream_events(device: &AgentDevice, id: &str) -> Result<String, CliError> {
    let raw = format!("{id}/events");
    let mut file = device
        .open(&path(&raw)?, OpenOptions::read())
        .map_err(|error| CliError::new(format!("agent: open {raw}: {error}"), 1))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|error| CliError::new(format!("agent: read {raw}: {error}"), 1))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if String::from_utf8_lossy(&out).contains("\"turn.completed\"") {
            break;
        }
    }
    String::from_utf8(out).map_err(|_| CliError::new("agent: non-utf8 events".to_owned(), 1))
}
