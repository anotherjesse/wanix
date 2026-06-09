//! `wanix tool`: built-in ToolFS devices served over the native mesh wire.
//!
//! A tool is one host-approved operation family exposed as files
//! (`docs/toolfs.md`, ADR 0009): the host fixes the operation and its policy,
//! a caller supplies only input bytes through the mounted job protocol. This
//! module owns the built-in v0 tool registry (in-process runners only — the
//! process runner is a later crate) and the per-tool endpoint identities;
//! `tool serve` itself lives in [`serve`].

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use wanix_id::NodeIdentity;
use wanix_tool::runners::{FakeModelEngine, ModelRunner, UpperRunner};
use wanix_tool::{RunOutcome, ToolClock, ToolRunner, ToolService, ToolSpec};

use crate::CliError;
use crate::volume::wanix_dir;

mod serve;

pub(crate) use serve::{parse_tool_serve_command, run_tool_serve_streaming};

/// The built-in v0 tools, sorted; the only names `tool serve --tool` accepts.
pub(crate) const BUILTIN_TOOL_NAMES: [&str; 3] = ["model", "sha256", "upper"];

/// Builds the [`ToolService`] for one built-in tool name, on the wall clock.
///
/// # Errors
///
/// Returns a usage error for a name outside [`BUILTIN_TOOL_NAMES`].
pub(crate) fn build_tool_service(name: &str) -> Result<ToolService, CliError> {
    match name {
        "upper" => Ok(ToolService::new(
            ToolSpec::v0("upper", "Uppercase UTF-8 text (deterministic, in-process)."),
            Box::new(UpperRunner),
            wall_clock(),
        )),
        "sha256" => Ok(ToolService::new(
            sha256_spec(),
            Box::new(Sha256Runner),
            wall_clock(),
        )),
        "model" => Ok(ToolService::new(
            ToolSpec::v0(
                "model",
                "Deterministic FAKE model for demos: completes every prompt as \
                 'fake-completion: <prompt>'. No real LLM is behind this tool.",
            ),
            Box::new(ModelRunner::new(std::sync::Arc::new(FakeModelEngine))),
            wall_clock(),
        )),
        other => Err(CliError::usage(format!(
            "tool serve: unknown built-in tool {other:?} (available: {})",
            BUILTIN_TOOL_NAMES.join(", ")
        ))),
    }
}

/// The sha256 spec: arbitrary bytes in, one hex digest line out. Hashing is
/// cheap, so the input cap is raised to 8 MiB (with the per-principal byte
/// quota raised to match: digests add almost nothing to stored bytes).
fn sha256_spec() -> ToolSpec {
    let mut spec = ToolSpec::v0(
        "sha256",
        "SHA-256 of the input bytes, as a lowercase hex digest line.",
    );
    spec.input.content_types = vec!["application/octet-stream".to_owned()];
    spec.input.max_bytes = 8 * 1024 * 1024;
    spec.limits.max_bytes_per_principal = 32 * 1024 * 1024;
    spec.outputs.primary.content_type = Some("text/plain; charset=utf-8".to_owned());
    spec
}

/// Hashes the input bytes with SHA-256; output is the lowercase hex digest
/// plus a newline. Lives in `wanix-cli` (over the `sha2` crate) so the
/// `wanix-tool` dependency set stays `wanix-fs` + `wanix-job` + serde only.
#[derive(Debug, Default, Clone, Copy)]
struct Sha256Runner;

impl ToolRunner for Sha256Runner {
    fn run(&self, input: &[u8], _params: Option<&Value>) -> RunOutcome {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(input);
        let mut out = String::with_capacity(digest.len() * 2 + 1);
        for byte in digest {
            out.push_str(&format!("{byte:02x}"));
        }
        out.push('\n');
        RunOutcome::success(out.into_bytes())
    }
}

/// The production tool clock: Unix-epoch milliseconds from the wall clock.
fn wall_clock() -> ToolClock {
    Box::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
    })
}

/// The per-tool mesh-endpoint identity key path,
/// `~/.wanix/tool-identities/<name>.key` (mirrors the volume-identities
/// precedent). Deliberately OUTSIDE anything served, so a tool's own endpoint
/// secret key is never exported to peers that mount it.
fn tool_identity_path(name: &str) -> Result<PathBuf, CliError> {
    Ok(wanix_dir()?
        .join("tool-identities")
        .join(format!("{name}.key")))
}

/// Loads (or creates) the stable per-tool endpoint identity for `name`. Each
/// tool gets a distinct key, hence a distinct peer id, so its mesh endpoint is
/// an independent resource (ADR 0007: one ticket names one resource).
fn load_tool_identity(name: &str) -> Result<NodeIdentity, CliError> {
    crate::mesh::resource::load_identity_at(&tool_identity_path(name)?)
}

#[cfg(test)]
mod tests {
    use super::{BUILTIN_TOOL_NAMES, Sha256Runner, build_tool_service};
    use wanix_tool::ToolRunner;

    #[test]
    fn sha256_runner_emits_a_lowercase_hex_digest_line() {
        let outcome = Sha256Runner.run(b"hello", None);
        assert_eq!(
            outcome.out,
            b"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\n".to_vec()
        );
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn every_builtin_tool_builds_and_names_match_specs() {
        for name in BUILTIN_TOOL_NAMES {
            let service = build_tool_service(name).unwrap();
            assert_eq!(service.spec().envelope.name, name);
        }
    }

    #[test]
    fn model_tool_is_clearly_labeled_a_fake() {
        let service = build_tool_service("model").unwrap();
        let description = service.spec().envelope.description.to_lowercase();
        assert!(description.contains("fake"), "{description}");
        assert!(description.contains("no real llm"), "{description}");
    }

    #[test]
    fn unknown_tool_is_a_usage_error_naming_the_builtins() {
        let error = build_tool_service("rm-rf").unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(
            error.to_string().contains("model, sha256, upper"),
            "{error}"
        );
    }
}
