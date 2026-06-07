//! [`CpuJobSpec`]: what a caller asks a remote node to run.
//!
//! The spec is the small, serializable description of a CPU job: the task kind
//! (`qjs`, `wasm`, `noop`, …), the program to run, its argv, environment, and
//! working directory. The acceptor turns it into a local task via the exact
//! local pattern (`allocate_root` → `bind(world)` → configure → `start`), with
//! the *world* being the caller's reverse-exported namespace.

use wanix_fs::{FsResult, NormalizedPath};

/// A description of the task a CPU job runs on the remote node.
///
/// The acceptor uses this to allocate and configure a task whose world is the
/// caller's reverse-exported 9P namespace. The fields mirror the `#task` control
/// surface (`kind`, `cmd`/argv, `env`, `dir`) so the remote run is byte-for-byte
/// the local launch, only with a remote world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuJobSpec {
    /// The task driver kind to run under (e.g. `qjs`, `wasm`, `noop`).
    pub kind: String,
    /// The program path within the world, the task's `program`/argv[0].
    pub program: String,
    /// Additional arguments passed after the program.
    pub args: Vec<String>,
    /// Environment lines in `KEY=value` form.
    pub env: Vec<String>,
    /// Working directory within the world, as a normalized Wanix path.
    pub cwd: NormalizedPath,
}

impl CpuJobSpec {
    /// Builds a spec for `kind` running `program` at the world root (`.`).
    ///
    /// # Errors
    ///
    /// Returns [`wanix_fs::FsError::InvalidPath`] only in the unreachable case
    /// that the literal `.` working directory fails to normalize.
    pub fn new(kind: impl Into<String>, program: impl Into<String>) -> FsResult<Self> {
        Ok(Self {
            kind: kind.into(),
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: NormalizedPath::new(".")?,
        })
    }

    /// Replaces the argument vector (the args after the program).
    #[must_use]
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Replaces the environment lines (`KEY=value`).
    #[must_use]
    pub fn with_env(mut self, env: Vec<String>) -> Self {
        self.env = env;
        self
    }

    /// Sets the working directory within the world.
    ///
    /// # Errors
    ///
    /// Returns [`wanix_fs::FsError::InvalidPath`] when `cwd` is not a valid
    /// normalized Wanix path.
    pub fn with_cwd(mut self, cwd: impl AsRef<str>) -> FsResult<Self> {
        self.cwd = NormalizedPath::new(cwd)?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_root_cwd_and_empty_argv() {
        let spec = CpuJobSpec::new("qjs", "build.js").unwrap();
        assert_eq!(spec.kind, "qjs");
        assert_eq!(spec.program, "build.js");
        assert!(spec.args.is_empty());
        assert!(spec.env.is_empty());
        assert_eq!(spec.cwd.as_str(), ".");
    }

    #[test]
    fn builders_set_argv_env_and_cwd() {
        let spec = CpuJobSpec::new("wasm", "tool.wasm")
            .unwrap()
            .with_args(vec!["--flag".to_owned()])
            .with_env(vec!["K=v".to_owned()])
            .with_cwd("work")
            .unwrap();
        assert_eq!(spec.args, vec!["--flag".to_owned()]);
        assert_eq!(spec.env, vec!["K=v".to_owned()]);
        assert_eq!(spec.cwd.as_str(), "work");
    }

    #[test]
    fn an_invalid_cwd_is_rejected() {
        assert!(
            CpuJobSpec::new("qjs", "x")
                .unwrap()
                .with_cwd("../escape")
                .is_err()
        );
    }
}
