---
title: Recipe 09 — The Web Door (named origins over one gateway)
slug: recipes/09-web-door-gateway
pageType: recipe
oneLiner: Bind the chat webapp and its mesh room under http://chat.localhost:PORT and a static docs site under a second name, drive posts/latest/SSE with curl, watch an outage map to 503, and read the one identity caveat before demoing.
audience: [newcomer, developer]
tags: [web, apps, mesh, appfs, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/serve/webdoor.rs
  - crates/wanix-cli/src/serve/webdoor/tests.rs
  - examples/chatroom/web/index.html
  - examples/chatroom/web/app.js
  - docs/appfs.md
seeAlso:
  - learn/publish-apps-via-web-door
  - recipes/07-chatroom-over-the-mesh
  - concepts/guest-defined-resources
  - concepts/key-is-the-address
  - concepts/blocking-stream-eof-contract
prerequisites:
  - recipes/07-chatroom-over-the-mesh
usedInFlows:
  - {flow: publish-apps-via-web-door, step: 2}
honestLimits:
  - "v0 gateway principals: the gateway dials every mounted room with ITS dialer key (~/.wanix/dialer.key), so the room sees one principal for all web users — every browser post is the gateway's (shorthex), and a web-set nick names that shared principal. Per-user web identity needs delegation certs / gateway principals (docs/appfs.md §Identity And Trust)."
  - "Loopback-only in v0: a non-loopback --listen with --bind is refused at startup; off-loopback gateway auth is recorded follow-up work."
  - "Mesh binds are dialed at serve startup and are hard: an offline room fails serve startup. After startup a dead room maps to 503; the first op right after a hard kill can take up to QUIC liveness (~30 s) before the fast ~5 s deadline path resumes (ADR 0008 residual)."
  - "Each streaming GET holds one gateway thread for the life of the connection (serve has no connection cap — known queued follow-up). An abandoned subscriber (closed tab, EventSource reconnect) is detected by a socket probe even on a quiet stream and its thread/upstream mount stream released."
canonicalCaveatFor: []
---

# Recipe 09 — The Web Door (named origins over one gateway)

Bind the chat webapp and its mesh room under `http://chat.localhost:PORT` and a static docs site under a second name, drive posts/latest/SSE with curl, watch an outage map to 503, and read the one identity caveat before demoing.

**What & why.** A browser has no iroh key, so the `serve` HTTP door can act as a gateway into namespaces: `--bind NAME=SOURCE` (repeatable) composes, per NAME, one namespace from every source bound to it — static directories and dialed mesh mounts unioned at the root — and serves it at `http://NAME.localhost:PORT` by `Host`-header routing. `*.localhost` resolves to loopback by resolver convention, so there is zero DNS setup. The mapping is generic HTTP→filesystem, not chat-specific. Everything below ran verbatim on one machine over loopback. The conceptual walkthrough is [Publish apps via the web door](/learn/publish-apps-via-web-door); the room being mounted is [Recipe 07](/recipes/07-chatroom-over-the-mesh)'s.

## 0. Prerequisite: a running room

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust app serve --app examples/chatroom --state /tmp/room --addr 127.0.0.1:0 --restart on-failure
# chatroom	iroh://71b44428...?addr=127.0.0.1:46350
```

## 1. One gateway, two named origins

Bind the bundled web client *and* the room under the name `chat` (they share one origin, so the page fetches the room's files with no CORS), and a plain directory under a second name. A NAME without a dot gets `.localhost` appended; repeated `--bind` for one NAME unions at the origin root, earliest bind winning path conflicts. No `--wanix-services` needed.

```sh
mkdir -p /tmp/empty
T='iroh://71b44428...?addr=127.0.0.1:46350'    # the room ticket
wanix-rust serve --root /tmp/empty \
  --bind chat=examples/chatroom/web \
  --bind "chat=$T" \
  --bind docs=docs/site/content \
  --listen 127.0.0.1:7699
```

```text
wanix-rust serve: gateway origin http://chat.localhost:7699/
wanix-rust serve: gateway origin http://docs.localhost:7699/
wanix-rust serve: serving /tmp/empty files with Wanix overlay
wanix-rust serve: listening on http://127.0.0.1:7699/
```

If the room registered itself in the catalog (`app serve ... --register room`, [Recipe 10](/recipes/10-name-your-world)), the ticket line collapses to `--bind chat=room` — executed: the serve printed `wanix-rust: name 'room' -> iroh://e1de10e6...?addr=... (resolved through the catalog at launch)` and the `POST /post` → `GET /latest` round trip below worked unchanged. `--bind` is the one *fallback* context: a name-spelled source with no catalog entry is treated as a relative directory (spell `./room` to force the directory when an entry exists).

## 2. The bare host is the name index

```sh
curl http://127.0.0.1:7699/
```

```text
<!doctype html><title>wanix gateway</title><h1>bound names</h1><ul><li><a href="http://chat.localhost:7699/">chat.localhost</a></li><li><a href="http://docs.localhost:7699/">docs.localhost</a></li></ul>
```

## 3. The room's files are plain same-origin URLs

Open `http://chat.localhost:7699/` in a browser for the webapp (post form, live EventSource feed, nick form) — or drive the same files with curl. GET of a sized file returns it whole; POST writes the body:

```sh
curl -X POST -d 'hello from curl' http://chat.localhost:7699/post
# ok
curl http://chat.localhost:7699/latest | tail -1
# {"at":1781081250165,"from":"ada (36469a42)","body":"hello from curl"}
```

Look at that `from` — it is the *gateway's* principal (in this run, the same dialer key that claimed `ada` in recipe 07), not curl's and not a browser user's. That is the v0 identity story; read the caveat in step 7 before demoing.

Where `*.localhost` doesn't resolve (some containers, odd resolvers), the routing is literally the `Host` header — set it yourself:

```sh
curl -H 'Host: chat.localhost' http://127.0.0.1:7699/latest | tail -1
# {"at":1781081250165,"from":"ada (36469a42)","body":"hello from curl"}
```

## 4. The live feed: chunked and SSE off one stream file

GET of a zero-length device file streams as chunked transfer — and as SSE `data:` lines when the client asks (`Accept: text/event-stream`), which is what the webapp's `EventSource` does. Park both consumers, then post:

```sh
curl -N http://chat.localhost:7699/stream &
curl -N -H 'Accept: text/event-stream' http://chat.localhost:7699/stream &
curl -X POST -d 'browsers get this line as it happens' http://chat.localhost:7699/post
```

Both readers print within a beat of the POST returning — first raw, then SSE-framed:

```text
{"at":1781081267713,"from":"iroh:36469a4270682126...","body":"browsers get this line as it happens"}
data: {"at":1781081267713,"from":"iroh:36469a4270682126...","body":"browsers get this line as it happens"}
```

The stream response headers, verbatim:

```text
HTTP/1.1 200 OK
Content-Type: text/plain; charset=utf-8
Transfer-Encoding: chunked
Cache-Control: no-store
Connection: close
```

No `Access-Control-Allow-Origin` anywhere on a gateway origin — that is the
point of the one-origin design: the page and its data already share an origin,
and a CORS grant would hand bound files and live streams to any other web page
in your browser. The same boundary holds for writes: a POST/PUT whose `Origin`
header names another origin is refused with 403 (`cross-site write refused`),
while Origin-less tools like curl pass.

## 5. The second name is just a static site

Any directory bound to a name is an origin: GET of a directory serves its `index.html` or a JSON listing, and files go out by content type.

```sh
curl http://docs.localhost:7699/
# [{"name":"concepts","type":"dir","size":3406},{"name":"devices","type":"dir","size":124},...
curl http://docs.localhost:7699/recipes/index.md | head -3
# ---
# title: Recipes
# slug: recipes/index
```

## 6. Errors map honestly: an outage is a 503, never a 404

Kill the room's `app serve` and ask again:

```sh
curl -i http://chat.localhost:7699/latest
```

```text
HTTP/1.1 503 Service Unavailable
Content-Length: 105
Content-Type: text/plain; charset=utf-8
Retry-After: 5

resource unreachable: mesh: mesh wire transport error: mesh stream operation exceeded its per-op deadline
```

Restart the room — it comes back on a *new* port — and the gateway recovers with **no re-bind**: the bind's peer half is the address, the stale hint is just a hint, and the next `GET /latest` returned the full history in this run. The gateway is loopback-only in v0; binding off-loopback is refused at startup, verbatim: "serve --bind exposes namespace reads AND writes (directories, mesh mounts) over unauthenticated HTTP; it is refused because the HTTP door is bound to a non-loopback address (the ADR 0006 trust rule — off-loopback gateway auth is recorded follow-up work). Bind to loopback (e.g. 127.0.0.1:PORT) or drop --bind".

## 7. v0 gateway principals — read this before demoing identity

The gateway dials the room with **its** dialer key (`~/.wanix/dialer.key`), so the room sees one principal for ALL web users: every browser post lands as the same `(shorthex)` — the gateway's — and a nick set through the web names the gateway principal, shared by everyone on that gateway. Session isolation, if any, is gateway/HTTP-layer state only, never mesh principal enforcement, and the webapp deliberately does not fake per-user attribution: it shows messages exactly as the room records them and only locally tags posts it sent itself. The mesh wire binds one principal per connection, and that connection is dialed by the gateway, not the browser; the future per-user path is delegation certs / gateway principals threaded into the attach (`docs/appfs.md` §Identity And Trust), out of scope here.

## Troubleshooting (friction actually hit while testing)

- **`http://chat.localhost:7699/` doesn't resolve.** Resolver-dependent; the routing only needs the `Host` header — `curl -H 'Host: chat.localhost' http://127.0.0.1:7699/...` always works (step 3).
- **The first request right after hard-killing the room hangs ~30 s before the 503.** The gateway's already-open connection has to be declared dead by QUIC liveness first; afterwards fresh ops fail within the ~5 s mesh deadline (ADR 0008 residual).
- **An offline room fails the gateway at startup.** Mesh binds are dialed when `serve` starts and are hard mounts — start the room first, or drop its bind.
- **A genuinely empty sized file streams an instantly-finished chunked body.** Device-file detection is metadata `len == 0`; harmless, but know it's the heuristic.
- **Bound-host requests bypass `/.well-known` routes.** A named origin owns its whole path space; discovery routes live on the bare host only. Gateway requests also carry no principal into local service devices.

## Cleanup

Ctrl-C the gateway and the room serve. Nothing else persists beyond recipe 07's state (`/tmp/room`, identities under `~/.wanix/`); remove `/tmp/empty` if you made it for this.
