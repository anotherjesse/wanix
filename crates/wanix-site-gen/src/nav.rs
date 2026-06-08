//! Navigation HTML generation.
//!
//! Replaces the mdBook `SUMMARY.md` sidebar with a plain HTML nav: a list of
//! sections, each with its pages, linking to the served directory paths the
//! site is reachable at. Section ordering is ported from `mdbook_prep.rs`.

use crate::page::{Page, file_stem};
use crate::render::escape_html;

/// Section order in the nav; anything else is appended afterwards.
const SECTION_ORDER: &[&str] = &[
    "learn",
    "concepts",
    "devices",
    "use-cases",
    "recipes",
    "reference",
    "find",
];

/// Builds the navigation HTML fragment shared by every generated page.
pub(crate) fn build_nav(pages: &[Page]) -> String {
    let mut out = String::new();

    if let Some(home) = pages.iter().find(|p| p.section.is_empty() && p.is_index) {
        out.push_str(&format!(
            "<a class=\"nav-home\" href=\"/\">{}</a>\n",
            escape_html(&home.title)
        ));
    }

    let mut sections: Vec<&str> = pages
        .iter()
        .map(|p| p.section.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    sections.sort_unstable();
    sections.dedup();
    sections.sort_by_key(|s| {
        SECTION_ORDER
            .iter()
            .position(|x| x == s)
            .unwrap_or(usize::MAX)
    });

    for section in sections {
        out.push_str(&format!(
            "<section class=\"nav-section\">\n<h2>{}</h2>\n<ul>\n",
            escape_html(&section_title(section))
        ));
        let mut in_section: Vec<&Page> = pages.iter().filter(|p| p.section == section).collect();
        in_section.sort_by(|a, b| {
            b.is_index
                .cmp(&a.is_index)
                .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        for page in in_section {
            out.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>\n",
                nav_href(page),
                escape_html(&page.title)
            ));
        }
        out.push_str("</ul>\n</section>\n");
    }
    out
}

/// The served directory path a nav entry links to.
fn nav_href(page: &Page) -> String {
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

/// Title-cases a section name for display.
fn section_title(section: &str) -> String {
    match section {
        "use-cases" => "Use Cases".to_string(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => other.to_string(),
            }
        }
    }
}
