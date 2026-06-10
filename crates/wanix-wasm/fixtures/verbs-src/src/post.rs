//! The `post` verb: a resource-shipped command that posts a message to the
//! resource it came from.
//!
//! Input convention (documented in docs/site/content/concepts/bin-verbs.md):
//! the argv words joined with spaces are the message body; with no argv the
//! body is read from stdin (so `echo hi | room:post` composes in a pipeline).
//! The verb runs confined: its namespace is exactly the resource at `/res`,
//! so the post lands on `/res/post` and nowhere else can be touched.

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let body = if args.is_empty() {
        let mut bytes = Vec::new();
        if let Err(err) = std::io::stdin().read_to_end(&mut bytes) {
            eprintln!("post: stdin: {err}");
            std::process::exit(1);
        }
        bytes
    } else {
        args.join(" ").into_bytes()
    };
    // The guest file exists by declaration: open write-only without
    // create/truncate (O_CREAT|O_TRUNC is refused on device-like files).
    let result = std::fs::OpenOptions::new()
        .write(true)
        .open("/res/post")
        .and_then(|mut file| file.write_all(&body));
    if let Err(err) = result {
        eprintln!("post: /res/post: {err}");
        std::process::exit(1);
    }
}
