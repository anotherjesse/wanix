//! Route map and internal slug-link rewriting.
//!
//! Ported from `mdbook_prep.rs` (`build_route_map`, `rewrite_links`,
//! `rewrite_target`) but the route values are *served directory paths*
//! (`/section/page/`) instead of `*.html`, matching the directory-index output
//! that the Phase 0 FS-backed static handler serves.

use std::collections::BTreeMap;

use crate::page::{Page, file_stem};

/// Maps every authored slug (`/section/page`, `/section`, `/`) to the served
/// directory path the corresponding page is reachable at (`/section/page/`).
pub(crate) fn build_route_map(pages: &[Page]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for page in pages {
        let served = served_path(page);
        if page.section.is_empty() && page.is_index {
            map.insert("/".to_string(), served.clone());
            map.insert(String::new(), served);
        } else if page.is_index {
            map.insert(format!("/{}", page.section), served);
        } else {
            let stem = file_stem(&page.rel);
            map.insert(format!("/{}/{stem}", page.section), served);
        }
    }
    map
}

/// The served directory path for a page (always trailing-slashed; home → `/`).
fn served_path(page: &Page) -> String {
    let stem = file_stem(&page.rel);
    if page.section.is_empty() {
        if stem == "home" {
            return "/".to_string();
        }
        return format!("/{stem}/");
    }
    if page.is_index {
        return format!("/{}/", page.section);
    }
    format!("/{}/{stem}/", page.section)
}

/// Rewrites `](/slug)` and `](/slug#frag)` markdown link targets to served
/// directory paths. External, relative, and anchor-only targets pass through.
pub(crate) fn rewrite_links(body: &str, routes: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'('
            && let Some(close) = body[i + 2..].find(')')
        {
            let target = &body[i + 2..i + 2 + close];
            out.push_str("](");
            out.push_str(&rewrite_target(target, routes));
            out.push(')');
            i = i + 2 + close + 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Rewrites a single link target; leaves non-internal targets untouched.
fn rewrite_target(target: &str, routes: &BTreeMap<String, String>) -> String {
    if !target.starts_with('/') {
        return target.to_string();
    }
    let (path, frag) = match target.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (target, None),
    };
    let key = path.trim_end_matches('/');
    let lookup = if key.is_empty() { "/" } else { key };
    match routes.get(lookup) {
        Some(served) => match frag {
            Some(f) => format!("{served}#{f}"),
            None => served.clone(),
        },
        None => target.to_string(),
    }
}
