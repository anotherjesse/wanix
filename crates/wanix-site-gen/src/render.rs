//! Frontmatter handling, GFM markdown rendering, and the page template.
//!
//! Frontmatter stripping and the `title`/first-H1 helpers are ported verbatim
//! from `mdbook_prep.rs`; rendering uses [`pulldown-cmark`] with the GFM
//! extensions enabled.

use pulldown_cmark::{Options, Parser, html};

/// Renders GFM markdown `body` (frontmatter already stripped) into an HTML
/// fragment string.
pub(crate) fn render_markdown(body: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(body, options);
    let mut out = String::with_capacity(body.len() * 3 / 2);
    html::push_html(&mut out, parser);
    out
}

/// Wraps a rendered body fragment in a minimal standalone HTML document with
/// the generated navigation.
pub(crate) fn wrap_page(title: &str, body_html: &str, nav_html: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n</head>\n<body>\n<nav>\n{nav_html}</nav>\n\
<main>\n{body_html}</main>\n</body>\n</html>\n",
        title = escape_html(title),
    )
}

/// Escapes the small set of characters unsafe in HTML text/attribute context.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Strips a leading `---`-delimited YAML frontmatter block.
pub(crate) fn strip_frontmatter(src: &str) -> &str {
    let trimmed = src.trim_start();
    if !trimmed.starts_with("---") {
        return src;
    }
    let after_open = match trimmed.find('\n') {
        Some(n) => &trimmed[n + 1..],
        None => return src,
    };
    if let Some(pos) = after_open.find("\n---") {
        let rest = &after_open[pos + 4..];
        return rest.strip_prefix('\n').unwrap_or(rest);
    }
    src
}

/// Extracts the `title:` value from a leading frontmatter block, if present.
pub(crate) fn frontmatter_title(src: &str) -> Option<String> {
    let mut lines = src.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.trim().strip_prefix("title:") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Returns the first `# H1` heading text, if any.
pub(crate) fn first_h1(src: &str) -> Option<String> {
    src.lines()
        .find_map(|l| l.strip_prefix("# ").map(|h| h.trim().to_string()))
}
