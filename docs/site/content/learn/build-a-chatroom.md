---
title: Build a chatroom — a guest-defined room over the mesh
slug: learn/build-a-chatroom
pageType: flow
oneLiner: Serve a small qjs program as a mesh-mounted chatroom, post from two identities, claim display nicks the transport can't be fooled by, watch live delivery with mount-cat --follow, fail to impersonate your friend, and restart the room without losing a message.
audience: [newcomer, developer]
tags: [mesh, cli, apps, appfs, qjs, shipped, caveat]
sourceRefs:
  - docs/site/content/recipes/07-chatroom-over-the-mesh.md
  - docs/appfs.md
  - examples/chatroom/main.js
  - examples/chatroom/app.wanix.json
  - crates/wanix-appfs/src/lib.rs:1-40
  - crates/wanix-appfs/src/service.rs:63-99
  - crates/wanix-fs/src/buffer.rs:27-30
  - crates/wanix-cli/src/app/serve.rs:185-202
  - crates/wanix-cli/src/app/guest.rs:11-18
  - crates/wanix-cli/src/mesh/ticket.rs:195-206
  - crates/wanix-id/src/identity.rs:90-93
  - docs/adrs/0008-live-mesh-resource-liveness.md
seeAlso:
  - concepts/guest-defined-resources
  - concepts/jobs-are-files
  - concepts/key-is-the-address
  - concepts/everything-is-a-file
  - concepts/blocking-stream-eof-contract
  - concepts/service-devices
  - recipes/07-chatroom-over-the-mesh
  - recipes/09-web-door-gateway
  - recipes/10-name-your-world
  - learn/publish-apps-via-web-door
  - learn/name-your-world
  - concepts/bin-verbs
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - "Message timestamps are real wall clock — the host stamps at_ms on every request (wire v0.2) and the guest uses it. Date.now() inside the served qjs guest is still engine-pinned (a fixed epoch): guest authors must take time from at_ms, never the engine clock."
  - "who is sessions, not heartbeats: a hard-killed subscriber lingers in who until the transport declares its connection dead (observed ~30 s on loopback)."
  - "Stopping the serve is abrupt process death: a fresh op fails unreachable in ~5 s, but an already-parked stream reader blocks until QUIC liveness fires (33 s observed) and then gets a transport error. With mount-cat --follow the lines already printed are already out; only a collected read loses its buffer. Stream readers are released with EOF only when the guest exits while the serve lives."
  - "A parked stream subscription pins one server session permit and one blocking-pool thread per open file while it lives (ADR 0008 follow-up)."
  - "Every ticket holder may attach (an open room with unforgeable attribution); allow-list rooms are ADR 0007 Layer 2 follow-up. Auto-restart is opt-in (app serve --restart on-failure); by default a dead guest fails ops Unreachable."
canonicalCaveatFor: [engine-pinned-guest-clock]
---

# Build a chatroom — a guest-defined room over the mesh

Serve a small qjs program as a mesh-mounted chatroom, post from two identities, claim display nicks the transport can't be fooled by, watch live delivery with `mount-cat --follow`, fail to impersonate your friend, and restart the room without losing a message.

This flow is the [guest-defined resources](/concepts/guest-defined-resources) idea run end to end. Volumes made *data* a resource; tools made *one operation* a resource ([jobs are files](/concepts/jobs-are-files)). Here a running *program* is the resource: `wanix app serve` turns a small JavaScript file into a mounted filesystem named by one `iroh://` ticket, and everything social about a chatroom — who said what, who is here, what arrives live — falls out of machinery the host already had. One machine, two terminals, about fifteen minutes. The tested transcript with real outputs is [Recipe 07](/recipes/07-chatroom-over-the-mesh).

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix='./target/debug/wanix'
```

## 1. The room is a program you can read

The whole chatroom is `examples/chatroom/` — a manifest and one script. The manifest declares the tree:

```json
{
  "wanix.resource": "v0",
  "kind": "app",
  "name": "chatroom",
  "runtime": { "kind": "qjs", "main": "main.js" },
  "files": ["post", "latest", "status", "nick", "roster"],
  "streams": ["stream"]
}
```

`files`/`streams` here are documentation: on the wire (v0.2) the *guest* is the tree authority — its first output line is a hello declaring its protocol version and tree. And `main.js` is a resident loop you can read in one sitting — this is its entire decision core:

```js
// One discrete operation -> one ok payload (or a thrown {kind, message}).
function handle(request) {
  const { op, path } = request;
  if (op === "stat") {
    if (path === "latest") return { size: textBytes(latestText()).length };
    if (path === "status") return { size: textBytes(statusText()).length };
    if (path === "roster") return { size: textBytes(rosterText()).length };
    return {};
  }
  if (op === "readdir") fail("not_supported", path + " is a file, not a directory");
  if (op === "write") {
    if (path === "post") return post(request, decodeText(request.data || ""));
    if (path === "nick") return setNick(request, decodeText(request.data || ""));
    fail("not_supported", path + " is read-only; write to post or nick");
  }
  if (op === "read") {
    if (path === "latest") return rangeReply(latestText(), request);  // honors offset/len
    if (path === "status") return rangeReply(statusText(), request);
    if (path === "roster") return rangeReply(rosterText(), request);
    fail("not_supported", path + " is write-only; read latest or roster instead");
  }
  fail("not_found", "unknown path " + path);
}

// Wire v0.2 handshake: declare the protocol version and the tree first.
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
  } catch (error) { /* ...one err line, FsError vocabulary... */ }
}
```

That is the file2chan split (`crates/wanix-appfs`): every read or write on a guest file arrives as one JSON line on stdin, the guest answers one line on stdout, and the adapter guarantees one request in flight at a time — the guest is a single actor and concurrency is never its problem. Reads arrive as byte ranges (`offset`/`len`), so a big file is just a sequence of chunked replies. Note what is *absent*: no sockets, no QUIC, no identity handling, no subscriber bookkeeping, no backpressure. The wire never touches this program.

And `post()` makes one decision worth reading twice:

```js
// post: one write is one message. Attribution is NEVER client-claimed: the
// author is the transport-verified principal ("iroh:<hex>") stamped into the
// request event by the host, and the timestamp is the host-stamped at_ms. If
// the body arrives as JSON carrying its own `from`/author, only its `body`
// text is kept and the claimed author is discarded.
function post(request, bodyText) {
  // ...extract body, discard any claimed author...
  const line = JSON.stringify({ at: request.at_ms, from: request.principal, body });
  messages.push(line);
  appendLog(line);                       // durable: /state/log, not guest RAM
  reply({ publish: { stream: "stream", data: encodeText(line + "\n") } });
  return {};
}
```

## 2. Serve it: one ticket names one running room

```sh
# Terminal 1 — parks forever; Ctrl-C to stop.
wanix app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0
```

```text
chatroom	iroh://71b44428...?addr=127.0.0.1:48171
# mount with: wanix mount-ls 'iroh://71b44428...?addr=127.0.0.1:48171'
```

The same two-line record every resource server prints: `iroh://PEER` is the room's persisted ed25519 identity ([the key is the address](/concepts/key-is-the-address)), `?addr=` is a route hint. The first ever start takes noticeably longer (~25 s in our debug-build run) while the QuickJS engine wasm compiles into the module cache; after that it is about a second. No `--addr` is refused outright — default-deny, same as `tool serve` and `volume serve`.

```sh
wanix mount-ls 'iroh://71b44428...?addr=127.0.0.1:48171'
# latest  nick  post  roster  status  stream  who
```

## 3. Post with no username anywhere

```sh
T='iroh://71b44428...?addr=127.0.0.1:48171'   # your full ticket from step 2
wanix mount-write "$T" post 'morning! mounted the room over the mesh'
wanix mount-cat "$T" latest
```

(Serve with `--register room` and `"$T"` can be spelled `room` everywhere below; the room also ships shell verbs — `room:post`, `room:watch` — through its `bin/`. Both executed in [Recipe 10](/recipes/10-name-your-world) and [Name your world](/learn/name-your-world).)

```text
{"at":1781080495768,"from":"(36469a42)","body":"morning! mounted the room over the mesh"}
```

You never typed a name, yet the message is attributed. `from` is the *derived display* of your persisted dialer key — `(shorthex)`, the first 8 hex chars of the scheme-prefixed principal `iroh:<hex>` (`~/.wanix/dialer.key`, `crates/wanix-cli/src/mesh/ticket.rs`). The serve edge binds each connection's *verified* QUIC peer identity into a principal-scoped view (`AppAttachPolicy`, `crates/wanix-cli/src/app/serve.rs`), and the adapter stamps that principal into every event the guest sees. Identity rides the transport; the payload carries body only.

One detail to notice now: `at` is real wall clock, but not the guest's — it is the host-stamped `at_ms` carried on every request (wire v0.2). The guest never trusts its own clock, which inside the served qjs engine is pinned to a fixed epoch; `at_ms` is its one trusted time source, and the timestamps you see are genuinely when the post arrived.

Now claim a nick and watch the rendering update — even for the message already posted:

```sh
wanix mount-write "$T" nick 'ada'
wanix mount-cat "$T" roster
# {"iroh:36469a4270682126...":"ada"}
wanix mount-cat "$T" latest
# {"at":1781080495768,"from":"ada (36469a42)","body":"morning! mounted the room over the mesh"}
```

A nick is display sugar you can only ever claim for *yourself*, never authority: it is keyed by your verified principal, two principals may pick the same nick and the shorthex disambiguates them, and the stored log keeps the raw principal (`roster` is the raw principal→nick map, persisted in `/state/nicks` and reloaded at boot). Display is derived at read time; truth is the key.

## 4. Simulating your friend on one machine

A second person is just a second key. Every per-user Wanix path resolves under `$HOME/.wanix/` (`crates/wanix-id/src/identity.rs`), so an alternate `HOME` is a complete second principal — useful for demos, and an honest statement of what a principal *is*:

```sh
HOME=/tmp/bob wanix mount-write "$T" post 'hey A — same room, different key'
HOME=/tmp/bob wanix mount-cat "$T" latest
```

```text
{"at":1781080495768,"from":"ada (36469a42)","body":"morning! mounted the room over the mesh"}
{"at":1781080509944,"from":"(ed2f9c56)","body":"hey A — same room, different key"}
```

First contact mints `/tmp/bob/.wanix/dialer.key`; B is `ed2f9c56...` from then on, durably. Two people, zero accounts, zero configuration — the handshake is the login. (And note A's nick rendered for B too — the roster belongs to the room, not the reader.)

## 5. Presence is host session state

`who` answers "who is here?" without asking the guest: it lists the principals currently holding open `stream` subscriptions. Park a live subscription as B (terminal 2), then ask as A:

```sh
# Terminal 2 — a real subscription that also prints lines as they arrive:
HOME=/tmp/bob wanix mount-cat "$T" stream --follow
```

```sh
wanix mount-cat "$T" who
# iroh:ed2f9c560c4e8152...
```

Presence here is not a heartbeat the app implements — it is the host's own session table made readable. The flip side is honest too: kill that subscriber hard and `who` keeps listing it until the transport declares the connection dead (~30 s of silence on loopback in our run).

## 6. The live moment, and the contract underneath it

Every open file on the native mesh wire rides its own QUIC stream ([ADR 0008](/concepts/blocking-stream-eof-contract)'s open-file rule), so `stream` can block forever without wedging anything else — the room's guest included, because stream files never touch the guest: the host fans each published line into per-subscriber bounded buffers (the `#plumb` `LineBuffer` discipline — lossy drop-oldest at 1 MiB, the explicit slow-reader policy).

`mount-cat --follow` is the consumer built for exactly this shape: it holds one open handle and prints each read as it returns, instead of collecting until EOF. Terminal 2 is already running it. Post from terminal 1:

```sh
wanix mount-write "$T" post 'this line should show up live'
```

```text
{"at":1781080530768,"from":"iroh:36469a4270682126...","body":"this line should show up live"}
```

...and the line lands in terminal 2 within a beat of the write returning: post → guest appends to `/state/log` → guest publishes → host fans out → every parked subscriber's blocked read completes. The stream carries the raw stored line (raw principal, no nick rendering — rendering is `latest`'s job). The interactive `sh` REPL's `cat` streams the same way at the prompt (Ctrl-C cancels it back to a fresh prompt at a real terminal); plain `mount-cat` *without* `--follow` is still collected — it prints only at EOF, which a live stream never reaches, so bound it with `timeout` if you must use it.

## 7. Impersonation fails politely

B claims to be A inside the payload:

```sh
HOME=/tmp/bob wanix mount-write "$T" post \
  '{"from":"36469a42...","body":"hi, this is definitely A"}'
wanix mount-cat "$T" latest | tail -1
```

```text
{"at":1781080547257,"from":"(ed2f9c56)","body":"hi, this is definitely A"}
```

Still B. The guest extracts `body` and discards the claimed author (`post()` above), and it *could not* honor the claim even if it wanted to — the principal in every event comes from the host, which took it from the QUIC handshake. There is no API for lying about who you are. The lesson generalizes: `post` carries body only; identity rides the transport. (Let B claim the nick `bob` now and the same line re-renders `bob (ed2f9c56)` — the chosen name follows the verified key, including onto the impersonation attempt.)

## 8. Kill the room; the room remembers

Stop the serve with Ctrl-C. That is abrupt process death — the serve has no signal handler, so the guest dies with the host process and no teardown runs. What a mounted client sees depends on what it was doing:

- A **fresh discrete op** (`mount-cat "$T" latest`) fails fast with the mesh's liveness vocabulary, not a hang:

```text
resource unreachable: peer 71b44428... did not answer within 5s — the provider is
offline or not discoverable from here, and the mount will work again when it returns ...
```

- An **already-parked stream reader** is not released promptly: open-file reads carry no per-op deadline ([ADR 0008](/concepts/blocking-stream-eof-contract)), so a parked `mount-cat "$T" stream --follow` keeps blocking until QUIC connection liveness declares the peer dead (33 s observed on loopback), then fails with a transport error (`resource unreachable: ... connection lost`). Every line it printed before the death is already on your screen — `--follow` prints incrementally, so only a *collected* read (no `--follow`) loses its buffered bytes.

The released-with-EOF behavior belongs to a different lifecycle event: when the *guest app* dies while the serve stays up, the host tears the stream surface down, so blocked stream readers observe EOF and discrete ops fail `Unreachable` instead of hanging.

Restart against the same `--state` and read `latest`: every message is back, with the nicks still rendering, because none of it was guest RAM — boot reloads `/state/log` and `/state/nicks`. The identity is persisted too (`~/.wanix/app-identities/chatroom.key`), so the peer half of the ticket is unchanged; only the hinted port moved, and on a LAN even your friend's *old* ticket keeps working (mDNS finds the peer; the stale hint is tolerated). The room is a name that survives its process. Pass `--restart on-failure` and the serve also supervises the *guest*: an exited guest is re-run with capped backoff behind the same ticket and endpoint (the default stays dead-stays-dead).

## Where this goes

The tested transcript with the troubleshooting that real runs produced is [Recipe 07](/recipes/07-chatroom-over-the-mesh). The natural next step is giving the room a browser audience — [publish apps via the web door](/learn/publish-apps-via-web-door) ([Recipe 09](/recipes/09-web-door-gateway)). The one-idea page is [guest-defined resources](/concepts/guest-defined-resources); the design note with CAS-pinned provenance and mount-approval deployment still ahead is `docs/appfs.md`. Compare the shape with [jobs are files](/concepts/jobs-are-files) — host-wrapped operation vs guest-defined resource is the ToolFS/AppFS boundary, and it is deliberate.

## Status / honest limits

- **Engine-pinned guest clock.** `Date.now()` in the served guest returns a fixed epoch. Message timestamps are real anyway because the guest takes time from the host-stamped `at_ms` on every request (wire v0.2) — guest authors must do the same, never trust the engine clock.
- **`--follow` is the streaming consumer; plain `mount-cat` is collected.** `mount-cat --follow` prints the never-EOF `stream` line by line as posts arrive, and the interactive sh REPL's `cat` streams (and Ctrl-C-cancels) at the prompt. Plain `mount-cat` without `--follow` collects until EOF and shows nothing on a live stream — bound it with `timeout`.
- **`who` is sessions.** A vanished subscriber lingers until the transport notices (~30 s observed); a clean drop disappears on the next delivery. A parked subscription also pins one server session permit and one blocking-pool thread per open file while it lives (ADR 0008 follow-up).
- **Serve death is abrupt.** Ctrl-C runs no teardown: fresh ops fail unreachable in ~5 s, but an already-parked stream reader blocks until QUIC liveness fires (33 s observed), then gets a transport error — with `--follow`, everything already printed is kept. Stream EOF release happens only when the guest exits while the serve lives.
- **Open room.** Any ticket holder attaches; attribution is unforgeable but admission control is ADR 0007 follow-up. Auto-restart of the guest is opt-in (`--restart on-failure`); CAS-pinned code provenance is deferred (`docs/appfs.md` §Provenance).
