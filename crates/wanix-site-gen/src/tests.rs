//! Host unit tests for the SSG logic (no wasm toolchain required).
//!
//! These exercise the *same* `generate_site` code that runs in the wasm guest,
//! over both an in-memory [`SiteIo`] and the checked-in fixture corpus on disk.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::{SiteEntry, SiteIo, StdFsIo, generate_site};

/// An in-memory [`SiteIo`] for disk-free host tests. Paths are `/`-separated,
/// relative to the IO root. Directories are implied by file paths.
#[derive(Default)]
struct MemIo {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemIo {
    fn with(files: &[(&str, &str)]) -> Self {
        let io = MemIo::default();
        for (path, body) in files {
            io.files
                .lock()
                .unwrap()
                .insert((*path).to_string(), body.as_bytes().to_vec());
        }
        io
    }

    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned()
    }
}

impl SiteIo for MemIo {
    fn read_dir(&self, dir: &str) -> std::io::Result<Vec<SiteEntry>> {
        let prefix = if dir.is_empty() || dir == "." {
            String::new()
        } else {
            format!("{}/", dir.trim_end_matches('/'))
        };
        let mut names: BTreeMap<String, bool> = BTreeMap::new();
        for path in self.files.lock().unwrap().keys() {
            let Some(rest) = path.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            match rest.split_once('/') {
                Some((seg, _)) => {
                    names.entry(seg.to_string()).or_insert(true);
                }
                None => {
                    names.insert(rest.to_string(), false);
                }
            }
        }
        Ok(names
            .into_iter()
            .map(|(name, is_dir)| SiteEntry { name, is_dir })
            .collect())
    }

    fn read_to_string(&self, path: &str) -> std::io::Result<String> {
        match self.get(path) {
            Some(bytes) => String::from_utf8(bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            None => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        }
    }

    fn create_dir_all(&self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_string(), bytes.to_vec());
        Ok(())
    }
}

const SAMPLE: &[(&str, &str)] = &[
    (
        "content/home.md",
        "---\ntitle: \"Home\"\n---\n# Home\n\nGo to [EIAF](/concepts/everything-is-a-file).\n",
    ),
    (
        "content/concepts/everything-is-a-file.md",
        "---\ntitle: Everything Is a File\n---\n# Everything Is a File\n\nBack [home](/).\n",
    ),
];

#[test]
fn page_count_matches_corpus() {
    let io = MemIo::with(SAMPLE);
    let report = generate_site(&io, "content", "site").expect("generate");
    assert_eq!(report.page_count, 2);
    // One index.html per source page, at served directory paths.
    assert!(
        io.get("site/index.html").is_some(),
        "home -> site/index.html"
    );
    assert!(
        io.get("site/concepts/everything-is-a-file/index.html")
            .is_some(),
        "page -> <section>/<page>/index.html"
    );
}

#[test]
fn frontmatter_is_stripped() {
    let io = MemIo::with(SAMPLE);
    generate_site(&io, "content", "site").expect("generate");
    let html = String::from_utf8(io.get("site/index.html").expect("home html")).expect("utf8");
    assert!(
        !html.contains("title: \"Home\""),
        "frontmatter title line must not leak into output: {html}"
    );
    // The frontmatter delimiter must not survive as body text.
    assert!(
        !html.contains("<main>\n<hr"),
        "leading --- must be consumed as frontmatter, not a horizontal rule"
    );
    assert!(html.contains("<h1>Home</h1>"), "body H1 rendered: {html}");
}

#[test]
fn internal_slug_link_rewritten() {
    let io = MemIo::with(SAMPLE);
    generate_site(&io, "content", "site").expect("generate");
    let html = String::from_utf8(io.get("site/index.html").expect("home html")).expect("utf8");
    // `/concepts/everything-is-a-file` -> served directory form, which resolves
    // to a generated index.html.
    assert!(
        html.contains("href=\"/concepts/everything-is-a-file/\""),
        "internal slug link rewritten to served directory path: {html}"
    );
    assert!(
        io.get("site/concepts/everything-is-a-file/index.html")
            .is_some(),
        "the rewritten link target resolves to a generated file"
    );
}

#[test]
fn gfm_tables_and_tasklists_render() {
    let io = MemIo::with(&[(
        "content/home.md",
        "---\ntitle: Home\n---\n| a | b |\n| - | - |\n| 1 | 2 |\n\n- [x] done\n",
    )]);
    generate_site(&io, "content", "site").expect("generate");
    let html = String::from_utf8(io.get("site/index.html").expect("home html")).expect("utf8");
    assert!(html.contains("<table>"), "GFM table rendered: {html}");
}

#[test]
fn fixture_corpus_generates_with_no_broken_internal_links() {
    // Exercise the identical logic over the checked-in fixture corpus on disk,
    // writing output into a unique temp dir so the source tree stays clean.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let corpus = format!("{manifest}/fixtures/corpus");
    let out_dir = std::env::temp_dir().join(format!(
        "wanix-site-gen-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let in_io = StdFsIo::new(&corpus);
    let out_io = StdFsIo::new(&out_dir);
    let report = generate_site_split(&in_io, &out_io).expect("generate corpus");
    assert_eq!(report.page_count, 3, "home + 2 concept pages");

    // "0 broken internal links": every rewritten internal link in the output
    // must point at a generated file (directory-index form).
    for rel in [
        "index.html",
        "concepts/everything-is-a-file/index.html",
        "concepts/the-filesystem-trait/index.html",
    ] {
        let html = out_io
            .read_to_string(rel)
            .unwrap_or_else(|_| panic!("read {rel}"));
        for target in internal_link_targets(&html) {
            let file = served_target_to_file(&target);
            assert!(
                out_io.read_to_string(&file).is_ok(),
                "broken internal link {target} -> {file} from {rel}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// Helper: generate from one IO root (corpus at its root) into another IO root
/// (output at its root), so input and output can live in separate directories.
fn generate_site_split(
    input: &dyn SiteIo,
    output: &dyn SiteIo,
) -> std::io::Result<crate::SiteReport> {
    generate_site(&SplitIo { input, output }, "", "")
}

/// A [`SiteIo`] that reads from one backing IO and writes to another.
struct SplitIo<'a> {
    input: &'a dyn SiteIo,
    output: &'a dyn SiteIo,
}

impl SiteIo for SplitIo<'_> {
    fn read_dir(&self, dir: &str) -> std::io::Result<Vec<SiteEntry>> {
        self.input.read_dir(dir)
    }
    fn read_to_string(&self, path: &str) -> std::io::Result<String> {
        self.input.read_to_string(path)
    }
    fn create_dir_all(&self, path: &str) -> std::io::Result<()> {
        self.output.create_dir_all(path)
    }
    fn write(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.output.write(path, bytes)
    }
}

/// Extracts `href="/..."` internal targets from rendered HTML.
fn internal_link_targets(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = "href=\"/";
    let mut rest = html;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len() - 1..]; // keep leading '/'
        if let Some(end) = after.find('"') {
            out.push(after[..end].to_string());
            rest = &after[end..];
        } else {
            break;
        }
    }
    out
}

/// Maps a served directory target (`/concepts/foo/` or `/`, optional `#frag`)
/// to its on-disk relative output file.
fn served_target_to_file(target: &str) -> String {
    let path = target.split('#').next().unwrap_or(target);
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "index.html".to_string()
    } else {
        format!("{trimmed}/index.html")
    }
}
