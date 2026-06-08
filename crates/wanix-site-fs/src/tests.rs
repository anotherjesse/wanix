use wanix_fs::MemFs;

use super::{SiteFile, read_site_file};

fn site() -> MemFs {
    let fs = MemFs::new();
    fs.write_file("index.html", b"<h1>home</h1>").unwrap();
    fs.write_file("concepts/foo/index.html", b"<h1>foo</h1>")
        .unwrap();
    fs.write_file("style.css", b"body{color:red}").unwrap();
    fs
}

fn found(result: SiteFile) -> (Vec<u8>, Option<String>) {
    match result {
        SiteFile::Found { bytes, extension } => (bytes, extension),
        SiteFile::NotFound => panic!("expected Found, got NotFound"),
        SiteFile::Forbidden => panic!("expected Found, got Forbidden"),
    }
}

#[test]
fn serves_index_for_root() {
    let fs = site();
    for path in ["", ".", "/"] {
        let (bytes, ext) = found(read_site_file(&fs, path));
        assert_eq!(bytes, b"<h1>home</h1>", "path {path:?}");
        assert_eq!(ext.as_deref(), Some("html"), "path {path:?}");
    }
}

#[test]
fn serves_nested_directory_index() {
    let fs = site();
    let (bytes, ext) = found(read_site_file(&fs, "concepts/foo"));
    assert_eq!(bytes, b"<h1>foo</h1>");
    assert_eq!(ext.as_deref(), Some("html"));

    // A trailing slash on the directory path resolves the same index.
    let (bytes, _) = found(read_site_file(&fs, "concepts/foo/"));
    assert_eq!(bytes, b"<h1>foo</h1>");

    // The explicit index path also resolves.
    let (bytes, _) = found(read_site_file(&fs, "concepts/foo/index.html"));
    assert_eq!(bytes, b"<h1>foo</h1>");
}

#[test]
fn serves_css_with_extension() {
    let fs = site();
    let (bytes, ext) = found(read_site_file(&fs, "style.css"));
    assert_eq!(bytes, b"body{color:red}");
    assert_eq!(ext.as_deref(), Some("css"));
}

#[test]
fn rejects_parent_traversal() {
    let fs = site();
    assert!(matches!(
        read_site_file(&fs, "../etc/passwd"),
        SiteFile::Forbidden
    ));
}

#[test]
fn missing_is_not_found() {
    let fs = site();
    assert!(matches!(
        read_site_file(&fs, "nope/missing.html"),
        SiteFile::NotFound
    ));
    // A directory with no index.html is a clean miss, not a server error.
    assert!(matches!(
        read_site_file(&fs, "concepts"),
        SiteFile::NotFound
    ));
}
