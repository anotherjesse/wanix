//! Source-page discovery and output-path mapping.
//!
//! Ported from `mdbook_prep.rs` (`Page`, `load_page`, `collect_md`) but using
//! the [`SiteIo`](crate::SiteIo) abstraction and emitting served directory-form
//! output paths (`<section>/<page>/index.html`) instead of `*.html`.

use crate::SiteIo;
use crate::render::{first_h1, frontmatter_title};

/// One entry returned by [`SiteIo::read_dir`](crate::SiteIo::read_dir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteEntry {
    /// The entry's file name (no directory components).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

impl SiteEntry {
    /// Creates a directory entry.
    #[must_use]
    pub fn dir(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_dir: true,
        }
    }

    /// Creates a regular-file entry.
    #[must_use]
    pub fn file(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_dir: false,
        }
    }
}

/// One source page discovered under the content root.
pub(crate) struct Page {
    /// Path relative to the content root, e.g. `concepts/everything-is-a-file.md`.
    pub rel: String,
    /// Raw markdown source (frontmatter included).
    pub src: String,
    /// Display title (frontmatter `title`, else first `# H1`, else the slug).
    pub title: String,
    /// Section name (`concepts`, `learn`, …) or empty for the home page.
    pub section: String,
    /// True for `home.md` and any `<section>/index.md`.
    pub is_index: bool,
}

impl Page {
    /// Builds a [`Page`] from its corpus-relative path and raw source.
    pub(crate) fn load(rel: String, src: String) -> Self {
        let stem = file_stem(&rel);
        let section = section_of(&rel);
        let is_index = stem == "index" || (section.is_empty() && stem == "home");
        let title = frontmatter_title(&src)
            .or_else(|| first_h1(&src))
            .unwrap_or_else(|| stem.replace('-', " "));
        Self {
            rel,
            src,
            title,
            section,
            is_index,
        }
    }

    /// The served output path for this page: `<section>/<page>/index.html`.
    ///
    /// The home page and each `<section>/index.md` collapse to the directory's
    /// own `index.html` so they answer `/` and `/<section>/` respectively.
    pub(crate) fn out_path(&self) -> String {
        let stem = file_stem(&self.rel);
        if self.section.is_empty() {
            if stem == "home" {
                return "index.html".to_string();
            }
            return format!("{stem}/index.html");
        }
        if self.is_index {
            return format!("{}/index.html", self.section);
        }
        format!("{}/{stem}/index.html", self.section)
    }
}

/// Returns the file stem (name without directory or `.md` extension).
pub(crate) fn file_stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// Returns the section (first path component) or empty for a top-level file.
pub(crate) fn section_of(rel: &str) -> String {
    match rel.split_once('/') {
        Some((section, _)) => section.to_string(),
        None => String::new(),
    }
}

/// Recursively collects `*.md` paths under `dir` (relative to the IO root),
/// accumulating each path relative to the original `input` root in `acc`.
pub(crate) fn collect_markdown(
    io: &dyn SiteIo,
    input: &str,
    rel_dir: &str,
    acc: &mut Vec<String>,
) -> std::io::Result<()> {
    let full = crate::join(input, rel_dir);
    for entry in io.read_dir(&full)? {
        let child_rel = if rel_dir.is_empty() {
            entry.name.clone()
        } else {
            format!("{rel_dir}/{}", entry.name)
        };
        if entry.is_dir {
            collect_markdown(io, input, &child_rel, acc)?;
        } else if entry.name.ends_with(".md") {
            acc.push(child_rel);
        }
    }
    Ok(())
}
