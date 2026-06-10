//! `tools.toml`: host-program tools for `tool serve --config`.
//!
//! The config is the host side of the ToolFS inversion (`docs/toolfs.md`
//! §"Runner Boundary"): the operator fixes the executable, its argv, and the
//! input/output mapping here; a caller only ever supplies input bytes through
//! the mounted job protocol. There is deliberately no shell anywhere in this
//! shape — a command is one absolute executable path and a fixed argv list,
//! with `{input}`/`{output}` as the only placeholders, each tied to its
//! `tempfile` mapping.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use wanix_tool::{ToolService, ToolSpec, ToolVisibility};

use super::process::ProcRunner;
use super::wall_clock;
use crate::CliError;

/// A parsed and validated `tools.toml`: one `[tools.NAME]` table per tool.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolConfigFile {
    #[serde(default)]
    tools: BTreeMap<String, ProcToolConfig>,
}

impl ToolConfigFile {
    /// The configured tool names, sorted.
    pub(crate) fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// The config for one tool name, when the file defines it.
    pub(crate) fn get(&self, name: &str) -> Option<&ProcToolConfig> {
        self.tools.get(name)
    }

    fn validate(&self) -> Result<(), CliError> {
        if self.tools.is_empty() {
            return Err(CliError::usage(
                "tools config: no [tools.NAME] tables defined",
            ));
        }
        for (name, tool) in &self.tools {
            validate_name(name)?;
            tool.validate(name)?;
        }
        Ok(())
    }
}

/// One `[tools.NAME]` table: the host-fixed process policy plus the shipped
/// spec sections (`limits`/`lifecycle` deserialize straight into the
/// `wanix-jobfs` shapes, so `spec.json` reflects exactly what was configured).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcToolConfig {
    description: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    input: ProcInputMode,
    #[serde(default)]
    output: ProcOutputMode,
    #[serde(default = "default_visibility")]
    visibility: ToolVisibility,
    #[serde(default)]
    limits: ConfigLimits,
    #[serde(default)]
    lifecycle: ConfigLifecycle,
}

fn default_visibility() -> ToolVisibility {
    ToolVisibility::Private
}

/// How the job's sealed input bytes reach the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcInputMode {
    /// Input bytes are written to the child's stdin.
    #[default]
    Stdin,
    /// Input bytes are written to a private workdir file whose absolute path
    /// replaces the single `{input}` argv placeholder.
    Tempfile,
}

/// Where the job's primary output comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcOutputMode {
    /// The child's captured stdout is the output.
    #[default]
    Stdout,
    /// The child writes a private workdir file whose absolute path replaces
    /// the single `{output}` argv placeholder.
    Tempfile,
}

/// Optional `[tools.NAME.limits]` overrides, snake_case per the docs sketch;
/// unset fields keep the shipped `JobLimits`/`ToolInput` defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigLimits {
    max_input_bytes: Option<u64>,
    run_timeout_ms: Option<u64>,
    max_concurrent_per_principal: Option<u64>,
    max_jobs_per_principal: Option<u64>,
    max_bytes_per_principal: Option<u64>,
    max_total_jobs: Option<u64>,
    max_total_bytes: Option<u64>,
    max_out_bytes: Option<u64>,
    max_err_bytes: Option<u64>,
}

/// Optional `[tools.NAME.lifecycle]` overrides (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigLifecycle {
    allocated_ttl_ms: Option<u64>,
    retain_done_ms: Option<u64>,
    retain_failed_ms: Option<u64>,
}

impl ProcToolConfig {
    /// Builds the served [`ToolService`]: the spec advertising exactly the
    /// configured policy, fronting a [`ProcRunner`] for the fixed command.
    pub(crate) fn build_service(&self, name: &str) -> Result<ToolService, CliError> {
        let spec = self.tool_spec(name);
        let runner = ProcRunner::new(
            self.command.clone().into(),
            self.args.clone(),
            self.input,
            self.output,
            spec.limits.max_out_bytes,
            spec.limits.max_err_bytes,
        );
        ToolService::new(spec, Box::new(runner), wall_clock())
            .map_err(|err| CliError::new(format!("tool serve: {name}: {err}"), 1))
    }

    /// The honest `spec.json` for this config: defaults plus exactly the
    /// configured limit/lifecycle overrides.
    fn tool_spec(&self, name: &str) -> ToolSpec {
        let mut spec = ToolSpec::v0(name, &self.description);
        spec.input.content_types = vec!["application/octet-stream".to_owned()];
        let limits = &self.limits;
        set(&mut spec.input.max_bytes, limits.max_input_bytes);
        set(&mut spec.limits.run_timeout_ms, limits.run_timeout_ms);
        set(
            &mut spec.limits.max_concurrent_per_principal,
            limits.max_concurrent_per_principal,
        );
        set(
            &mut spec.limits.max_jobs_per_principal,
            limits.max_jobs_per_principal,
        );
        set(
            &mut spec.limits.max_bytes_per_principal,
            limits.max_bytes_per_principal,
        );
        set(&mut spec.limits.max_total_jobs, limits.max_total_jobs);
        set(&mut spec.limits.max_total_bytes, limits.max_total_bytes);
        set(&mut spec.limits.max_out_bytes, limits.max_out_bytes);
        set(&mut spec.limits.max_err_bytes, limits.max_err_bytes);
        let lifecycle = &self.lifecycle;
        set(
            &mut spec.lifecycle.allocated_ttl_ms,
            lifecycle.allocated_ttl_ms,
        );
        set(&mut spec.lifecycle.retain_done_ms, lifecycle.retain_done_ms);
        set(
            &mut spec.lifecycle.retain_failed_ms,
            lifecycle.retain_failed_ms,
        );
        spec
    }

    fn validate(&self, name: &str) -> Result<(), CliError> {
        let reject =
            |reason: String| Err(CliError::usage(format!("tools config: {name}: {reason}")));
        if !self.command.starts_with('/') {
            return reject(format!(
                "command {:?} must be an absolute executable path",
                self.command
            ));
        }
        if self.command.chars().any(char::is_whitespace)
            || self.command.contains(['|', '&', ';', '$', '`', '<', '>'])
        {
            return reject(format!(
                "command {:?} looks like a shell string; shells are not supported — \
                 use one absolute executable path plus an args list",
                self.command
            ));
        }
        if self.visibility != ToolVisibility::Private {
            return reject(format!(
                "visibility {:?} is not implemented: v0 serves private job views only",
                self.visibility.as_str()
            ));
        }
        self.validate_placeholder("{input}", self.input == ProcInputMode::Tempfile, "input")?;
        self.validate_placeholder(
            "{output}",
            self.output == ProcOutputMode::Tempfile,
            "output",
        )?;
        for arg in &self.args {
            if arg.starts_with('{') && arg.ends_with('}') && arg != "{input}" && arg != "{output}" {
                return reject(format!(
                    "unknown placeholder {arg:?} (only {{input}} and {{output}} exist)"
                ));
            }
        }
        Ok(())
    }

    /// A placeholder must appear exactly once, as a whole argument, exactly
    /// when its `tempfile` mapping is configured.
    fn validate_placeholder(
        &self,
        placeholder: &str,
        required: bool,
        mode_key: &str,
    ) -> Result<(), CliError> {
        let whole = self.args.iter().filter(|arg| *arg == placeholder).count();
        let embedded = self
            .args
            .iter()
            .any(|arg| arg != placeholder && arg.contains(placeholder));
        if embedded {
            return Err(CliError::usage(format!(
                "tools config: {placeholder} must be a whole argument, not embedded in one"
            )));
        }
        match (required, whole) {
            (true, 1) | (false, 0) => Ok(()),
            (true, n) => Err(CliError::usage(format!(
                "tools config: {mode_key} = \"tempfile\" needs exactly one {placeholder} \
                 argument (found {n})"
            ))),
            (false, _) => Err(CliError::usage(format!(
                "tools config: {placeholder} is only allowed when {mode_key} = \"tempfile\""
            ))),
        }
    }
}

/// A tool name becomes a path segment (identity file, mount records), so keep
/// it to a conservative charset.
fn validate_name(name: &str) -> Result<(), CliError> {
    let ok = !name.is_empty()
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "tools config: tool name {name:?} must be [A-Za-z0-9._-]+ and not start with '.' or '-'"
        )))
    }
}

fn set<T: Copy>(slot: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *slot = value;
    }
}

/// Reads and validates a `tools.toml`.
///
/// # Errors
///
/// Returns a usage error when the file cannot be read, is not valid TOML for
/// this shape, defines no tools, or any tool violates the process policy
/// (relative or shell-looking command, bad placeholder, non-private
/// visibility, bad name).
pub(crate) fn load_tool_config(path: &Path) -> Result<ToolConfigFile, CliError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        CliError::usage(format!("tool serve --config {}: {error}", path.display()))
    })?;
    let file: ToolConfigFile = toml::from_str(&text).map_err(|error| {
        CliError::usage(format!("tool serve --config {}: {error}", path.display()))
    })?;
    file.validate()?;
    Ok(file)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
