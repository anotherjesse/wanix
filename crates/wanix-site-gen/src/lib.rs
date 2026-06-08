//! Static-site generator logic for the Rust-native Wanix port.
//!
//! This crate ports the markdown→HTML pipeline that originally produced an
//! mdBook project (`docs/site/preview/src/mdbook_prep.rs`) but (1) swaps the
//! renderer to [`pulldown-cmark`] so it compiles cleanly to `wasm32-wasip1`
//! (no C dependencies), (2) emits served directory-index output
//! (`<section>/<page>/index.html`) instead of `*.html`, and (3) reads and
//! writes through a small [`SiteIo`] abstraction so the *same* corpus logic
//! runs against the host `std::fs` (host tests) and inside a wasm guest, where
//! libstd's `std::fs` lowers to the WASI calls the task's namespace exposes.
//!
//! The wasm artifact lives in a nested standalone workspace (`wasm-src/`) and a
//! checked-in build of it (`fixtures/site-gen.wasm`) lets `--locked` tests run
//! the generator as a real Wanix `.wasm` task without the wasm toolchain.

mod links;
mod nav;
mod page;
mod render;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use page::SiteEntry;

/// A minimal filesystem abstraction the generator reads input from and writes
/// output into.
///
/// A host implementation backs it with `std::fs` (see [`StdFsIo`]); the wasm
/// guest provides its own `std::fs`-backed implementation, which libstd lowers
/// to WASI calls against the task's preopened namespace. Keeping the corpus
/// logic generic over this trait means the host tests exercise the *identical*
/// code path that runs in the wasm task.
pub trait SiteIo {
    /// Lists the entries directly under `dir` (a path relative to the IO root).
    ///
    /// # Errors
    /// Returns any underlying read error (e.g. the directory does not exist).
    fn read_dir(&self, dir: &str) -> std::io::Result<Vec<SiteEntry>>;

    /// Reads the UTF-8 contents of `path` (relative to the IO root).
    ///
    /// # Errors
    /// Returns any underlying read or decode error.
    fn read_to_string(&self, path: &str) -> std::io::Result<String>;

    /// Creates `path` and every missing parent directory.
    ///
    /// # Errors
    /// Returns any underlying create error.
    fn create_dir_all(&self, path: &str) -> std::io::Result<()>;

    /// Writes `bytes` to `path`, creating or truncating it.
    ///
    /// # Errors
    /// Returns any underlying write error.
    fn write(&self, path: &str, bytes: &[u8]) -> std::io::Result<()>;
}

mod stdio_impl;
pub use stdio_impl::StdFsIo;

/// Summary of a generation run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteReport {
    /// Number of HTML pages emitted (one `index.html` per source page).
    pub page_count: usize,
}

/// Renders the markdown corpus under `input` into an HTML site under `output`,
/// reading and writing through `io`.
///
/// For each source page it strips YAML frontmatter, renders GFM markdown to
/// HTML, rewrites internal slug links (`/section/page`) to the served
/// directory-index form (`/section/page/`), wraps the body in a page template
/// carrying generated navigation, and writes `<section>/<page>/index.html`.
///
/// # Errors
/// Returns any IO error surfaced by `io` while reading the corpus or writing
/// the output tree.
pub fn generate_site(io: &dyn SiteIo, input: &str, output: &str) -> std::io::Result<SiteReport> {
    let mut rel_paths = Vec::new();
    page::collect_markdown(io, input, "", &mut rel_paths)?;
    rel_paths.sort();

    let mut pages = Vec::with_capacity(rel_paths.len());
    for rel in &rel_paths {
        let src = io.read_to_string(&join(input, rel))?;
        pages.push(page::Page::load(rel.clone(), src));
    }

    let routes = links::build_route_map(&pages);
    let nav_html = nav::build_nav(&pages);

    for p in &pages {
        let body_md = render::strip_frontmatter(&p.src);
        let rewritten = links::rewrite_links(body_md, &routes);
        let body_html = render::render_markdown(&rewritten);
        let html = render::wrap_page(&p.title, &body_html, &nav_html);

        let out_rel = p.out_path();
        let out_full = join(output, &out_rel);
        if let Some(parent) = parent_of(&out_full) {
            io.create_dir_all(&parent)?;
        }
        io.write(&out_full, html.as_bytes())?;
    }

    Ok(SiteReport {
        page_count: pages.len(),
    })
}

/// Joins two `/`-separated relative paths, collapsing the `"."` base.
fn join(base: &str, rel: &str) -> String {
    if base.is_empty() || base == "." {
        rel.to_string()
    } else if rel.is_empty() {
        base.to_string()
    } else {
        format!("{}/{rel}", base.trim_end_matches('/'))
    }
}

/// Returns the parent directory of a `/`-separated path, if any.
fn parent_of(path: &str) -> Option<String> {
    path.rfind('/').map(|i| path[..i].to_string())
}

/// Re-exported so callers (and tests) can build a route map for link checks.
#[must_use]
pub fn route_map_for(pages_input: &[(String, String)]) -> BTreeMap<String, String> {
    let pages: Vec<page::Page> = pages_input
        .iter()
        .map(|(rel, src)| page::Page::load(rel.clone(), src.clone()))
        .collect();
    links::build_route_map(&pages)
}
