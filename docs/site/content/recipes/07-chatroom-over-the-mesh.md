---
title: Recipe 07 — A Chatroom That Is a Mounted Program
slug: recipes/07-chatroom-over-the-mesh
pageType: use-case
oneLiner: app-serve the bundled qjs chatroom with a durable --state dir, post from two principals with no usernames, watch presence and live delivery, defeat a payload impersonation, then kill and restart the room with its history intact.
audience: [developer]
tags: [mesh, cli, apps, appfs, qjs, shipped, caveat]
sourceRefs:
  - examples/chatroom/main.js
  - examples/chatroom/app.wanix.json
  - crates/wanix-cli/src/app/serve.rs:185-202
  - crates/wanix-cli/src/app/guest.rs:11-18
  - crates/wanix-appfs/src/service.rs:162-175
  - crates/wanix-appfs/src/buffer.rs:18-25
  - crates/wanix-cli/src/mesh/ticket.rs:195-206
  - crates/wanix-id/src/identity.rs:90-93
  - crates/wanix-cli/src/serve/webdoor.rs
  - examples/chatroom/web/app.js
seeAlso:
  - learn/build-a-chatroom
  - concepts/guest-defined-resources
  - concepts/key-is-the-address
  - concepts/blocking-stream-eof-contract
  - recipes/06-compose-volume-and-tools
prerequisites:
  - concepts/key-is-the-address
usedInFlows:
  - {flow: build-a-chatroom, step: 2}
honestLimits:
  - "Message timestamps are the host-stamped at_ms on each request (wire v0.2), not the guest's clock — Date.now() inside the served qjs guest is still engine-pinned and should not be used for time."
  - "mount-cat is collected (prints at EOF), so reading the never-EOF stream file shows nothing until the process is killed — bound it with timeout; the live view today is a host-side tail of the --state log."
  - "Any ticket holder may attach (attribution stays unforgeable); allow-list rooms and CAS-pinned code provenance are deferred (docs/appfs.md)."
  - "The WebDoor gateway is ONE principal to the room: every browser user posts as the gateway's dialer key, and a web-set nick names that shared principal. Per-user web identity needs delegation certs / gateway principals (docs/appfs.md §Identity And Trust) — not v0."
canonicalCaveatFor: []
---

# Recipe 07 — A Chatroom That Is a Mounted Program

`app serve` the bundled qjs chatroom with a durable `--state` dir, post from two principals with no usernames, watch presence and live delivery, defeat a payload impersonation, then kill and restart the room with its history intact.

**What & why.** The room is `examples/chatroom/` — a manifest plus one readable `main.js` run as a resident qjs guest behind the `wanix-appfs` file2chan adapter. The host owns the ticket, the verified principals, the never-EOF `stream` fan-out, and `who`; the guest owns only what the files *mean*. Everything below ran verbatim on one machine over loopback (tickets and 64-hex keys shortened to a recognizable prefix; yours will differ). The conceptual walkthrough is [Build a chatroom](/learn/build-a-chatroom).

## 0. Build the binary

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. Serve the room (terminal 1)

```sh
wanix-rust app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0
```

The process parks forever (Ctrl-C to stop) and prints the standard resource record on stderr:

```text
chatroom	iroh://bd4e24de...?addr=127.0.0.1:43490
# mount with: wanix-rust mount-ls 'iroh://bd4e24de...?addr=127.0.0.1:43490'
```

`iroh://bd4e24de...` is the room's persisted identity (`~/.wanix/app-identities/chatroom.key`, created on first serve); `?addr=` is a route hint. The very first start in a fresh environment took ~25 s in this run (debug build, cold module cache compiling the QuickJS wasm) before the ticket appeared; subsequent starts take about a second. `--state /tmp/room` is the room's whole durable memory — the guest sees it at `/state`.

## 2. Person A: mount, post, read

```sh
T='iroh://bd4e24de...?addr=127.0.0.1:43490'
wanix-rust mount-ls "$T"
```

```text
latest
nick
post
roster
status
stream
who
```

```sh
wanix-rust mount-write "$T" post 'morning! mounted the room over the mesh'
# wrote 39 bytes to n/remote/post
wanix-rust mount-cat "$T" latest
```

```text
{"at":1765000000000,"from":"(36469a42)","body":"morning! mounted the room over the mesh"}
```

No username was supplied anywhere. `from` is the derived display of A's persisted dialer key (`~/.wanix/dialer.key`): `(shorthex)` — the first 8 hex chars of the scheme-prefixed principal `iroh:<hex>` taken from the verified QUIC handshake by the serve's attach policy, never from the payload. Writing a name to `nick` renders it `nick (shorthex)` instead; `roster` is the raw principal→nick map, the raw principal stays in the stored log, and you can only ever name *yourself* — a nick is display sugar, never authority (two principals claiming the same nick stay disambiguated by shorthex). `at` is the host-stamped wall clock (`at_ms`) the adapter puts on every request — the guest's one trusted time source (wire v0.2).

## 3. Person B: a second principal on one machine

A friend is just a different key. Simulate one with an alternate `HOME` (all per-user Wanix state lives under `$HOME/.wanix/`; first contact mints `/tmp/bob/.wanix/dialer.key`):

```sh
HOME=/tmp/bob wanix-rust mount-write "$T" post 'hey A — same room, different key'
HOME=/tmp/bob wanix-rust mount-cat "$T" latest
```

```text
{"at":1765000000000,"from":"(36469a42)","body":"morning! mounted the room over the mesh"}
{"at":1765000000113,"from":"(04bd5311)","body":"hey A — same room, different key"}
```

```sh
wanix-rust mount-cat "$T" status
# {"app":"chatroom","messages":2}
```

## 4. Presence: `who` lists open stream subscriptions

`stream` is a never-EOF read, and `mount-cat` is collected — so a stream read parks silently. Use that deliberately, bounded with `timeout`, to hold a subscription open while A asks who is around:

```sh
HOME=/tmp/bob timeout 12 wanix-rust mount-cat "$T" stream &
sleep 2
wanix-rust mount-cat "$T" who
```

```text
iroh:04bd5311...
```

Presence is host session state — the principals currently holding open `stream` subscriptions, one per line, sorted. The guest is never asked.

## 5. The live moment

The room's memory is the `--state` log on the serving machine, appended by the guest on every post *before* it publishes to subscribers. Tail it there while B posts from terminal 2:

```sh
# Terminal 1 (serving machine):
timeout 8 tail -n 0 -f /tmp/room/log
```

```sh
# Terminal 2:
HOME=/tmp/bob wanix-rust mount-write "$T" post 'this line should show up live'
```

The tail prints the line within a beat of the write returning:

```text
{"at":1765000000291,"from":"iroh:04bd5311...","body":"this line should show up live"}
```

(The mesh `stream` file delivers the same line to every parked subscriber buffer — step 4 proved the subscription exists — but no shipped CLI verb prints it incrementally yet; see Troubleshooting.)

## 6. The attribution proof

B posts a JSON body claiming to be A:

```sh
HOME=/tmp/bob wanix-rust mount-write "$T" post \
  '{"from":"36469a42...","body":"hi, this is definitely A"}'
wanix-rust mount-cat "$T" latest | tail -1
```

```text
{"at":1765000000404,"from":"(04bd5311)","body":"hi, this is definitely A"}
```

Still attributed to B. The guest keeps only the `body` of a JSON payload and discards any claimed author; the principal it stamps came from the host, which took it from the transport. Identity rides the transport; `post` carries body only.

## 7. Kill the room; restart it; nothing is lost

Ctrl-C the serve in terminal 1 — abrupt process death, taking the resident guest with it (the serve has no signal handler; no teardown runs). A *fresh* mounted read now gets liveness, not a hang:

```sh
wanix-rust mount-cat "$T" latest
```

```text
resource unreachable: peer bd4e24de... did not answer within 5s — the provider is offline or not discoverable from here, and the mount will work again when it returns (a bare iroh://PEER is found by mDNS on the LAN; pass ?addr=IP:PORT as a direct route hint)
```

The history is sitting in plain host files — `cat /tmp/room/log` shows all four messages. Restart against the same state:

```sh
wanix-rust app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0
# chatroom	iroh://bd4e24de...?addr=127.0.0.1:41108     <- same peer, new port
```

```sh
wanix-rust mount-cat 'iroh://bd4e24de...?addr=127.0.0.1:41108' latest   # all 4 messages
wanix-rust mount-cat "$T" status      # the OLD ticket — stale port hint — still works
```

```text
{"app":"chatroom","messages":4}
```

The room's memory was never guest RAM (`main.js` reloads `/state/log` at boot), the identity key survives the process, and on loopback/LAN even the stale ticket keeps working: the peer half is the address, mDNS finds the new route, the hint is just a hint. Both a bare `iroh://bd4e24de...` (no `?addr=` at all) and the old ticket reached the restarted room in this run.

The manual restart is also automatable: `app serve ... --restart on-failure` supervises the guest, re-running it with capped backoff whenever it exits and swapping the fresh adapter behind the *same* ticket and endpoint. Connections opened against the dead generation keep failing honestly (`Unreachable`); new connections reach the restarted room.

## 8. Put a browser on it: the WebDoor gateway

A browser has no iroh key — so the `serve` HTTP door can act as a gateway:
`--bind NAME=SOURCE` (repeatable) composes, per NAME, one namespace from every
source bound to it and serves it at `http://NAME.localhost:PORT` by
`Host`-header routing (`*.localhost` resolves to loopback by resolver
convention — zero DNS setup). Bind the bundled web client *and* the room under
one name and they share one origin, so the page fetches the room's files with
no CORS:

```sh
T='iroh://bd4e24de...?addr=127.0.0.1:43490'    # the room ticket from step 1
wanix-rust serve --root /tmp/empty \
  --bind chat=examples/chatroom/web \
  --bind "chat=$T" \
  --bind docs=docs/site/content \
  --listen 127.0.0.1:7699
# wanix-rust serve: gateway origin http://chat.localhost:7699/
# wanix-rust serve: gateway origin http://docs.localhost:7699/
```

(`docs` shows the same door serving a plain static site: any directory bound
to a name is an origin — `http://docs.localhost:7699/` lists it as JSON when
there is no `index.html`, and serves files by content type.)

Open `http://chat.localhost:7699/` for the webapp (post form, live
EventSource feed, nick form), or drive it with curl — the bare host lists the
bound names, and the room's files are plain same-origin URLs:

```sh
curl http://127.0.0.1:7699/                            # index of bound names
curl -X POST -d 'hello from curl' http://chat.localhost:7699/post
curl http://chat.localhost:7699/latest
curl -N http://chat.localhost:7699/stream              # chunked, never-EOF: lines arrive as posted
curl -N -H 'Accept: text/event-stream' http://chat.localhost:7699/stream   # same feed as SSE
```

The mapping is generic, not chat-specific: GET of a sized file returns it
whole; GET of a zero-length device file streams as chunked transfer (or SSE
`data:` lines under `Accept: text/event-stream`); POST/PUT writes the body;
GET of a directory serves its `index.html` or a JSON listing; and errors map
honestly — kill the room's `app serve` and `GET /latest` answers
`503 Service Unavailable` with `Retry-After: 5` (an outage, never a 404).
The gateway is loopback-only in v0: a non-loopback `--listen` with `--bind`
is refused at startup (the ADR 0006 trust rule; off-loopback gateway auth is
recorded follow-up work).

> **v0 gateway principals — read this before demoing identity.** The gateway
> dials the room with **its** dialer key (`~/.wanix/dialer.key`), so the room
> sees one principal for ALL web users: every browser post lands as the same
> `(shorthex)` — the gateway's — and a nick set through the web names the
> gateway principal, shared by everyone on that gateway. Session isolation,
> if any, is gateway/HTTP-layer state only, never mesh principal enforcement,
> and the webapp deliberately does not fake per-user attribution: it shows
> messages exactly as the room records them and only locally tags posts it
> sent itself. The mesh wire binds one principal per connection, and that
> connection is dialed by the gateway, not the browser; the future per-user
> path is delegation certs / gateway principals threaded into the attach
> (docs/appfs.md §Identity And Trust), out of scope here.

## Troubleshooting (friction actually hit while testing)

- **`mount-cat "$T" stream` prints nothing, forever.** Not a bug, two contracts stacking: `stream` is an honest never-EOF read (its own QUIC stream, blocking until a publish), and `mount-cat` is collected — it prints only at EOF. Killing it loses the collected bytes. Always bound it with `timeout` (it still registers a real subscription — `who` shows you), read `latest` for history, and tail the `--state` log for a live view. Do not park `cat` on it in the `sh` REPL either: Ctrl-C there cancels the input line, not a blocked read.
- **`who` keeps listing someone who left.** A hard-killed subscriber (process killed mid-read) holds its registry entry until the transport declares the connection dead — about 30 s of silence on loopback in this run. Presence is session state, not a heartbeat.
- **A parked `mount-cat "$T" stream` outlives the serve dying — for ~35 s.** An already-blocked stream read carries no per-op deadline (ADR 0008), so when the serve is killed it keeps blocking until QUIC connection liveness declares the peer dead (~35 s on loopback in our run), then fails with `resource unreachable: ... connection lost` — and its collected bytes are lost. Only a *fresh* op gets the friendly 5 s message above. Blocked stream readers are released with EOF only when the guest exits while the serve stays up.
- **First `app serve` seems to hang before printing the ticket.** Cold module cache: the QuickJS engine wasm is being compiled (~25 s in this debug-build run). Subsequent starts are ~1 s.
- **`app serve` refuses to start without `--addr`.** Default-deny, verbatim: "app serve on the public endpoint exposes the app to anyone with its ticket; pass --addr IP:PORT or --insecure-open to deliberately export to the open internet".
- **Reading `post` fails with `operation not supported: post is write-only; read latest instead`.** It is write-only by the guest's own rules, and the guidance after the colon is the guest's own `not_supported` message, carried across the wire (verbatim from this run: `mount-cat failed to read: operation not supported: post is write-only; read latest instead`).
- **`resource unreachable: peer ... did not answer within 5s`.** The serve is down. The message says the recovery: the mount works again when the room returns, and the peer id half of the ticket never changes.

## Cleanup

Ctrl-C the serve. The room's history stays in `/tmp/room/log`, its identity in `~/.wanix/app-identities/chatroom.key`, your principal in `~/.wanix/dialer.key`, and the simulated friend in `/tmp/bob/.wanix/dialer.key` — delete `/tmp/room` and `/tmp/bob` when you are done with them.
