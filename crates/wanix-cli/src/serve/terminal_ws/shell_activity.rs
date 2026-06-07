use wanix_fs::NormalizedPath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::serve) struct ShellMutationOperation {
    pub(in crate::serve) kind: String,
    pub(in crate::serve) status: String,
    pub(in crate::serve) diagnostic: Option<String>,
    pub(in crate::serve) source: Option<String>,
    pub(in crate::serve) target: Option<String>,
    pub(in crate::serve) paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub(in crate::serve) struct ShellInputActivityTracker {
    cwd: String,
    line: String,
}

impl ShellInputActivityTracker {
    pub(in crate::serve) fn new(cwd: &NormalizedPath) -> Self {
        Self {
            cwd: cwd.as_str().to_owned(),
            line: String::new(),
        }
    }

    pub(in crate::serve) fn sync_cwd(&mut self, cwd: &NormalizedPath) {
        self.cwd = cwd.as_str().to_owned();
    }

    pub(in crate::serve) fn observe_input(&mut self, input: &[u8]) -> Vec<ShellMutationOperation> {
        let mut operations = Vec::new();
        for ch in String::from_utf8_lossy(input).chars() {
            match ch {
                '\r' | '\n' => {
                    if let Some(operation) = self.finish_line() {
                        operations.push(operation);
                    }
                    self.line.clear();
                }
                '\u{7f}' | '\u{8}' => {
                    self.line.pop();
                }
                '\u{3}' => {
                    self.line.clear();
                }
                ch if !ch.is_control() => self.line.push(ch),
                _ => {}
            }
        }
        operations
    }

    fn finish_line(&mut self) -> Option<ShellMutationOperation> {
        let words = shell_words(self.line.trim());
        let command = words.first()?;
        if command == "cd" {
            self.cwd = resolve_wanix_path(&self.cwd, words.get(1).map_or(".", String::as_str));
            return None;
        }
        let redirected = shell_redirection_paths(&words, &self.cwd);
        match command.as_str() {
            "write" => {
                let target = if words.len() >= 3 {
                    words.get(1).map(|path| resolve_wanix_path(&self.cwd, path))
                } else {
                    redirected.last().cloned()
                };
                target.map(|target| operation("write", None, Some(target)))
            }
            "mkdir" | "rm" | "rmdir" => words
                .get(1)
                .map(|path| resolve_wanix_path(&self.cwd, path))
                .or_else(|| redirected.last().cloned())
                .map(|target| operation(command, None, Some(target))),
            "cp" => {
                let source = words.get(1).map(|path| resolve_wanix_path(&self.cwd, path));
                let target = words
                    .get(2)
                    .map(|path| resolve_wanix_path(&self.cwd, path))
                    .or_else(|| redirected.last().cloned());
                match (source, target) {
                    (Some(source), Some(target)) => {
                        Some(operation("cp", Some(source), Some(target)))
                    }
                    (None, Some(target)) => Some(operation("cp", None, Some(target))),
                    _ => None,
                }
            }
            "mv" => {
                let source = words.get(1).map(|path| resolve_wanix_path(&self.cwd, path));
                let target = words.get(2).map(|path| resolve_wanix_path(&self.cwd, path));
                match (source, target) {
                    (Some(source), Some(target)) => {
                        Some(operation("mv", Some(source), Some(target)))
                    }
                    _ => None,
                }
            }
            "ln" if words.get(1).is_some_and(|flag| flag == "-s") => {
                let source = words.get(2).map(|path| resolve_wanix_path(&self.cwd, path));
                let target = words
                    .get(3)
                    .map(|path| resolve_wanix_path(&self.cwd, path))
                    .or_else(|| redirected.last().cloned());
                match (source, target) {
                    (Some(source), Some(target)) => {
                        Some(operation("ln-s", Some(source), Some(target)))
                    }
                    (None, Some(target)) => Some(operation("ln-s", None, Some(target))),
                    _ => None,
                }
            }
            _ if !redirected.is_empty() => Some(ShellMutationOperation {
                kind: "redirect".to_owned(),
                status: "changed".to_owned(),
                diagnostic: None,
                source: None,
                target: redirected.last().cloned(),
                paths: unique_paths(redirected),
            }),
            _ => None,
        }
    }
}

pub(in crate::serve) fn operations_for_changed_paths(
    operations: &[ShellMutationOperation],
    changed_paths: &[String],
) -> Vec<ShellMutationOperation> {
    operations
        .iter()
        .filter(|operation| operation_intersects_changed_paths(operation, changed_paths))
        .cloned()
        .collect()
}

pub(in crate::serve) fn operations_without_changed_paths(
    operations: &[ShellMutationOperation],
    diagnostic: Option<&str>,
) -> Vec<ShellMutationOperation> {
    let diagnostic = diagnostic.map(str::to_owned);
    operations
        .iter()
        .map(|operation| ShellMutationOperation {
            status: "unchanged".to_owned(),
            diagnostic: diagnostic.clone(),
            paths: Vec::new(),
            ..operation.clone()
        })
        .collect()
}

fn operation_intersects_changed_paths(
    operation: &ShellMutationOperation,
    changed_paths: &[String],
) -> bool {
    operation.paths.iter().any(|operation_path| {
        changed_paths
            .iter()
            .any(|changed_path| changed_path == operation_path)
    })
}

fn operation(kind: &str, source: Option<String>, target: Option<String>) -> ShellMutationOperation {
    let mut paths = Vec::new();
    if let Some(source) = &source {
        paths.push(source.clone());
    }
    if let Some(target) = &target {
        paths.push(target.clone());
    }
    ShellMutationOperation {
        kind: kind.to_owned(),
        status: "changed".to_owned(),
        diagnostic: None,
        source,
        target,
        paths: unique_paths(paths),
    }
}

fn unique_paths(paths: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for path in paths {
        if !path.starts_with('#') && !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn shell_redirection_paths(words: &[String], cwd: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        if matches!(word.as_str(), ">" | "1>" | "2>" | ">>" | "1>>" | "2>>") {
            if let Some(path) = words.get(index + 1) {
                paths.push(resolve_wanix_path(cwd, path));
            }
            index += 2;
            continue;
        }
        if let Some(path) = word.strip_prefix("2>").filter(|path| !path.is_empty()) {
            paths.push(resolve_wanix_path(cwd, path));
        } else if let Some(path) = word.strip_prefix('>').filter(|path| !path.is_empty()) {
            paths.push(resolve_wanix_path(cwd, path));
        }
        index += 1;
    }
    paths
}

fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaping = false;
    for ch in line.chars() {
        if escaping {
            current.push(ch);
            escaping = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaping = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn resolve_wanix_path(cwd: &str, path: &str) -> String {
    if path.starts_with('#') {
        return path.to_owned();
    }
    let rooted = path.starts_with('/');
    let prefix = if rooted || cwd == "." { "" } else { cwd };
    let raw = if rooted || prefix.is_empty() {
        path.to_owned()
    } else {
        format!("{prefix}/{path}")
    };
    let mut parts = Vec::new();
    for part in raw.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use wanix_fs::NormalizedPath;

    use super::{
        ShellInputActivityTracker, operations_for_changed_paths, operations_without_changed_paths,
        resolve_wanix_path, shell_words,
    };

    #[test]
    fn parses_shell_words_with_quotes_and_escapes() {
        assert_eq!(
            shell_words(r#"write "made file.txt" hello\ world"#),
            ["write", "made file.txt", "hello world"]
        );
    }

    #[test]
    fn resolves_wanix_paths_against_shell_cwd() {
        assert_eq!(resolve_wanix_path(".", "made.txt"), "/made.txt");
        assert_eq!(
            resolve_wanix_path("app/sub", "../made.txt"),
            "/app/made.txt"
        );
        assert_eq!(resolve_wanix_path("app", "/made.txt"), "/made.txt");
    }

    #[test]
    fn tracks_write_and_move_operations() {
        let mut tracker = ShellInputActivityTracker::new(&NormalizedPath::new("app").unwrap());
        let operations = tracker.observe_input(b"write made.txt hello\nmv made.txt moved.txt\n");

        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].kind, "write");
        assert_eq!(operations[0].status, "changed");
        assert_eq!(operations[0].target.as_deref(), Some("/app/made.txt"));
        assert_eq!(operations[0].paths, ["/app/made.txt"]);
        assert_eq!(operations[1].kind, "mv");
        assert_eq!(operations[1].source.as_deref(), Some("/app/made.txt"));
        assert_eq!(operations[1].target.as_deref(), Some("/app/moved.txt"));
        assert_eq!(operations[1].paths, ["/app/made.txt", "/app/moved.txt"]);
    }

    #[test]
    fn syncs_cwd_and_filters_operations_to_changed_paths() {
        let mut tracker = ShellInputActivityTracker::new(&NormalizedPath::root());
        tracker.observe_input(b"cd app\n");
        tracker.sync_cwd(&NormalizedPath::new("app").unwrap());
        let operations = tracker.observe_input(b"qjs demo.js > out.txt 2> err.txt\n");
        let filtered = operations_for_changed_paths(
            &operations,
            &["/app/out.txt".to_owned(), "/app/err.txt".to_owned()],
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].kind, "redirect");
        assert_eq!(filtered[0].paths, ["/app/out.txt", "/app/err.txt"]);
    }

    #[test]
    fn marks_operations_without_changed_paths_as_unchanged() {
        let mut tracker = ShellInputActivityTracker::new(&NormalizedPath::new("app").unwrap());
        let operations = tracker.observe_input(b"rm missing.txt\n");
        let unchanged =
            operations_without_changed_paths(&operations, Some("rm: missing.txt: errno -44"));

        assert_eq!(unchanged.len(), 1);
        assert_eq!(unchanged[0].kind, "rm");
        assert_eq!(unchanged[0].status, "unchanged");
        assert_eq!(
            unchanged[0].diagnostic.as_deref(),
            Some("rm: missing.txt: errno -44")
        );
        assert_eq!(unchanged[0].target.as_deref(), Some("/app/missing.txt"));
        assert!(unchanged[0].paths.is_empty());
    }
}
