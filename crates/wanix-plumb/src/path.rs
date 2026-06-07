//! Parsing a `#plumb` path into its addressed form.
//!
//! The device tree is two levels deep: the root lists active topics, each
//! `<topic>` is a directory, and each topic exposes a `send` file (write
//! envelopes) and a `recv` file (read received envelopes). A topic name is a
//! single path segment — it never contains `/` — so the parse is unambiguous.

use wanix_fs::{FsError, FsResult, NormalizedPath};

/// Maximum length, in bytes, of a single topic-name segment.
///
/// A topic name is caller-controlled — over an imported `#plumb`, a remote 9P
/// client supplies it through a walk path — and every distinct name a backend
/// has seen is retained (a gossip membership, a pump task, a directory-listing
/// string). Bounding the name length caps the per-topic string cost and makes
/// the topic namespace enumerable rather than letting a peer name topics with
/// megabyte-long walk segments. 255 bytes matches the conventional Plan 9
/// directory-entry name ceiling and is ample for `kind`-style routing names.
pub(crate) const MAX_TOPIC_LEN: usize = 255;

/// A parsed `#plumb` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlumbPath<'a> {
    /// The device root directory, listing active topics.
    Root,
    /// A topic directory `<topic>`.
    Topic(&'a str),
    /// `<topic>/send`: write a JSON envelope to publish it.
    Send(&'a str),
    /// `<topic>/recv`: read the received newline-JSON envelope stream.
    Recv(&'a str),
}

/// Parses a normalized `#plumb` path, rejecting unknown shapes.
pub(crate) fn parse_path(path: &NormalizedPath) -> FsResult<PlumbPath<'_>> {
    let raw = path.as_str();
    if raw == "." {
        return Ok(PlumbPath::Root);
    }
    let mut parts = raw.split('/');
    let topic = parts.next().ok_or(FsError::NotFound)?;
    if topic.is_empty() {
        return Err(FsError::NotFound);
    }
    if topic.len() > MAX_TOPIC_LEN {
        // Reject an over-long topic segment before it can allocate a membership,
        // a pump task, or a retained directory string. A hostile importer must
        // not be able to name topics with unbounded walk segments.
        return Err(FsError::NotFound);
    }
    match (parts.next(), parts.next()) {
        (None, _) => Ok(PlumbPath::Topic(topic)),
        (Some("send"), None) => Ok(PlumbPath::Send(topic)),
        (Some("recv"), None) => Ok(PlumbPath::Recv(topic)),
        _ => Err(FsError::NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> FsResult<PlumbPath<'_>> {
        let path = NormalizedPath::new(raw).unwrap();
        // SAFETY of lifetime: the test binds `path` long enough; parse borrows it.
        let parsed = parse_path(&path);
        // Re-map to an owned discriminant so the borrow does not escape.
        parsed.map(|p| match p {
            PlumbPath::Root => PlumbPath::Root,
            PlumbPath::Topic(_) => PlumbPath::Topic("t"),
            PlumbPath::Send(_) => PlumbPath::Send("t"),
            PlumbPath::Recv(_) => PlumbPath::Recv("t"),
        })
    }

    #[test]
    fn parses_each_shape() {
        assert_eq!(parse(".").unwrap(), PlumbPath::Root);
        assert_eq!(parse("build").unwrap(), PlumbPath::Topic("t"));
        assert_eq!(parse("build/send").unwrap(), PlumbPath::Send("t"));
        assert_eq!(parse("build/recv").unwrap(), PlumbPath::Recv("t"));
    }

    #[test]
    fn rejects_unknown_leaf() {
        assert!(parse("build/other").is_err());
        assert!(parse("build/send/extra").is_err());
    }

    #[test]
    fn rejects_an_over_long_topic_name() {
        // A name at the ceiling is accepted; one byte over is rejected, so a
        // remote importer cannot allocate unbounded topic strings via walk paths.
        let ok = "a".repeat(MAX_TOPIC_LEN);
        assert!(parse(&ok).is_ok());
        let too_long = "a".repeat(MAX_TOPIC_LEN + 1);
        assert!(NormalizedPath::new(&too_long).is_ok_and(|p| parse_path(&p).is_err()));
        // Over-long names are rejected on the send/recv leaves too.
        let send = format!("{too_long}/send");
        assert!(NormalizedPath::new(&send).is_ok_and(|p| parse_path(&p).is_err()));
    }
}
