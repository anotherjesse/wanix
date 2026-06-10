// Wanix chatroom — a guest-defined room served as files (docs/appfs.md,
// ADR 0007 "Worked example: a chatroom").
//
// This app IS the resource: `wanix-rust app serve --app examples/chatroom
// --state DIR` runs it as a resident qjs task behind the wanix-appfs
// file2chan adapter. The host owns everything concurrent — open handles,
// never-EOF `stream` subscriptions, presence (`who`) — and delivers each
// discrete filesystem operation to this loop as one JSON line on stdin.
// The guest only decides: read one request, reply one line on stdout,
// repeat until stdin EOF.
//
// Wire v0.2: the guest's FIRST line is a hello declaring its protocol
// version and its tree — the guest is the tree authority, and the manifest's
// files/streams are documentation. Read requests carry a byte range
// (offset/len); every request carries host wall-clock time (at_ms) and the
// transport-verified principal as "iroh:<hex>" (opaque to this app); stat
// replies may declare a size.
//
// The tree (declared by the hello below; documented in app.wanix.json):
//   post     write a message body            (guest: this file)
//   latest   read the last 50 messages       (guest: this file)
//   status   read app status JSON            (guest: this file)
//   stream   never-EOF feed of new messages  (host-owned; fed by publishes)
//   who      current subscriber principals   (host-owned, implicit)
import * as std from "qjs:std";

const LOG = "/state/log"; // durable history; mounted by `app serve --state`
const LATEST_COUNT = 50; // how many messages `latest` returns

// The adapter carries arbitrary bytes as base64. qjs has atob/btoa, which
// speak byte-strings, so text crosses through an explicit UTF-8 step.
const textBytes = (text) => unescape(encodeURIComponent(text)); // one char per byte
const encodeText = (text) => btoa(textBytes(text));
const decodeText = (data) => decodeURIComponent(escape(atob(data)));

// One ranged read reply (v0.2): serve the requested byte range of `text`.
// A reply shorter than the requested len tells the host it hit end-of-file.
function rangeReply(text, request) {
  const bytes = textBytes(text);
  const offset = request.offset || 0;
  const len = request.len === undefined ? bytes.length : request.len;
  return { data: btoa(bytes.slice(offset, offset + len)) };
}

// Guest memory is a cache: history lives in /state/log (one message JSON per
// line) and is reloaded at boot, so the room survives a guest restart.
// v0 keeps the full log in memory; `latest` only ever serves the tail.
const messages = [];
const history = std.loadFile(LOG);
if (history) {
  for (const line of history.split("\n")) {
    if (line) messages.push(line);
  }
}

function reply(object) {
  std.out.puts(JSON.stringify(object) + "\n");
  std.out.flush();
}

// An error the main loop turns into an err reply (FsError vocabulary).
function fail(kind, message) {
  throw { kind, message };
}

function appendLog(line) {
  const file = std.open(LOG, "a");
  if (!file) fail("other", "cannot append " + LOG);
  file.puts(line + "\n");
  file.close();
}

// post: one write is one message. Attribution is NEVER client-claimed: the
// author is the transport-verified principal ("iroh:<hex>") stamped into the
// request event by the host, and the timestamp is the host-stamped at_ms. If
// the body arrives as JSON carrying its own `from`/author, only its `body`
// text is kept and the claimed author is discarded.
function post(request, bodyText) {
  let body = bodyText;
  try {
    const claimed = JSON.parse(bodyText);
    if (claimed && typeof claimed === "object" && typeof claimed.body === "string") {
      body = claimed.body;
    }
  } catch (_ignored) {
    // Plain text body.
  }
  const line = JSON.stringify({ at: request.at_ms, from: request.principal, body });
  messages.push(line);
  appendLog(line);
  // Feed the host-owned `stream` file: the host fans this line out to every
  // current subscriber buffer; this loop never blocks on a slow reader.
  reply({ publish: { stream: "stream", data: encodeText(line + "\n") } });
  return {};
}

function latestText() {
  const recent = messages.slice(-LATEST_COUNT);
  return recent.map((line) => line + "\n").join("");
}

function statusText() {
  return JSON.stringify({ app: "chatroom", messages: messages.length }) + "\n";
}

// One discrete operation -> one ok payload (or a thrown {kind, message}).
function handle(request) {
  const { op, path } = request;
  if (op === "stat") {
    // Declared sizes (v0.2): the host reports them as metadata lengths.
    if (path === "latest") return { size: textBytes(latestText()).length };
    if (path === "status") return { size: textBytes(statusText()).length };
    return {}; // every path the adapter routes is declared
  }
  if (op === "readdir") fail("not_supported", path + " is a file, not a directory");
  if (op === "write") {
    if (path === "post") return post(request, decodeText(request.data || ""));
    fail("not_supported", path + " is read-only; write to post");
  }
  if (op === "read") {
    if (path === "latest") return rangeReply(latestText(), request);
    if (path === "status") return rangeReply(statusText(), request);
    fail("not_supported", "post is write-only; read latest instead");
  }
  fail("not_found", "unknown path " + path);
}

// Wire v0.2 handshake: declare the protocol version and the tree before
// anything else. This hello is the tree authority; app.wanix.json documents
// the same shape for humans and catalogs.
reply({ hello: { proto: 1, files: ["post", "latest", "status"], streams: ["stream"] } });

// The resident loop: one request in flight at a time (the adapter
// guarantees it), blocking in getline between events. stdin EOF means the
// host is gone — exit cleanly.
let line;
while ((line = std.in.getline()) !== null) {
  if (!line) continue;
  const request = JSON.parse(line);
  try {
    reply({ id: request.id, ok: handle(request) });
  } catch (error) {
    reply({
      id: request.id,
      err: {
        kind: typeof error.kind === "string" ? error.kind : "other",
        message: String(error.message || error),
      },
    });
  }
}
