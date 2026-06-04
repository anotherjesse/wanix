use std::collections::BTreeMap;
use std::num::NonZeroU64;

use wanix_fs::{FsResult, NormalizedPath};

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
    pub fn new(program: impl AsRef<str>) -> FsResult<Self> {
        Ok(Self {
            program: NormalizedPath::new(program)?,
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: NormalizedPath::new(".")?,
        })
    }

    pub(crate) fn unset() -> Self {
        Self::new(".").expect("root path is valid")
    }
}
