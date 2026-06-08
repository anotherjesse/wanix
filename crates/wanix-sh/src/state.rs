//! Shell state: working directory, environment, and last exit status.
//!
//! This is the single state type threaded through execution (and, in later
//! cycles, expansion and prompt rendering). `cwd` is the shell's *logical*
//! working directory — root is `.` — used by `pwd`, the prompt, and `$PWD`. The
//! guest's real process cwd never moves, so command and file resolution stay
//! cwd-independent; children do not inherit a moved cwd yet (a documented limit).

use std::collections::BTreeMap;

/// Mutable shell state threaded through a run.
#[derive(Debug, Clone)]
pub struct ShellState {
    cwd: String,
    env: BTreeMap<String, String>,
    last_status: i32,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            cwd: ".".to_owned(),
            env: BTreeMap::new(),
            last_status: 0,
        }
    }
}

impl ShellState {
    /// Creates an empty state with cwd at the root (`.`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The logical working directory (`.` is root).
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Sets the logical working directory.
    pub fn set_cwd(&mut self, cwd: String) {
        self.cwd = cwd;
    }

    /// A user-facing cwd rendering: `.` becomes `/`, else `/<cwd>`.
    #[must_use]
    pub fn cwd_display(&self) -> String {
        if self.cwd == "." {
            "/".to_owned()
        } else {
            format!("/{}", self.cwd)
        }
    }

    /// Looks up an environment variable.
    #[must_use]
    pub fn env_get(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str)
    }

    /// Sets an environment variable.
    pub fn env_set(&mut self, key: String, value: String) {
        self.env.insert(key, value);
    }

    /// Removes an environment variable.
    pub fn env_unset(&mut self, key: &str) {
        self.env.remove(key);
    }

    /// Iterates the environment in sorted key order.
    pub fn env_iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.env.iter()
    }

    /// The last pipeline's exit status (drives `$?` and `&&`/`||`).
    #[must_use]
    pub fn last_status(&self) -> i32 {
        self.last_status
    }

    /// Records the last pipeline's exit status.
    pub fn set_last_status(&mut self, status: i32) {
        self.last_status = status;
    }
}

/// Joins a `cd` argument onto a base cwd, normalizing `.`/`..` (Wanix-relative,
/// root is `.`). An absolute-looking argument (leading `/`) restarts from root.
#[must_use]
pub fn join_cwd(base: &str, arg: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    let start = if arg.starts_with('/') { "" } else { base };
    for part in start.split('/').chain(arg.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    if components.is_empty() {
        ".".to_owned()
    } else {
        components.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cwd_is_root() {
        assert_eq!(ShellState::new().cwd(), ".");
        assert_eq!(ShellState::new().cwd_display(), "/");
    }

    #[test]
    fn env_get_set_unset_roundtrip() {
        let mut state = ShellState::new();
        state.env_set("A".into(), "1".into());
        assert_eq!(state.env_get("A"), Some("1"));
        state.env_unset("A");
        assert_eq!(state.env_get("A"), None);
    }

    #[test]
    fn cwd_display_prefixes_slash() {
        let mut state = ShellState::new();
        state.set_cwd("work/sub".into());
        assert_eq!(state.cwd_display(), "/work/sub");
    }

    #[test]
    fn join_cwd_handles_relative_absolute_and_dotdot() {
        assert_eq!(join_cwd(".", "work"), "work");
        assert_eq!(join_cwd("work", "sub"), "work/sub");
        assert_eq!(join_cwd("work/sub", ".."), "work");
        assert_eq!(join_cwd("work", "/etc"), "etc");
        assert_eq!(join_cwd("work", "."), "work");
        assert_eq!(join_cwd("a/b", "../c"), "a/c");
        assert_eq!(join_cwd(".", ".."), ".");
    }
}
