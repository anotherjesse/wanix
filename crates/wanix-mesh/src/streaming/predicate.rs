//! The predicate that classifies an imported path as a blocking stream.
//!
//! [`StreamingImportFs`](super::StreamingImportFs) routes only *blocking* reads
//! onto their own bidi stream — opening a fresh QUIC stream for every read would
//! be wasteful. The predicate names the service files whose reads park
//! indefinitely: an `#agent` session's event/reply streams and a `#plumb`
//! topic's receive stream. A caller can supply its own predicate for other
//! never-EOF service files.

use std::sync::Arc;

use wanix_fs::NormalizedPath;

/// Decides whether an imported `open` path is a blocking, dedicated-stream read.
///
/// Returns `true` for a path whose read blocks indefinitely (a streaming service
/// file), so it must not share the serial import connection.
pub type StreamPredicate = Arc<dyn Fn(&NormalizedPath) -> bool + Send + Sync>;

/// The default blocking-stream predicate: `#agent`/`#plumb` streaming files.
///
/// Matches, anywhere in the path (so it works whether the import is mounted at
/// `/n/A` or bound elsewhere):
///
/// - `#agent/<id>/events` — the never-EOF normalized event stream,
/// - `#agent/<id>/reply` — blocks until the latest turn completes,
/// - `#plumb/<topic>/recv` — the blocking received-envelope stream.
///
/// These are exactly the imported reads that would freeze a serial 9P import,
/// per the blueprint's head-of-line correction.
#[must_use]
pub fn default_blocking_stream() -> StreamPredicate {
    Arc::new(|path: &NormalizedPath| is_blocking_stream(path.as_str()))
}

/// Classifies a normalized path string against the default blocking-stream set.
fn is_blocking_stream(raw: &str) -> bool {
    let segments: Vec<&str> = raw.split('/').collect();
    // Scan for a `#agent`/`#plumb` device segment followed by `<name>/<leaf>`.
    for window in segments.windows(3) {
        match window {
            [device, _name, leaf] if *device == "#agent" && matches!(*leaf, "events" | "reply") => {
                return true;
            }
            [device, _name, leaf] if *device == "#plumb" && *leaf == "recv" => {
                return true;
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::is_blocking_stream;

    #[test]
    fn matches_agent_streaming_files() {
        assert!(is_blocking_stream("#agent/1/events"));
        assert!(is_blocking_stream("#agent/7/reply"));
        assert!(is_blocking_stream("n/A/#agent/1/events"));
    }

    #[test]
    fn matches_plumb_recv() {
        assert!(is_blocking_stream("#plumb/build/recv"));
        assert!(is_blocking_stream("n/A/#plumb/build/recv"));
    }

    #[test]
    fn does_not_match_request_response_files() {
        // status/prompt/ctl/send are short ops on the shared stream.
        assert!(!is_blocking_stream("#agent/1/status"));
        assert!(!is_blocking_stream("#agent/1/prompt"));
        assert!(!is_blocking_stream("#plumb/build/send"));
        assert!(!is_blocking_stream("notes/hello.txt"));
        assert!(!is_blocking_stream("#agent/new"));
    }
}
