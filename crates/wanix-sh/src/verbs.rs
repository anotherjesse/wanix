//! Resource-qualified verb invocation: `NAME:CMD [args]`.
//!
//! Mounting a resource means gaining its vocabulary: a resource ships
//! executable verbs in its `bin/` directory, and the shell invokes them as
//! `NAME:CMD` where `NAME` is a currently mounted resource and `CMD` names
//! exactly one of `bin/CMD.js` or `bin/CMD.wasm` under it. There is
//! deliberately NO `$PATH` merging of mounted `bin/` directories: a verb only
//! ever runs qualified by the resource it came from, so a hostile mount can
//! never squat an unqualified command name (squatting safety). The child runs
//! confined by default — its namespace is exactly the resource bound at `res`,
//! plus the stdio fds and argv/env the shell passes explicitly (the
//! `wanix-task` `confine` ctl verb; ADR 0007 §Confinement contract).

use crate::error::{ShellError, ShellResult};
use crate::ns::NamespaceOps;
use crate::resolve::resolve_command;

/// Where mounted resources are looked up, in order: `n/NAME` (mesh mounts by
/// name) then `vol/NAME` (the explicit `--mount-mesh NAME=/vol/...` and
/// recipe-bind convention).
const MOUNT_ROOTS: &[&str] = &["n", "vol"];

/// Inside a confined child the resource is always at this path (the
/// `wanix-task` confinement contract), so verb program paths start here.
const CONFINED_RESOURCE_PATH: &str = "res";

/// A resolved launch target: the program path as the *child* will resolve it,
/// plus the confinement root (the resource's mount path in the shell's
/// namespace) when the target is a resource verb.
pub(crate) struct SpawnTarget {
    pub(crate) program: String,
    pub(crate) confine: Option<String>,
}

/// Resolves a command word to a launch target: a `NAME:CMD` verb resolves to
/// a confined `res/bin/CMD.{js,wasm}` launch; anything else resolves as a
/// plain command via [`resolve_command`].
///
/// # Errors
///
/// Returns an error when a verb-spelled word names an unmounted resource, a
/// missing verb, or an ambiguous one (callers report it as command-not-found,
/// status 127), or when existence cannot be determined.
pub(crate) fn resolve_spawn_target(word: &str, ns: &dyn NamespaceOps) -> ShellResult<SpawnTarget> {
    match parse_verb(word) {
        Some((name, cmd)) => resolve_verb(name, cmd, ns),
        None => Ok(SpawnTarget {
            program: resolve_command(word, ns)?,
            confine: None,
        }),
    }
}

/// Splits `NAME:CMD` when both halves are name-spelled. Anything with a
/// scheme, slash, dot, or uppercase is never a verb and falls through to
/// plain command resolution — resolution routes on spelling alone.
fn parse_verb(word: &str) -> Option<(&str, &str)> {
    let (name, cmd) = word.split_once(':')?;
    (is_name_spelling(name) && is_name_spelling(cmd)).then_some((name, cmd))
}

/// The catalog NAME spelling: lowercase `[a-z0-9-]`, alphanumeric first/last.
fn is_name_spelling(part: &str) -> bool {
    let bytes = part.as_bytes();
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
}

fn resolve_verb(name: &str, cmd: &str, ns: &dyn NamespaceOps) -> ShellResult<SpawnTarget> {
    let Some(root) = mount_root(name, ns)? else {
        return Err(ShellError::Io(format!(
            "{name} is not a mounted resource (no /n/{name} or /vol/{name})"
        )));
    };
    let js = format!("{root}/bin/{cmd}.js");
    let wasm = format!("{root}/bin/{cmd}.wasm");
    let extension = match (ns.exists(&js)?, ns.exists(&wasm)?) {
        (true, true) => {
            return Err(ShellError::Io(format!(
                "ambiguous verb: both bin/{cmd}.js and bin/{cmd}.wasm exist under /{root}"
            )));
        }
        (false, false) => {
            return Err(ShellError::Io(format!(
                "no verb bin/{cmd}.js or bin/{cmd}.wasm under /{root}"
            )));
        }
        (true, false) => "js",
        (false, true) => "wasm",
    };
    Ok(SpawnTarget {
        program: format!("{CONFINED_RESOURCE_PATH}/bin/{cmd}.{extension}"),
        confine: Some(root),
    })
}

/// Finds the mount path a resource name refers to, trying [`MOUNT_ROOTS`] in
/// order.
fn mount_root(name: &str, ns: &dyn NamespaceOps) -> ShellResult<Option<String>> {
    for dir in MOUNT_ROOTS {
        let root = format!("{dir}/{name}");
        if ns.exists(&root)? {
            return Ok(Some(root));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::parse_verb;

    #[test]
    fn verb_spelling_requires_two_names() {
        assert_eq!(parse_verb("room:post"), Some(("room", "post")));
        assert_eq!(parse_verb("my-room:do-it2"), Some(("my-room", "do-it2")));
        // Schemes, paths, dots, uppercase, and bare colons are never verbs.
        for word in [
            "iroh://peer",
            "room:bin/post",
            "room:post.js",
            "Room:post",
            "room:",
            ":post",
            "room",
            "a:b:c",
            "-room:post",
            "room-:post",
        ] {
            assert_eq!(parse_verb(word), None, "{word} must not parse as a verb");
        }
    }
}
