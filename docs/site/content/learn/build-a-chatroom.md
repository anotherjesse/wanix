---
title: Build a chatroom — a guest-defined room over the mesh
slug: learn/build-a-chatroom
pageType: flow
oneLiner: Serve a ~120-line qjs program as a mesh-mounted chatroom, post from two identities with no usernames anywhere, watch presence and live delivery, fail to impersonate your friend, and restart the room without losing a message.
audience: [newcomer, developer]
tags: [mesh, cli, apps, appfs, qjs, shipped, caveat]
sourceRefs:
  - docs/site/content/recipes/07-chatroom-over-the-mesh.md
  - docs/appfs.md
  - examples/chatroom/main.js
  - examples/chatroom/app.wanix.json
  - crates/wanix-appfs/src/lib.rs:1-40
  - crates/wanix-appfs/src/service.rs:63-99
  - crates/wanix-appfs/src/buffer.rs:18-25
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
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - "Message timestamps are engine-pinned: Date.now() inside the served qjs guest returns a fixed epoch (observed at=1700000000000 on every message), so ordering is log order, not wall clock."
  - "No shipped CLI verb streams the never-EOF stream file incrementally yet: mount-cat and the shell's cat are collected (they print at EOF), so a parked stream read shows nothing until killed. Bound it with timeout; the live view today is a host-side tail of the --state log."
  - "who is sessions, not heartbeats: a hard-killed subscriber lingers in who until the transport declares its connection dead (observed ~30 s on loopback)."
  - "Stopping the serve is abrupt process death: a fresh op fails unreachable in ~5 s, but an already-parked stream reader blocks until QUIC liveness fires (~35 s observed) and then gets a transport error, losing its collected bytes. Stream readers are released with EOF only when the guest exits while the serve lives."
  - "Every ticket holder may attach (an open room with unforgeable attribution); allow-list rooms are ADR 0007 Layer 2 follow-up. No auto-restart in v0: a dead guest fails ops Unreachable."
canonicalCaveatFor: [engine-pinned-guest-clock]
---

# Build a chatroom — a guest-defined room over the mesh

Serve a ~120-line qjs program as a mesh-mounted chatroom, post from two identities with no usernames anywhere, watch presence and live delivery, fail to impersonate your friend, and restart the room without losing a message.

This flow is the [guest-defined resources](/concepts/guest-defined-resources) idea run end to end. Volumes made *data* a resource; tools made *one operation* a resource ([jobs are files](/concepts/jobs-are-files)). Here a running *program* is the resource: `wanix-rust app serve` turns a small JavaScript file into a mounted filesystem named by one `iroh://` ticket, and everything social about a chatroom — who said what, who is here, what arrives live — falls out of machinery the host already had. One machine, two terminals, about fifteen minutes. The tested transcript with real outputs is [Recipe 07](/recipes/07-chatroom-over-the-mesh).

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. The room is a program you can read

The whole chatroom is `examples/chatroom/` — a manifest and one script. The manifest declares the tree:

```json
{
  "wanix.resource": "v0",
  "kind": "app",
  "name": "chatroom",
  "runtime": { "kind": "qjs", "main": "main.js" },
  "files": ["post", "latest", "status"],
  "streams": ["stream"]
}
```

`files` are routed to the guest as discrete events; `stream` is host-owned and never-EOF; `who` exists implicitly. And `main.js` is a resident loop you can read in one sitting — this is its entire decision core:

```js
// One discrete operation -> one ok payload (or a thrown {kind, message}).
function handle(request) {
  const { op, path, principal } = request;
  if (op === "stat") return {};
  if (op === "readdir") fail("not_supported", path + " is a file, not a directory");
  if (op === "write") {
    if (path === "post") return post(principal, decodeText(request.data || ""));
    fail("not_supported", path + " is read-only; write to post");
  }
  if (op === "read") {
    if (path === "latest") return latest();
    if (path === "status") return status();
    fail("not_supported", "post is write-only; read latest instead");
  }
  fail("not_found", "unknown path " + path);
}

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

That is the file2chan split (`crates/wanix-appfs`): every read or write on a guest file arrives as one JSON line on stdin, the guest answers one line on stdout, and the adapter guarantees one request in flight at a time — the guest is a single actor and concurrency is never its problem. Note what is *absent*: no sockets, no QUIC, no identity handling, no subscriber bookkeeping, no backpressure. The wire never touches this program.

And `post()` makes one decision worth reading twice:

```js
// post: one write is one message. Attribution is NEVER client-claimed: the
// author is the transport-verified principal stamped into the request event
// by the host. If the body arrives as JSON carrying its own `from`/author,
// only its `body` text is kept and the claimed author is discarded.
function post(principal, bodyText) {
  // ...extract body, discard any claimed author...
  const line = JSON.stringify({ at: Date.now(), from: principal, body });
  messages.push(line);
  appendLog(line);                       // durable: /state/log, not guest RAM
  reply({ publish: { stream: "stream", data: encodeText(line + "\n") } });
  return {};
}
```

## 2. Serve it: one ticket names one running room

```sh
# Terminal 1 — parks forever; Ctrl-C to stop.
wanix-rust app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0
```

```text
chatroom	iroh://bd4e24de...?addr=127.0.0.1:43490
# mount with: wanix-rust mount-ls 'iroh://bd4e24de...?addr=127.0.0.1:43490'
```

The same two-line record every resource server prints: `iroh://PEER` is the room's persisted ed25519 identity ([the key is the address](/concepts/key-is-the-address)), `?addr=` is a route hint. The first ever start takes noticeably longer (~25 s in our debug-build run) while the QuickJS engine wasm compiles into the module cache; after that it is about a second. No `--addr` is refused outright — default-deny, same as `tool serve` and `volume serve`.

```sh
wanix-rust mount-ls 'iroh://bd4e24de...?addr=127.0.0.1:43490'
# latest  post  status  stream  who
```

## 3. Post with no username anywhere

```sh
T='iroh://bd4e24de...?addr=127.0.0.1:43490'   # your full ticket from step 2
wanix-rust mount-write "$T" post 'morning! mounted the room over the mesh'
wanix-rust mount-cat "$T" latest
```

```text
{"at":1700000000000,"from":"36469a42...","body":"morning! mounted the room over the mesh"}
```

You never typed a name, yet the message is attributed. `from` is the hex of your persisted dialer key (`~/.wanix/dialer.key`, `crates/wanix-cli/src/mesh/ticket.rs`): the serve edge binds each connection's *verified* QUIC peer identity into a principal-scoped view (`AppAttachPolicy`, `crates/wanix-cli/src/app/serve.rs`), and the adapter stamps that principal into every event the guest sees. Identity rides the transport; the payload carries body only.

One honest oddity to notice now: `at` is always `1700000000000`. `Date.now()` inside the served guest is engine-pinned, so timestamps are deterministic — message order is log order, not wall clock.

## 4. Simulating your friend on one machine

A second person is just a second key. Every per-user Wanix path resolves under `$HOME/.wanix/` (`crates/wanix-id/src/identity.rs`), so an alternate `HOME` is a complete second principal — useful for demos, and an honest statement of what a principal *is*:

```sh
HOME=/tmp/bob wanix-rust mount-write "$T" post 'hey A — same room, different key'
HOME=/tmp/bob wanix-rust mount-cat "$T" latest
```

```text
{"at":1700000000000,"from":"36469a42...","body":"morning! mounted the room over the mesh"}
{"at":1700000000000,"from":"04bd5311...","body":"hey A — same room, different key"}
```

First contact mints `/tmp/bob/.wanix/dialer.key`; B is `04bd5311...` from then on, durably. Two people, zero accounts, zero configuration — the handshake is the login.

## 5. Presence is host session state

`who` answers "who is here?" without asking the guest: it lists the principals currently holding open `stream` subscriptions. Park a bounded subscription as B, then ask as A:

```sh
HOME=/tmp/bob timeout 12 wanix-rust mount-cat "$T" stream &
wanix-rust mount-cat "$T" who
# 04bd5311...
```

Presence here is not a heartbeat the app implements — it is the host's own session table made readable. The flip side is honest too: kill that subscriber hard and `who` keeps listing it until the transport declares the connection dead (~30 s of silence on loopback in our run).

## 6. The live moment, and the contract underneath it

Every open file on the native mesh wire rides its own QUIC stream ([ADR 0008](/concepts/blocking-stream-eof-contract)'s open-file rule), so `stream` can block forever without wedging anything else — the room's guest included, because stream files never touch the guest: the host fans each published line into per-subscriber bounded buffers (the `#plumb` `LineBuffer` discipline — lossy drop-oldest at 1 MiB, the explicit slow-reader policy).

What you must know before you `cat` it: the shipped CLI verbs are *collected* — `mount-cat` prints only at EOF, and `stream` never EOFs while the guest lives. A parked `mount-cat stream` is genuinely subscribed (step 5's `who` proved it) but will show you nothing until killed. So bound it with `timeout`, and watch the live arrival where it is observable today — the room's durable log on the serving machine:

```sh
# Terminal 1 (serving machine): the state log IS the room's memory.
timeout 8 tail -n 0 -f /tmp/room/log &

# Terminal 2: B posts...
HOME=/tmp/bob wanix-rust mount-write "$T" post 'this line should show up live'
```

```text
{"at":1700000000000,"from":"04bd5311...","body":"this line should show up live"}
```

...and the line lands in the tail within a beat of the write returning: post → guest appends to `/state/log` → guest publishes → host fans out. A resident streaming client (the same `getline` loop the room itself runs, pointed at `stream`) is the named next step; today no CLI verb prints the stream incrementally.

## 7. Impersonation fails politely

B claims to be A inside the payload:

```sh
HOME=/tmp/bob wanix-rust mount-write "$T" post \
  '{"from":"36469a42...","body":"hi, this is definitely A"}'
wanix-rust mount-cat "$T" latest | tail -1
```

```text
{"at":1700000000000,"from":"04bd5311...","body":"hi, this is definitely A"}
```

Still B. The guest extracts `body` and discards the claimed author (`post()` above), and it *could not* honor the claim even if it wanted to — the principal in every event comes from the host, which took it from the QUIC handshake. There is no API for lying about who you are. The lesson generalizes: `post` carries body only; identity rides the transport.

## 8. Kill the room; the room remembers

Stop the serve with Ctrl-C. That is abrupt process death — the serve has no signal handler, so the guest dies with the host process and no teardown runs. What a mounted client sees depends on what it was doing:

- A **fresh discrete op** (`mount-cat "$T" latest`) fails fast with the mesh's liveness vocabulary, not a hang:

```text
resource unreachable: peer bd4e24de... did not answer within 5s — the provider is
offline or not discoverable from here, and the mount will work again when it returns ...
```

- An **already-parked stream reader** is not released promptly: open-file reads carry no per-op deadline ([ADR 0008](/concepts/blocking-stream-eof-contract)), so a parked `mount-cat "$T" stream` keeps blocking until QUIC connection liveness declares the peer dead (~35 s observed on loopback), then fails with a transport error (`resource unreachable: ... connection lost`) — and because `mount-cat` is collected, any bytes it had buffered are lost with it.

The released-with-EOF behavior belongs to a different lifecycle event: when the *guest app* dies while the serve stays up, the host tears the stream surface down, so blocked stream readers observe EOF and discrete ops fail `Unreachable` instead of hanging.

Restart against the same `--state` and read `latest`: every message is back, because the history was never guest RAM — boot reloads `/state/log`. The identity is persisted too (`~/.wanix/app-identities/chatroom.key`), so the peer half of the ticket is unchanged; only the hinted port moved, and on a LAN even your friend's *old* ticket keeps working (mDNS finds the peer; the stale hint is tolerated). The room is a name that survives its process.

## Where this goes

The tested transcript with the troubleshooting that real runs produced is [Recipe 07](/recipes/07-chatroom-over-the-mesh). The one-idea page is [guest-defined resources](/concepts/guest-defined-resources); the design note with the HTTP surface, CAS-pinned provenance, and mount-approval deployment still ahead is `docs/appfs.md`. Compare the shape with [jobs are files](/concepts/jobs-are-files) — host-wrapped operation vs guest-defined resource is the ToolFS/AppFS boundary, and it is deliberate.

## Status / honest limits

- **Engine-pinned clock.** `Date.now()` in the served guest returns a fixed epoch; `at` is deterministic, not wall time.
- **No streaming CLI consumer yet.** `mount-cat`/shell `cat` collect until EOF, so the never-EOF `stream` shows nothing until the reader is killed; bound experiments with `timeout` and never park `cat` on it in the sh REPL (Ctrl-C there cancels the input line, not a blocked read).
- **`who` is sessions.** A vanished subscriber lingers until the transport notices (~30 s observed); a clean drop disappears on the next delivery.
- **Serve death is abrupt.** Ctrl-C runs no teardown: fresh ops fail unreachable in ~5 s, but an already-parked stream reader blocks until QUIC liveness fires (~35 s observed), then gets a transport error and loses its collected bytes. Stream EOF release happens only when the guest exits while the serve lives.
- **Open room.** Any ticket holder attaches; attribution is unforgeable but admission control is ADR 0007 follow-up. No auto-restart in v0; CAS-pinned code provenance is deferred (`docs/appfs.md` §Provenance).
