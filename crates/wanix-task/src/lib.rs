//! Task model and `#task` filesystem for Rust Wanix.
//!
//! This crate owns task allocation through `#task/new/*`, task metadata files,
//! fd tables, driver selection, and per-task namespaces.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use wanix_fs::NormalizedPath;
use wanix_vfs::Namespace;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix task model";

/// Stable task identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(NonZeroU64);

impl TaskId {
    /// Creates a task id.
    ///
    /// # Panics
    ///
    /// Panics if `id` is zero. Root tasks should start at `1`.
    #[must_use]
    pub fn new(id: u64) -> Self {
        Self(NonZeroU64::new(id).expect("task id must be non-zero"))
    }

    /// Returns the numeric task id.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// File descriptor identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fd(u32);

impl Fd {
    /// Standard input.
    pub const STDIN: Self = Self(0);
    /// Standard output.
    pub const STDOUT: Self = Self(1);
    /// Standard error.
    pub const STDERR: Self = Self(2);

    /// Returns the numeric fd.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Explicit task launch description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    /// Program path resolved inside the task namespace.
    pub program: NormalizedPath,
    /// Program arguments, excluding `program`.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Current directory inside the namespace.
    pub cwd: NormalizedPath,
}

impl TaskSpec {
    /// Creates a task spec with no args, no env, and root cwd.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `program` is not a normalized path.
    pub fn new(program: impl AsRef<str>) -> wanix_fs::FsResult<Self> {
        Ok(Self {
            program: NormalizedPath::new(program)?,
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: NormalizedPath::new(".")?,
        })
    }
}

/// A task and its namespace.
#[derive(Debug, Clone)]
pub struct Task {
    id: TaskId,
    spec: TaskSpec,
    namespace: Namespace,
}

impl Task {
    /// Creates a task.
    #[must_use]
    pub fn new(id: TaskId, spec: TaskSpec, namespace: Namespace) -> Self {
        Self {
            id,
            spec,
            namespace,
        }
    }

    /// Returns the task id.
    #[must_use]
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Returns the task spec.
    #[must_use]
    pub fn spec(&self) -> &TaskSpec {
        &self.spec
    }

    /// Returns the task namespace.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }
}

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, Fd, Task, TaskId, TaskSpec};
    use wanix_vfs::Namespace;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn task_spec_uses_explicit_program_args_env_and_cwd() {
        let mut spec = TaskSpec::new("bin/app").unwrap();
        spec.args.push("--help".to_owned());
        spec.env.insert("KEY".to_owned(), "value".to_owned());

        assert_eq!(spec.program.as_str(), "bin/app");
        assert_eq!(spec.args, ["--help"]);
        assert_eq!(spec.env["KEY"], "value");
        assert_eq!(spec.cwd.as_str(), ".");
    }

    #[test]
    fn task_carries_id_spec_and_namespace() {
        let task = Task::new(
            TaskId::new(1),
            TaskSpec::new("bin/app").unwrap(),
            Namespace::new(),
        );

        assert_eq!(task.id().get(), 1);
        assert_eq!(task.spec().program.as_str(), "bin/app");
        assert!(task.namespace().bindings().is_empty());
        assert_eq!(Fd::STDOUT.get(), 1);
    }
}
