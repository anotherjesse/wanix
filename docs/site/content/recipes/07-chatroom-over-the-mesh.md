---
title: Recipe 07 — A Chatroom That Is a Mounted Program
slug: recipes/07-chatroom-over-the-mesh
pageType: use-case
oneLiner: app-serve the bundled qjs chatroom with a durable --state dir, post from two principals, claim nicks that render but never authenticate, watch live delivery with mount-cat --follow, defeat a payload impersonation, then kill and restart the room with its history and nicks intact.
audience: [developer]
tags: [mesh, cli, apps, appfs, qjs, shipped, caveat]
sourceRefs:
  - examples/chatroom/main.js
  - examples/chatroom/app.wanix.json
  - crates/wanix-cli/src/app/serve.rs:185-202
  - crates/wanix-cli/src/app/guest.rs:11-18
  - crates/wanix-appfs/src/service.rs:162-175
  - crates/wanix-fs/src/buffer.rs:27-30
  - crates/wanix-cli/src/mesh/ticket.rs:195-206
  - crates/wanix-id/src/identity.rs:90-93
seeAlso:
  - learn/build-a-chatroom
  - concepts/guest-defined-resources
  - concepts/key-is-the-address
  - concepts/blocking-stream-eof-contract
  - recipes/06-compose-volume-and-tools
  - recipes/09-web-door-gateway
  - recipes/10-name-your-world
  - concepts/bin-verbs
prerequisites:
  - concepts/key-is-the-address
usedInFlows:
  - {flow: build-a-chatroom, step: 2}
honestLimits:
  - "Message timestamps are real wall clock: the host stamps at_ms on every request (wire v0.2). The guest's own Date.now() is still engine-pinned and must not be used for time — the shipped chatroom uses at_ms."
  - "A parked stream reader pins one server session permit and one blocking-pool thread per open file while it lives, and after abrupt serve death it blocks until QUIC liveness fires (~33 s observed) — though with --follow every line it already printed is already out."
  - "Any ticket holder may attach (attribution stays unforgeable); allow-list rooms and CAS-pinned code provenance are deferred (docs/appfs.md)."
  - "The WebDoor gateway (recipe 09) is ONE principal to the room: every browser user posts as the gateway's dialer key, and a web-set nick names that shared principal. Per-user web identity needs delegation certs / gateway principals (docs/appfs.md §Identity And Trust) — not v0."
canonicalCaveatFor: []
---

# Recipe 07 — A Chatroom That Is a Mounted Program

`app serve` the bundled qjs chatroom with a durable `--state` dir, post from two principals, claim nicks that render but never authenticate, watch live delivery with `mount-cat --follow`, defeat a payload impersonation, then kill and restart the room with its history and nicks intact.

**What & why.** The room is `examples/chatroom/` — a manifest plus one readable `main.js` run as a resident qjs guest behind the `wanix-appfs` file2chan adapter. The host owns the ticket, the verified principals, the never-EOF `stream` fan-out, and `who`; the guest owns only what the files *mean*. Everything below ran verbatim on one machine over loopback (tickets and 64-hex keys shortened to a recognizable prefix; yours will differ). The conceptual walkthrough is [Build a chatroom](/learn/build-a-chatroom).

## 0. Build the binary

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix='./target/debug/wanix'
ls examples/chatroom/main.js examples/chatroom/app.wanix.json   # the room ships in-tree
```

## 1. Serve the room (terminal 1)

```sh
wanix app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0
```

The process parks forever (Ctrl-C to stop) and prints the standard resource record on stderr:

```text
chatroom	iroh://71b44428...?addr=127.0.0.1:48171
# mount with: wanix mount-ls 'iroh://71b44428...?addr=127.0.0.1:48171'
```

`iroh://71b44428...` is the room's persisted identity (`~/.wanix/app-identities/chatroom.key`, created on first serve); `?addr=` is a route hint. The very first start in a fresh environment takes ~25 s (debug build, cold module cache compiling the QuickJS wasm) before the ticket appears; subsequent starts take about a second. `--state /tmp/room` is the room's whole durable memory — the guest sees it at `/state`.

Optional: add `--register room` and the serve also writes a catalog entry, so every `"$T"` below can be spelled `room` instead (`wanix mount-cat room latest`, `--mount-mesh room`) — executed end to end in [Recipe 10](/recipes/10-name-your-world), along with the room's shipped `bin/` verbs (`room:post`, `room:watch`, `room:roster` — [bin verbs](/concepts/bin-verbs)). The ticket flow below is the no-catalog fallback and what a name resolves to.

## 2. Person A: mount, post, read

```sh
T='iroh://71b44428...?addr=127.0.0.1:48171'   # <- replace with the full iroh:// ticket YOUR terminal 1 printed
wanix mount-ls "$T"
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
wanix mount-write "$T" post 'morning! mounted the room over the mesh'
# wrote 39 bytes to n/remote/post
wanix mount-cat "$T" latest
```

```text
{"at":1781080495768,"from":"(36469a42)","body":"morning! mounted the room over the mesh"}
```

No username was supplied anywhere. `from` is the derived display of A's persisted dialer key (`~/.wanix/dialer.key`): `(shorthex)` — the first 8 hex chars of the scheme-prefixed principal `iroh:<hex>` taken from the verified QUIC handshake by the serve's attach policy, never from the payload. And `at` is real wall clock: the host stamps `at_ms` on every request (wire v0.2) — the guest's one trusted time source.

## 3. Claim a nick — display sugar, never authority

```sh
wanix mount-write "$T" nick 'ada'
# wrote 3 bytes to n/remote/nick
wanix mount-cat "$T" roster
```

```text
{"iroh:36469a4270682126...":"ada"}
```

`roster` is the raw principal→nick map. `latest` now renders A's messages — including the one already posted — as `nick (shorthex)`:

```sh
wanix mount-cat "$T" latest
# {"at":1781080495768,"from":"ada (36469a42)","body":"morning! mounted the room over the mesh"}
```

The stored log keeps the raw principal; display is derived at read time, truth is the key. A nick is keyed *only* by the writer's verified principal, so you can only ever name yourself, and two principals claiming the same nick stay disambiguated by shorthex.

## 4. Person B: a second principal on one machine

A friend is just a different key. Simulate one with an alternate `HOME` (all per-user Wanix state lives under `$HOME/.wanix/`; first contact mints `/tmp/bob/.wanix/dialer.key`):

```sh
HOME=/tmp/bob wanix mount-write "$T" post 'hey A — same room, different key'
HOME=/tmp/bob wanix mount-cat "$T" latest
```

```text
{"at":1781080495768,"from":"ada (36469a42)","body":"morning! mounted the room over the mesh"}
{"at":1781080509944,"from":"(ed2f9c56)","body":"hey A — same room, different key"}
```

```sh
wanix mount-cat "$T" status
# {"app":"chatroom","messages":2}
```

## 5. The live moment: `mount-cat --follow`, and `who`

`stream` is an honest never-EOF read, and `mount-cat --follow` is its streaming consumer — it prints each line as it arrives on one open handle. Park B on the stream in terminal 2:

```sh
# Terminal 2 — blocks, printing lines as they arrive (Ctrl-C to stop):
HOME=/tmp/bob wanix mount-cat "$T" stream --follow
```

While that subscription is open, A asks who is around:

```sh
wanix mount-cat "$T" who
# iroh:ed2f9c560c4e8152...
```

Presence is host session state — the principals currently holding open `stream` subscriptions, one per line, sorted. The guest is never asked. Now A posts:

```sh
wanix mount-write "$T" post 'this line should show up live'
```

...and terminal 2 prints the line within a beat of the write returning:

```text
{"at":1781080530768,"from":"iroh:36469a4270682126...","body":"this line should show up live"}
```

(The stream fan-out carries the raw stored line — raw principal, no nick rendering; rendering is `latest`'s job.)

## 6. The attribution proof

B posts a JSON body claiming to be A:

```sh
HOME=/tmp/bob wanix mount-write "$T" post \
  '{"from":"36469a42...","body":"hi, this is definitely A"}'
wanix mount-cat "$T" latest | tail -1
```

```text
{"at":1781080547257,"from":"(ed2f9c56)","body":"hi, this is definitely A"}
```

Still attributed to B. The guest keeps only the `body` of a JSON payload and discards any claimed author; the principal it stamps came from the host, which took it from the transport. Identity rides the transport; `post` carries body only. And a nick changes nothing — let B claim one now:

```sh
HOME=/tmp/bob wanix mount-write "$T" nick 'bob'
wanix mount-cat "$T" roster
# {"iroh:36469a4270682126...":"ada","iroh:ed2f9c560c4e8152...":"bob"}
wanix mount-cat "$T" latest | tail -1
# {"at":1781080547257,"from":"bob (ed2f9c56)","body":"hi, this is definitely A"}
```

The impersonation attempt is now signed with B's *own chosen name* — the rendering follows the verified key wherever it goes.

## 7. Kill the room; restart it; nothing is lost

Ctrl-C the serve in terminal 1 — abrupt process death, taking the resident guest with it (the serve has no signal handler; no teardown runs). A *fresh* mounted read now gets liveness, not a hang:

```sh
wanix mount-cat "$T" latest
```

```text
resource unreachable: peer 71b44428... did not answer within 5s — the provider is offline or not discoverable from here, and the mount will work again when it returns (a bare iroh://PEER is found by mDNS on the LAN; pass ?addr=IP:PORT as a direct route hint)
```

The history is sitting in plain host files — `ls /tmp/room` shows `log` and `nicks`, and `cat /tmp/room/log` shows all four messages with raw principals. Restart against the same state, this time with the guest supervisor on:

```sh
wanix app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0 --restart on-failure
# chatroom	iroh://71b44428...?addr=127.0.0.1:53497     <- same peer, new port
```

```sh
wanix mount-cat 'iroh://71b44428...?addr=127.0.0.1:53497' latest
```

```text
{"at":1781080495768,"from":"ada (36469a42)","body":"morning! mounted the room over the mesh"}
{"at":1781080509944,"from":"bob (ed2f9c56)","body":"hey A — same room, different key"}
{"at":1781080530768,"from":"ada (36469a42)","body":"this line should show up live"}
{"at":1781080547257,"from":"bob (ed2f9c56)","body":"hi, this is definitely A"}
```

```sh
wanix mount-cat "$T" status      # the OLD ticket — stale port hint — still works
# {"app":"chatroom","messages":4}
```

The room's memory was never guest RAM (`main.js` reloads `/state/log` *and* `/state/nicks` at boot — the nick rendering survived the restart above), the identity key survives the process, and on loopback/LAN even the stale ticket keeps working: the peer half is the address, mDNS finds the new route, the hint is just a hint. Both a bare `iroh://71b44428...` (no `?addr=` at all) and the old ticket reached the restarted room in this run.

`--restart on-failure` supervises the *guest*: whenever it dies — it exits, or it stops answering past the 30 s op deadline (a wedged handler; the channel latches down and the generation is abandoned) — the supervisor re-runs it with capped backoff and swaps the fresh adapter behind the *same* ticket and endpoint. Connections opened against the dead generation keep failing honestly (`Unreachable`); new connections reach the restarted room. The default stays dead-stays-dead.

## 8. Put a browser on it

A browser has no iroh key — so the `serve` HTTP door can act as a gateway:
`serve --bind chat=examples/chatroom/web --bind "chat=$T"` composes the
bundled web client and the room into one origin at
`http://chat.localhost:PORT`, with posts, the live EventSource feed, and the
nick form working in a plain browser. That is its own tested transcript —
[Recipe 09 — The web door](/recipes/09-web-door-gateway) — including the one
identity caveat to read before demoing: the gateway is a *single* principal
to the room, so every browser user posts as the gateway's key.

## Troubleshooting (friction actually hit while testing)

- **Plain `mount-cat "$T" stream` prints nothing, forever.** Without `--follow`, `mount-cat` is collected — it prints only at EOF, and `stream` never EOFs while the guest lives. Use `mount-cat "$T" stream --follow` for the live view (it prints each line as it arrives), or bound a collected read with `timeout`. The `sh` REPL's `cat` streams too: `cat /n/room/stream` at the prompt printed a post live in this run, and at a real terminal Ctrl-C cancels it back to a fresh prompt (status 130) instead of wedging.
- **`who` keeps listing someone who left.** A hard-killed subscriber (process killed mid-read) holds its registry entry until the transport declares the connection dead — about 30 s of silence on loopback in this run. Presence is session state, not a heartbeat.
- **A parked `mount-cat "$T" stream --follow` outlives the serve dying — for ~33 s.** An already-blocked stream read carries no per-op deadline (ADR 0008), so when the serve is killed it keeps blocking until QUIC connection liveness declares the peer dead (33 s on loopback in this run), then fails with `resource unreachable: mesh: mesh wire transport error: quic recv read failed: connection lost`. Everything `--follow` printed before the death is already on your screen — only a still-buffered collected read loses bytes. Only a *fresh* op gets the friendly 5 s message above. Blocked stream readers are released with EOF only when the guest exits while the serve stays up.
- **First `app serve` seems to hang before printing the ticket.** Cold module cache: the QuickJS engine wasm is being compiled (~25 s in this debug-build run). Subsequent starts are ~1 s.
- **`app serve` refuses to start without `--addr`.** Default-deny, verbatim: "app serve on the public endpoint exposes the app to anyone with its ticket; pass --addr IP:PORT or --insecure-open to deliberately export to the open internet".
- **Reading `post` fails with `operation not supported`.** It is write-only by the guest's own rules, and the guidance after the colon is the guest's own `not_supported` message, carried across the wire (verbatim from this run: `mount-cat failed to read: operation not supported: post is write-only; read latest or roster instead`).
- **A nick is refused with `invalid`.** The guest validates: 32 chars max after trimming, no control characters — a bad nick fails the write and changes nothing (verbatim from this run: `mount-write failed: invalid argument: nick longer than 32 characters`; `roster` was untouched).
- **`resource unreachable: peer ... did not answer within 5s`.** The serve is down. The message says the recovery: the mount works again when the room returns, and the peer id half of the ticket never changes.

## Cleanup

Ctrl-C the serve. The room's history stays in `/tmp/room/log` and its nicks in `/tmp/room/nicks`, its identity in `~/.wanix/app-identities/chatroom.key`, your principal in `~/.wanix/dialer.key`, and the simulated friend in `/tmp/bob/.wanix/dialer.key` — delete `/tmp/room` and `/tmp/bob` when you are done with them.
