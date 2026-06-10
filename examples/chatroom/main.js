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
//   nick     write your display name         (guest: this file, write-only)
//   roster   read the principal->nick map    (guest: this file)
//   stream   never-EOF feed of new messages  (host-owned; fed by publishes)
//   who      current subscriber principals   (host-owned, implicit)
//
// `who` stays RAW host truth — full scheme-prefixed principals, one per line.
// It is host-owned session state and must keep working when this guest is
// dead, so it never passes through nick rendering; clients shorten it for
// display themselves.
import * as std from "qjs:std";

const LOG = "/state/log"; // durable history; mounted by `app serve --state`
const NICKS = "/state/nicks"; // durable principal->nick JSON map
const LATEST_COUNT = 50; // how many messages `latest` returns
const NICK_MAX = 32; // display-name length cap (characters)

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

// Nicks are self-claimed display names keyed by the transport-verified
// principal — reloaded at boot like the log, so they survive a guest restart.
let nicks = {};
const savedNicks = std.loadFile(NICKS);
if (savedNicks) {
  try {
    nicks = JSON.parse(savedNicks);
  } catch (_corrupt) {
    nicks = {}; // an unreadable map loses nicks, never the room
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

function saveNicks() {
  const file = std.open(NICKS, "w");
  if (!file) fail("other", "cannot write " + NICKS);
  file.puts(JSON.stringify(nicks) + "\n");
  file.close();
}

// shorthex = the first 8 hex chars after the scheme prefix of a principal
// like "iroh:<64-hex>" (a scheme-less principal falls back to its first 8
// chars). Display only; the full principal is the identity.
function shortHex(principal) {
  const colon = principal.indexOf(":");
  return (colon >= 0 ? principal.slice(colon + 1) : principal).slice(0, 8);
}

// Display attribution is DERIVED: "nick (shorthex)" when the author claimed
// a nick, "(shorthex)" otherwise. A nick is display sugar, never authority —
// two principals may claim the same string and the shorthex disambiguates.
function displayFrom(principal) {
  const short = "(" + shortHex(principal) + ")";
  const nick = nicks[principal];
  return typeof nick === "string" ? nick + " " + short : short;
}

// nick: you can only ever name YOURSELF. The verified principal of the
// request is the only key ever used; the body is just the nick string.
function setNick(request, bodyText) {
  const nick = bodyText.trim();
  if (!nick) fail("invalid", "empty nick");
  if (nick.length > NICK_MAX) {
    fail("invalid", "nick longer than " + NICK_MAX + " characters");
  }
  if (/[\u0000-\u001f\u007f]/.test(nick)) {
    fail("invalid", "nick contains control characters");
  }
  nicks[request.principal] = nick;
  saveNicks();
  return {};
}

// post: one non-empty write is one message. Attribution is NEVER
// client-claimed: the author is the transport-verified principal
// ("iroh:<hex>") stamped into the request event by the host, and the
// timestamp is the host-stamped at_ms. If the body arrives as JSON carrying
// its own `from`/author, only its `body` text is kept and the claimed author
// is discarded. A zero-length write is a flush, not a message (buffered-stdio
// clients — e.g. a qjs verb's std file close — emit one on close); ignore it.
function post(request, bodyText) {
  if (bodyText.length === 0) return {};
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

// latest renders attribution as "nick (shorthex)" — display derived at read
// time. The raw principal stays in the stored log and in stream publishes:
// display is derived, truth is the key.
function latestText() {
  return messages
    .slice(-LATEST_COUNT)
    .map((stored) => {
      const message = JSON.parse(stored);
      const rendered = { at: message.at, from: displayFrom(message.from), body: message.body };
      return JSON.stringify(rendered) + "\n";
    })
    .join("");
}

function rosterText() {
  return JSON.stringify(nicks) + "\n";
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
    if (path === "roster") return { size: textBytes(rosterText()).length };
    return {}; // every path the adapter routes is declared
  }
  if (op === "readdir") fail("not_supported", path + " is a file, not a directory");
  if (op === "write") {
    if (path === "post") return post(request, decodeText(request.data || ""));
    if (path === "nick") return setNick(request, decodeText(request.data || ""));
    fail("not_supported", path + " is read-only; write to post or nick");
  }
  if (op === "read") {
    if (path === "latest") return rangeReply(latestText(), request);
    if (path === "status") return rangeReply(statusText(), request);
    if (path === "roster") return rangeReply(rosterText(), request);
    fail("not_supported", path + " is write-only; read latest or roster instead");
  }
  fail("not_found", "unknown path " + path);
}

// Wire v0.2 handshake: declare the protocol version and the tree before
// anything else. This hello is the tree authority; app.wanix.json documents
// the same shape for humans and catalogs.
reply({
  hello: {
    proto: 1,
    files: ["post", "latest", "status", "nick", "roster"],
    streams: ["stream"],
  },
});

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
