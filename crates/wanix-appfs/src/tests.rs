//! Adapter tests over a scripted in-process guest channel (no real guest).

use std::collections::VecDeque;
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use wanix_fs::{File, FileSystem, FileType, FsError, NormalizedPath, OpenOptions};

use crate::{
    AppErrKind, AppFs, AppFsService, AppHello, AppOp, AppPublish, AppReceiver, AppReply,
    AppRequest, AppSender, AppTree, GuestLine, MAX_LINE_LEN, PROTO_VERSION, READ_CHUNK_LEN,
    decode_data, encode_data, hello_to_line, publish_to_line,
};

/// A scripted guest: each adapter request is answered by a handler that
/// returns the guest lines (publishes and/or a reply) to deliver. `recv_line`
/// blocks while no line is queued, so a handler that returns no reply models
/// a wedged guest. The state records every request and flags any violation of
/// the one-request-in-flight property.
type Handler = Box<dyn FnMut(&AppRequest) -> Vec<Vec<u8>> + Send>;

const TEST_DEADLINE: Duration = Duration::from_secs(5);

struct ScriptInner {
    handler: Handler,
    queue: VecDeque<Vec<u8>>,
    in_flight: bool,
    overlap: bool,
    sent: Vec<AppRequest>,
}

struct ScriptState {
    inner: Mutex<ScriptInner>,
    signal: Condvar,
}

impl ScriptState {
    fn sent(&self) -> Vec<AppRequest> {
        self.inner.lock().unwrap().sent.clone()
    }

    fn overlap(&self) -> bool {
        self.inner.lock().unwrap().overlap
    }
}

struct ScriptedSender {
    state: Arc<ScriptState>,
}

struct ScriptedReceiver {
    state: Arc<ScriptState>,
}

fn is_reply(line: &[u8]) -> bool {
    matches!(GuestLine::parse(line), Ok(GuestLine::Reply(_)))
}

impl AppSender for ScriptedSender {
    fn send_line(&mut self, line: &[u8]) -> io::Result<()> {
        let mut inner = self.state.inner.lock().unwrap();
        if inner.in_flight {
            inner.overlap = true;
        }
        inner.in_flight = true;
        let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
        let request: AppRequest = serde_json::from_slice(trimmed).expect("well-formed request");
        let lines = (inner.handler)(&request);
        inner.sent.push(request);
        inner.queue.extend(lines);
        drop(inner);
        self.state.signal.notify_all();
        Ok(())
    }
}

impl AppReceiver for ScriptedReceiver {
    fn recv_line(&mut self) -> io::Result<Vec<u8>> {
        let mut inner = self.state.inner.lock().unwrap();
        loop {
            if let Some(line) = inner.queue.pop_front() {
                if is_reply(&line) {
                    inner.in_flight = false;
                }
                return Ok(line);
            }
            // A scripted guest with nothing queued is a wedged guest: block
            // forever, exactly like a real guest that never replies.
            inner = self.state.signal.wait(inner).unwrap();
        }
    }
}

/// A bare v0.2 hello line declaring no tree (the host-declared tree wins).
fn hello_line() -> Vec<u8> {
    hello_to_line(&AppHello {
        proto: PROTO_VERSION,
        files: None,
        streams: None,
    })
    .unwrap()
}

fn scripted(
    handler: impl FnMut(&AppRequest) -> Vec<Vec<u8>> + Send + 'static,
) -> (Box<dyn AppSender>, Box<dyn AppReceiver>, Arc<ScriptState>) {
    scripted_with_first_line(hello_line(), handler)
}

/// A scripted guest whose first (pre-queued) output line is `first` — the
/// handshake seam: tests pick the hello (or a non-hello) the guest opens with.
fn scripted_with_first_line(
    first: Vec<u8>,
    handler: impl FnMut(&AppRequest) -> Vec<Vec<u8>> + Send + 'static,
) -> (Box<dyn AppSender>, Box<dyn AppReceiver>, Arc<ScriptState>) {
    let state = Arc::new(ScriptState {
        inner: Mutex::new(ScriptInner {
            handler: Box::new(handler),
            queue: VecDeque::from([first]),
            in_flight: false,
            overlap: false,
            sent: Vec::new(),
        }),
        signal: Condvar::new(),
    });
    (
        Box::new(ScriptedSender {
            state: Arc::clone(&state),
        }),
        Box::new(ScriptedReceiver {
            state: Arc::clone(&state),
        }),
        state,
    )
}

fn chat_tree() -> AppTree {
    AppTree::declare(&["post", "latest", "history"], &["stream"]).unwrap()
}

fn chat_service(
    handler: impl FnMut(&AppRequest) -> Vec<Vec<u8>> + Send + 'static,
) -> (AppFsService, Arc<ScriptState>) {
    let (sender, receiver, state) = scripted(handler);
    (
        AppFsService::new(chat_tree(), sender, receiver).unwrap(),
        state,
    )
}

fn ok_line(id: u64, ok: serde_json::Value) -> Vec<u8> {
    format!("{{\"id\":{id},\"ok\":{ok}}}\n").into_bytes()
}

fn err_line(id: u64, kind: &str, message: &str) -> Vec<u8> {
    format!("{{\"id\":{id},\"err\":{{\"kind\":\"{kind}\",\"message\":\"{message}\"}}}}\n")
        .into_bytes()
}

fn publish_line(stream: &str, bytes: &[u8]) -> Vec<u8> {
    publish_to_line(&AppPublish {
        stream: stream.to_owned(),
        data: encode_data(bytes),
    })
    .unwrap()
}

fn path(raw: &str) -> NormalizedPath {
    NormalizedPath::new(raw).unwrap()
}

fn open(fs: &AppFs, raw: &str, options: OpenOptions) -> Box<dyn File> {
    fs.open(&path(raw), options).unwrap()
}

fn write_only() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

fn read_to_end(file: &mut dyn File) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0_u8; 7];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn drain_ready(file: &mut dyn File) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0_u8; 64 * 1024];
    while file.read_ready().unwrap() {
        let n = file.read(&mut buf).unwrap();
        out.extend_from_slice(&buf[..n]);
    }
    out
}

// --- protocol shape pins ------------------------------------------------

#[test]
fn request_line_shape_is_pinned() {
    let request = AppRequest {
        id: 7,
        op: AppOp::Write,
        path: "post".to_owned(),
        principal: "iroh:abcd".to_owned(),
        at_ms: 1700000000000,
        data: Some(encode_data(b"hi")),
        offset: None,
        len: None,
    };
    assert_eq!(
        request.to_line().unwrap(),
        b"{\"id\":7,\"op\":\"write\",\"path\":\"post\",\"principal\":\"iroh:abcd\",\
          \"at_ms\":1700000000000,\"data\":\"aGk=\"}\n"
    );
    let read = AppRequest {
        id: 1,
        op: AppOp::Read,
        path: "latest".to_owned(),
        principal: "p".to_owned(),
        at_ms: 5,
        data: None,
        offset: Some(0),
        len: Some(262144),
    };
    assert_eq!(
        read.to_line().unwrap(),
        b"{\"id\":1,\"op\":\"read\",\"path\":\"latest\",\"principal\":\"p\",\"at_ms\":5,\
          \"offset\":0,\"len\":262144}\n"
    );
    for (op, name) in [
        (AppOp::Read, "\"read\""),
        (AppOp::Write, "\"write\""),
        (AppOp::Readdir, "\"readdir\""),
        (AppOp::Stat, "\"stat\""),
    ] {
        assert_eq!(serde_json::to_string(&op).unwrap(), name);
    }
}

#[test]
fn hello_line_shape_is_pinned() {
    let bare = hello_line();
    assert_eq!(bare, b"{\"hello\":{\"proto\":1}}\n");
    let GuestLine::Hello(hello) = GuestLine::parse(&bare).unwrap() else {
        panic!("expected hello");
    };
    assert_eq!(hello.proto, PROTO_VERSION);
    assert!(hello.files.is_none() && hello.streams.is_none());

    let declared =
        GuestLine::parse(b"{\"hello\":{\"proto\":1,\"files\":[\"post\"],\"streams\":[\"s\"]}}\n")
            .unwrap();
    let GuestLine::Hello(hello) = declared else {
        panic!("expected hello");
    };
    assert_eq!(hello.files.as_deref(), Some(&["post".to_owned()][..]));
    assert_eq!(hello.streams.as_deref(), Some(&["s".to_owned()][..]));
    assert!(GuestLine::parse(b"{\"hello\":{\"proto\":1,\"oops\":2}}").is_err());
}

#[test]
fn reply_shapes_are_pinned() {
    let GuestLine::Reply(reply) =
        GuestLine::parse(b"{\"id\":3,\"ok\":{\"data\":\"aGk=\"}}\n").unwrap()
    else {
        panic!("expected reply");
    };
    assert_eq!(reply.id, 3);
    assert_eq!(reply.into_result().unwrap().data_bytes().unwrap(), b"hi");

    let GuestLine::Reply(reply) = GuestLine::parse(
        b"{\"id\":4,\"ok\":{\"entries\":[{\"name\":\"a\",\"dir\":true},{\"name\":\"b\"}]}}",
    )
    .unwrap() else {
        panic!("expected reply");
    };
    let entries = reply.into_result().unwrap().entries.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "a");
    assert!(entries[0].dir);
    assert_eq!(entries[1].name, "b");
    assert!(!entries[1].dir, "dir defaults to false");

    let GuestLine::Reply(reply) = GuestLine::parse(b"{\"id\":5,\"ok\":{}}").unwrap() else {
        panic!("expected reply");
    };
    let ok = reply.into_result().unwrap();
    assert!(ok.data.is_none() && ok.entries.is_none() && ok.size.is_none());
    assert_eq!(ok.data_bytes().unwrap(), b"");

    // A stat reply may declare the file's size (v0.2).
    let GuestLine::Reply(reply) = GuestLine::parse(b"{\"id\":6,\"ok\":{\"size\":42}}").unwrap()
    else {
        panic!("expected reply");
    };
    assert_eq!(reply.into_result().unwrap().size, Some(42));

    assert!(GuestLine::parse(b"{\"id\":1,\"ok\":{},\"oops\":1}").is_err());
    let both = AppReply {
        id: 1,
        ok: Some(crate::AppOk::default()),
        err: Some(crate::AppErr {
            kind: AppErrKind::Other,
            message: String::new(),
        }),
    };
    assert!(both.into_result().is_err());
    let neither = AppReply {
        id: 1,
        ok: None,
        err: None,
    };
    assert!(neither.into_result().is_err());
}

#[test]
fn err_kinds_map_onto_fs_error_vocabulary() {
    let cases = [
        ("not_found", FsError::NotFound),
        ("permission_denied", FsError::PermissionDenied),
        // A message-carrying not_supported keeps its text: the guest's
        // guidance ("read latest instead") must reach the caller instead of
        // being dropped by the payload-free FsError::NotSupported variant.
        (
            "not_supported",
            FsError::Other("operation not supported: detail".to_owned()),
        ),
        // Guest content validation is an argument error, not a path error:
        // the guidance ("nick longer than 32 characters") must read as what
        // it is on every plane.
        ("invalid", FsError::InvalidArgument("detail".to_owned())),
        ("other", FsError::Other("detail".to_owned())),
    ];
    for (kind, expected) in cases {
        let line = err_line(9, kind, "detail");
        let GuestLine::Reply(reply) = GuestLine::parse(&line).unwrap() else {
            panic!("expected reply");
        };
        assert_eq!(reply.into_result().unwrap_err(), expected, "kind {kind}");
    }
    // A messageless not_supported keeps the canonical variant.
    let line = err_line(9, "not_supported", "");
    let GuestLine::Reply(reply) = GuestLine::parse(&line).unwrap() else {
        panic!("expected reply");
    };
    assert_eq!(reply.into_result().unwrap_err(), FsError::NotSupported);
}

#[test]
fn publish_line_shape_and_base64_round_trip() {
    let line = publish_line("stream", b"hello\n");
    assert_eq!(
        line,
        b"{\"publish\":{\"stream\":\"stream\",\"data\":\"aGVsbG8K\"}}\n"
    );
    let GuestLine::Publish(publish) = GuestLine::parse(&line).unwrap() else {
        panic!("expected publish");
    };
    assert_eq!(publish.stream, "stream");
    assert_eq!(decode_data(&publish.data).unwrap(), b"hello\n");

    let bytes: Vec<u8> = (0_u8..=255).collect();
    assert_eq!(decode_data(&encode_data(&bytes)).unwrap(), bytes);
    assert!(decode_data("not base64!!").is_err());
}

#[test]
fn oversized_lines_are_rejected_in_both_directions() {
    let huge = vec![b'x'; MAX_LINE_LEN + 1];
    assert!(GuestLine::parse(&huge).is_err());
    let request = AppRequest {
        id: 1,
        op: AppOp::Write,
        path: "post".to_owned(),
        principal: "p".to_owned(),
        at_ms: 0,
        data: Some(encode_data(&vec![0_u8; MAX_LINE_LEN])),
        offset: None,
        len: None,
    };
    assert!(request.to_line().is_err());
}

// --- discrete-op routing -------------------------------------------------

#[test]
fn read_routes_to_guest_with_principal_and_serves_by_offset() {
    let (service, state) = chat_service(|request| {
        vec![ok_line(
            request.id,
            serde_json::json!({ "data": encode_data(b"hello world") }),
        )]
    });
    let fs = service.open_view("peer-a");
    let mut file = open(&fs, "latest", OpenOptions::read());
    assert_eq!(read_to_end(file.as_mut()), b"hello world");

    let sent = state.sent();
    assert_eq!(sent.len(), 1, "one chunk fetch for many small reads");
    assert_eq!(sent[0].op, AppOp::Read);
    assert_eq!(sent[0].path, "latest");
    assert_eq!(sent[0].principal, "peer-a");
    assert_eq!(sent[0].data, None);
    assert_eq!(sent[0].offset, Some(0));
    assert_eq!(sent[0].len, Some(READ_CHUNK_LEN));
    assert!(sent[0].at_ms > 0, "requests carry host wall-clock time");
}

/// Ranged reads (v0.2): a file larger than one chunk is fetched as a
/// sequence of `READ_CHUNK_LEN` ranges, so the 1 MiB line ceiling no longer
/// caps app file size.
#[test]
fn large_files_are_read_in_chunks_beyond_the_line_ceiling() {
    let chunk = usize::try_from(READ_CHUNK_LEN).unwrap();
    let content: Vec<u8> = (0..chunk * 2 + 17).map(|i| (i % 251) as u8).collect();
    let served = content.clone();
    let (service, state) = chat_service(move |request| {
        let offset = usize::try_from(request.offset.unwrap()).unwrap();
        let len = usize::try_from(request.len.unwrap()).unwrap();
        let end = (offset + len).min(served.len());
        let range = &served[offset.min(served.len())..end];
        vec![ok_line(
            request.id,
            serde_json::json!({ "data": encode_data(range) }),
        )]
    });
    let fs = service.open_view("peer-big");
    let mut file = open(&fs, "latest", OpenOptions::read());
    assert_eq!(read_to_end(file.as_mut()), content);

    let sent = state.sent();
    assert_eq!(sent.len(), 3, "two full chunks plus the short tail");
    assert_eq!(sent[1].offset, Some(READ_CHUNK_LEN));
    assert_eq!(sent[2].offset, Some(READ_CHUNK_LEN * 2));
}

#[test]
fn write_routes_base64_payload_to_guest() {
    let (service, state) = chat_service(|request| vec![ok_line(request.id, serde_json::json!({}))]);
    let fs = service.open_view("peer-w");
    let mut file = open(&fs, "post", write_only());
    assert_eq!(file.write(b"hi").unwrap(), 2);

    let sent = state.sent();
    assert_eq!(sent[0].op, AppOp::Write);
    assert_eq!(sent[0].path, "post");
    assert_eq!(sent[0].principal, "peer-w");
    assert_eq!(sent[0].data.as_deref(), Some("aGk="));
}

#[test]
fn readdir_routes_to_guest() {
    let (service, state) = chat_service(|request| {
        vec![ok_line(
            request.id,
            serde_json::json!({ "entries": [
                { "name": "2024", "dir": true },
                { "name": "msg-1", "dir": false },
            ]}),
        )]
    });
    let fs = service.open_view("peer-d");
    let entries = fs.read_dir(&path("history")).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name(), "2024");
    assert_eq!(entries[0].metadata().file_type(), FileType::Directory);
    assert_eq!(entries[1].name(), "msg-1");
    assert_eq!(entries[1].metadata().file_type(), FileType::File);
    assert_eq!(state.sent()[0].op, AppOp::Readdir);
    assert_eq!(state.sent()[0].principal, "peer-d");
}

#[test]
fn stat_routes_to_guest_and_maps_errors() {
    let (service, state) = chat_service(|request| match request.path.as_str() {
        "post" => vec![ok_line(request.id, serde_json::json!({}))],
        "history" => vec![ok_line(request.id, serde_json::json!({ "size": 42 }))],
        _ => vec![err_line(request.id, "not_found", "gone")],
    });
    let fs = service.open_view("peer-s");
    let meta = fs.metadata(&path("post")).unwrap();
    assert_eq!(meta.file_type(), FileType::File);
    assert_eq!(meta.len(), 0, "no declared size reads honestly as 0");
    assert_eq!(state.sent()[0].op, AppOp::Stat);
    // A guest-declared stat size becomes the reported metadata length
    // (v0.2): the adapter stops lying `len 0` when the guest provides it.
    assert_eq!(fs.metadata(&path("history")).unwrap().len(), 42);
    let file = open(&fs, "history", OpenOptions::read());
    assert_eq!(file.metadata().unwrap().len(), 42);
    assert_eq!(fs.metadata(&path("latest")).unwrap_err(), FsError::NotFound);
}

#[test]
fn guest_error_surfaces_through_read() {
    let (service, _state) =
        chat_service(|request| vec![err_line(request.id, "permission_denied", "members only")]);
    let fs = service.open_view("peer-x");
    let mut file = open(&fs, "latest", OpenOptions::read());
    let mut buf = [0_u8; 4];
    assert_eq!(file.read(&mut buf).unwrap_err(), FsError::PermissionDenied);
}

#[test]
fn mismatched_reply_id_is_a_protocol_error() {
    let (service, _state) = chat_service(|_| vec![ok_line(9999, serde_json::json!({}))]);
    let fs = service.open_view("p");
    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"x").unwrap_err(),
        FsError::Other(message) if message.contains("does not match")
    ));
}

#[test]
fn broken_channel_maps_to_unreachable() {
    struct BrokenHalf;
    impl AppSender for BrokenHalf {
        fn send_line(&mut self, _line: &[u8]) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "guest exited"))
        }
    }
    /// Speaks a clean hello, then the channel breaks.
    struct HelloThenBroken(bool);
    impl AppReceiver for HelloThenBroken {
        fn recv_line(&mut self) -> io::Result<Vec<u8>> {
            if !self.0 {
                self.0 = true;
                return Ok(hello_line());
            }
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "guest exited"))
        }
    }
    // A channel already broken at handshake time fails construction.
    impl AppReceiver for BrokenHalf {
        fn recv_line(&mut self) -> io::Result<Vec<u8>> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "guest exited"))
        }
    }
    assert!(matches!(
        AppFsService::new(chat_tree(), Box::new(BrokenHalf), Box::new(BrokenHalf))
            .map(|_| ())
            .unwrap_err(),
        FsError::Unreachable(_)
    ));
    // A channel that breaks after the handshake unreaches discrete ops.
    let service = AppFsService::new(
        chat_tree(),
        Box::new(BrokenHalf),
        Box::new(HelloThenBroken(false)),
    )
    .unwrap();
    let fs = service.open_view("p");
    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"x").unwrap_err(),
        FsError::Unreachable(_)
    ));
}

// --- v0.2 handshake, op-fatal errors, deadlines ----------------------------

#[test]
fn hello_is_the_tree_authority_and_proto_mismatch_is_refused() {
    // A hello that declares files/streams overrides the host-declared tree.
    let declared = hello_to_line(&AppHello {
        proto: PROTO_VERSION,
        files: Some(vec!["notes".to_owned()]),
        streams: Some(vec!["feed".to_owned()]),
    })
    .unwrap();
    let (sender, receiver, _state) = scripted_with_first_line(declared, |request| {
        vec![ok_line(request.id, serde_json::json!({}))]
    });
    let service = AppFsService::new(chat_tree(), sender, receiver).unwrap();
    let fs = service.open_view("p");
    let names: Vec<String> = fs
        .read_dir(&path("."))
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(names, ["feed", "notes", "who"]);
    assert_eq!(
        fs.metadata(&path("post")).unwrap_err(),
        FsError::NotFound,
        "the manifest-declared tree is demoted to documentation"
    );

    // A proto the host does not speak is a construction-time error.
    let (sender, receiver, _state) =
        scripted_with_first_line(b"{\"hello\":{\"proto\":2}}\n".to_vec(), |_| Vec::new());
    let error = AppFsService::new(chat_tree(), sender, receiver)
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(error, FsError::Other(message) if message.contains("proto")));

    // A first line that is not a hello is a construction-time error.
    let (sender, receiver, _state) =
        scripted_with_first_line(ok_line(1, serde_json::json!({})), |_| Vec::new());
    let error = AppFsService::new(chat_tree(), sender, receiver)
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(error, FsError::Other(message) if message.contains("hello")));
}

/// Op-fatal vs channel-fatal (v0.2): a complete, newline-framed line the
/// host cannot honor as a single reply — oversized or malformed — fails the
/// in-flight op with the specific error, but the channel stays up: framing
/// is line-based, so nothing desynchronized.
#[test]
fn oversized_or_malformed_reply_fails_the_op_but_not_the_channel() {
    let mut bad_lines = VecDeque::from([
        {
            let mut huge = vec![b'x'; MAX_LINE_LEN + 1];
            huge.push(b'\n');
            huge
        },
        b"this is not json\n".to_vec(),
    ]);
    let (service, state) = chat_service(move |request| match bad_lines.pop_front() {
        Some(line) => vec![line],
        None => vec![ok_line(request.id, serde_json::json!({}))],
    });
    let fs = service.open_view("p");

    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"a").unwrap_err(),
        FsError::Other(message) if message.contains("exceeds")
    ));
    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"b").unwrap_err(),
        FsError::Other(message) if message.contains("invalid app guest line")
    ));
    // The channel survived both: the next op reaches the guest and succeeds.
    let mut file = open(&fs, "post", write_only());
    assert_eq!(file.write(b"c").unwrap(), 1);
    assert_eq!(state.sent().len(), 3);
}

/// Per-request deadline (v0.2): a guest that never answers fails the op as
/// `Unreachable` naming the op and deadline, latches the channel down, and
/// closes the stream surface — an unresponsive guest is a dead guest.
#[test]
fn unanswered_op_expires_at_the_deadline_and_latches_down() {
    let (sender, receiver, state) = scripted(|_| Vec::new());
    let service =
        AppFsService::with_op_deadline(chat_tree(), sender, receiver, Duration::from_millis(200))
            .unwrap();
    let fs = service.open_view("p");

    let mut file = open(&fs, "post", write_only());
    match file.write(b"x").unwrap_err() {
        FsError::Unreachable(message) => {
            assert!(message.contains("did not reply to write"), "{message}");
            assert!(message.contains("200ms"), "{message}");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
    // Latched: the next op fails without reaching the guest.
    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"y").unwrap_err(),
        FsError::Unreachable(_)
    ));
    assert_eq!(state.sent().len(), 1, "the downed channel must not be used");
    // The stream surface is torn down: a fresh subscription reads EOF.
    let mut late = open(&fs, "stream", OpenOptions::read());
    let mut buf = [0_u8; 8];
    assert_eq!(late.read(&mut buf).unwrap(), 0);
}

// --- streams, publishes, presence ---------------------------------------

#[test]
fn publish_fans_out_to_all_subscribers_with_independent_cursors() {
    let (service, _state) = chat_service(|request| {
        vec![
            publish_line("stream", b"one\n"),
            publish_line("stream", b"two\n"),
            ok_line(request.id, serde_json::json!({})),
        ]
    });
    let mut sub_a = open(&service.open_view("peer-a"), "stream", OpenOptions::read());
    let mut sub_b = open(&service.open_view("peer-b"), "stream", OpenOptions::read());
    assert!(!sub_a.read_ready().unwrap());

    let fs = service.open_view("peer-w");
    open(&fs, "post", write_only()).write(b"go").unwrap();

    assert_eq!(drain_ready(sub_a.as_mut()), b"one\ntwo\n");
    assert_eq!(drain_ready(sub_b.as_mut()), b"one\ntwo\n");
}

#[test]
fn publish_to_undeclared_stream_fails_the_in_flight_op_and_latches_down() {
    let (service, state) = chat_service(|request| {
        vec![
            publish_line("nope", b"x"),
            ok_line(request.id, serde_json::json!({})),
        ]
    });
    let fs = service.open_view("p");
    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"x").unwrap_err(),
        FsError::Other(message) if message.contains("undeclared stream")
    ));
    // The abandoned in-flight reply must not desynchronize later ops: the
    // protocol violation is terminal, so every subsequent discrete op fails
    // honestly as Unreachable without reaching the guest.
    let mut file = open(&fs, "post", write_only());
    assert!(matches!(
        file.write(b"y").unwrap_err(),
        FsError::Unreachable(message) if message.contains("undeclared stream")
    ));
    assert_eq!(state.sent().len(), 1, "the downed channel must not be used");
    // The guest can never publish through this channel again, so the stream
    // surface is torn down: a fresh subscription reads EOF instead of
    // parking forever.
    let mut late = open(&fs, "stream", OpenOptions::read());
    let mut buf = [0_u8; 8];
    assert_eq!(late.read(&mut buf).unwrap(), 0);
}

/// The host owns pumping (ADR 0010): a publish emitted *after* a reply is
/// fanned out to stream subscribers on arrival, with no further discrete op
/// on the channel to carry it.
#[test]
fn post_reply_publish_reaches_subscribers_without_another_op() {
    let (service, _state) = chat_service(|request| {
        vec![
            ok_line(request.id, serde_json::json!({})),
            publish_line("stream", b"after the reply\n"),
        ]
    });
    let mut sub = open(&service.open_view("peer-r"), "stream", OpenOptions::read());
    open(&service.open_view("peer-w"), "post", write_only())
        .write(b"go")
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0_u8; 64];
        let n = sub.read(&mut buf).unwrap();
        let _ = sender.send(buf[..n].to_vec());
    });
    let bytes = receiver
        .recv_timeout(TEST_DEADLINE)
        .expect("a post-reply publish must be delivered without another op");
    assert_eq!(bytes, b"after the reply\n");
}

#[test]
fn slow_reader_loses_oldest_but_keeps_newest() {
    let half_mib = 512 * 1024;
    let (service, _state) = chat_service(move |request| match request.data {
        None => vec![ok_line(request.id, serde_json::json!({}))],
        Some(_) => {
            let fill = vec![b'f'; half_mib];
            vec![
                publish_line("stream", &fill),
                publish_line("stream", &fill),
                publish_line("stream", &fill),
                publish_line("stream", b"NEWEST\n"),
                ok_line(request.id, serde_json::json!({})),
            ]
        }
    });
    let mut sub = open(
        &service.open_view("peer-slow"),
        "stream",
        OpenOptions::read(),
    );
    let fs = service.open_view("peer-w");
    open(&fs, "post", write_only()).write(b"flood").unwrap();

    let drained = drain_ready(sub.as_mut());
    assert!(
        drained.len() <= crate::MAX_LINE_LEN,
        "buffer must stay under the 1 MiB ceiling, drained {} bytes",
        drained.len()
    );
    assert!(
        drained.ends_with(b"NEWEST\n"),
        "the newest publish must survive drop-oldest eviction"
    );
}

#[test]
fn who_lists_open_subscription_principals_per_line() {
    let (service, _state) =
        chat_service(|request| vec![ok_line(request.id, serde_json::json!({}))]);
    let read_who = |fs: &AppFs| {
        let mut file = open(fs, "who", OpenOptions::read());
        read_to_end(file.as_mut())
    };
    let fs = service.open_view("viewer");
    assert_eq!(read_who(&fs), b"");

    let sub_a = open(&service.open_view("peer-a"), "stream", OpenOptions::read());
    let sub_b1 = open(&service.open_view("peer-b"), "stream", OpenOptions::read());
    let sub_b2 = open(&service.open_view("peer-b"), "stream", OpenOptions::read());
    assert_eq!(read_who(&fs), b"peer-a\npeer-b\n");
    assert_eq!(
        fs.metadata(&path("who")).unwrap().len(),
        b"peer-a\npeer-b\n".len() as u64
    );

    drop(sub_b1);
    assert_eq!(
        read_who(&fs),
        b"peer-a\npeer-b\n",
        "peer-b still subscribed"
    );
    drop(sub_b2);
    assert_eq!(read_who(&fs), b"peer-a\n", "closed subscriptions leave who");
    drop(sub_a);
    assert_eq!(read_who(&fs), b"");
}

// --- liveness and serialization ------------------------------------------

#[test]
fn wedged_guest_blocks_discrete_ops_but_not_stream_reads() {
    // The guest publishes once and then never replies: the discrete op must
    // stay blocked, while a host-owned stream read proceeds with the publish.
    let (service, _state) = chat_service(|_request| vec![publish_line("stream", b"still alive\n")]);
    let mut sub = open(&service.open_view("peer-r"), "stream", OpenOptions::read());

    let (op_done_tx, op_done_rx) = mpsc::channel();
    let op_view = service.open_view("peer-w");
    thread::spawn(move || {
        let mut file = open(&op_view, "post", write_only());
        let result = file.write(b"hello?");
        let _ = op_done_tx.send(result);
    });

    let (stream_tx, stream_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0_u8; 64];
        let n = sub.read(&mut buf).unwrap();
        let _ = stream_tx.send(buf[..n].to_vec());
    });
    let line = stream_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("stream read must proceed while the guest is wedged");
    assert_eq!(line, b"still alive\n");

    assert_eq!(
        op_done_rx.recv_timeout(Duration::from_millis(300)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "the discrete op must still be blocked on the wedged guest"
    );
}

#[test]
fn requests_are_serialized_one_in_flight_at_a_time() {
    let (service, state) = chat_service(|request| {
        vec![ok_line(
            request.id,
            serde_json::json!({ "data": encode_data(b"ok") }),
        )]
    });
    let mut workers = Vec::new();
    for worker in 0..4 {
        let view = service.open_view(format!("peer-{worker}"));
        workers.push(thread::spawn(move || {
            for _ in 0..25 {
                let mut file = open(&view, "latest", OpenOptions::read());
                assert_eq!(read_to_end(file.as_mut()), b"ok");
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(
        !state.overlap(),
        "a second request was sent while one was in flight"
    );
    let sent = state.sent();
    assert_eq!(sent.len(), 100);
    let mut ids: Vec<u64> = sent.iter().map(|request| request.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 100, "operation ids must be unique");
}

// --- tree shape -----------------------------------------------------------

#[test]
fn tree_declaration_is_validated() {
    assert!(AppTree::declare(&["post", "post"], &[]).is_err());
    assert!(AppTree::declare(&["post"], &["post"]).is_err());
    assert!(AppTree::declare(&["who"], &[]).is_err());
    assert!(AppTree::declare(&[], &["who"]).is_err());
    assert!(AppTree::declare(&["a/b"], &[]).is_err());
    assert!(AppTree::declare(&[""], &[]).is_err());
    assert!(AppTree::declare(&["."], &[]).is_err());
    assert!(AppTree::declare(&["post"], &["stream"]).is_ok());
}

#[test]
fn host_answered_tree_shape() {
    let (service, state) = chat_service(|request| vec![ok_line(request.id, serde_json::json!({}))]);
    let fs = service.open_view("p");

    let root = path(".");
    assert_eq!(
        fs.open(&root, OpenOptions::read()).map(|_| ()).unwrap_err(),
        FsError::IsDirectory
    );
    assert_eq!(fs.metadata(&root).unwrap().file_type(), FileType::Directory);
    let names: Vec<String> = fs
        .read_dir(&root)
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(names, ["history", "latest", "post", "stream", "who"]);

    assert_eq!(
        fs.metadata(&path("missing")).unwrap_err(),
        FsError::NotFound
    );
    assert_eq!(
        fs.open(&path("stream"), write_only())
            .map(|_| ())
            .unwrap_err(),
        FsError::PermissionDenied
    );
    assert_eq!(
        fs.open(&path("who"), write_only()).map(|_| ()).unwrap_err(),
        FsError::PermissionDenied
    );
    assert_eq!(
        fs.read_dir(&path("stream")).unwrap_err(),
        FsError::NotDirectory
    );
    assert_eq!(
        fs.read_dir(&path("who")).unwrap_err(),
        FsError::NotDirectory
    );
    // Stream/who metadata and the root listing never consulted the guest.
    assert!(state.sent().is_empty());
}

/// Guest-lifecycle teardown: [`AppFsService::stream_closer`] releases a
/// parked stream reader with EOF and makes later subscriptions start at EOF
/// — the lifecycle-honest inverse of never-EOF once the guest app has exited.
#[test]
fn stream_closer_releases_blocked_readers_with_eof() {
    let (service, _state) = chat_service(|_request| Vec::new());
    let closer = service.stream_closer();
    let view = service.open_view("peer-a");
    let mut parked = open(&view, "stream", OpenOptions::read());

    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buf = [0_u8; 16];
        sender.send(parked.read(&mut buf).unwrap()).unwrap();
    });
    // Let the reader park on the empty buffer, then tear the surface down.
    thread::sleep(Duration::from_millis(50));
    closer.close_all();
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("parked stream reader must be released"),
        0
    );
    reader.join().unwrap();

    // A subscription opened after teardown reads EOF immediately instead of
    // parking on a stream that can never receive another publish.
    let mut late = open(&view, "stream", OpenOptions::read());
    assert!(late.read_ready().unwrap());
    let mut buf = [0_u8; 16];
    assert_eq!(late.read(&mut buf).unwrap(), 0);
}
