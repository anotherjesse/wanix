---
title: Publish apps via the web door
slug: learn/publish-apps-via-web-door
pageType: flow
oneLiner: Give a mesh-served app a browser audience — one gateway, named origins, zero DNS.
audience: [newcomer, developer]
tags: [web, apps, mesh, appfs, shipped, caveat]
sourceRefs:
  - crates/wanix-cli/src/serve/webdoor.rs
  - crates/wanix-cli/src/serve/webdoor/tests.rs
  - examples/chatroom/web/app.js
  - docs/appfs.md
  - docs/site/content/recipes/09-web-door-gateway.md
seeAlso:
  - recipes/09-web-door-gateway
  - learn/build-a-chatroom
  - concepts/guest-defined-resources
  - concepts/everything-is-a-file
  - concepts/blocking-stream-eof-contract
prerequisites:
  - learn/build-a-chatroom
usedInFlows: []
honestLimits:
  - "The gateway is ONE principal to every mounted room (its ~/.wanix/dialer.key); per-user web identity is delegation-cert / gateway-principal follow-up (docs/appfs.md §Identity And Trust)."
  - "Loopback-only in v0: --bind with a non-loopback --listen is refused at startup."
  - "Each streaming GET pins one gateway thread; serve has no connection cap yet."
canonicalCaveatFor: []
---

# Publish apps via the web door

Give a mesh-served app a browser audience — one gateway, named origins, zero DNS.

The [chatroom flow](/learn/build-a-chatroom) ends with a working room that only iroh-key holders can reach. This flow opens it to a browser, which has no key — and shows that the opening is *generic*: not a chat feature, but one HTTP→namespace mapping any bound name gets for free. Fifteen minutes, one machine, curl as the stand-in browser. The tested transcript is [Recipe 09](/recipes/09-web-door-gateway).

## 1. The idea: names are origins, origins are namespaces

```sh
wanix-rust serve --root /tmp/empty \
  --bind chat=examples/chatroom/web \
  --bind "chat=$T" \
  --bind docs=docs/site/content \
  --listen 127.0.0.1:7699
```

`--bind NAME=SOURCE` (repeatable) composes, per NAME, one namespace from every source bound to it — static directories, dialed `iroh://` mesh mounts, and catalog names (a registered room makes the ticket line just `--bind chat=room`, resolved at launch — executed in [Recipe 09](/recipes/09-web-door-gateway)) unioned at the root — and serves it at `http://NAME.localhost:PORT` by `Host`-header routing. `*.localhost` resolves to loopback by resolver convention, so there is nothing to configure; where it doesn't, `curl -H 'Host: chat.localhost'` is the same thing said explicitly. The bare host serves an index of bound names.

Binding the webapp's static files *and* the room's mesh ticket under one name is the move that matters: page and data share one origin, so the browser fetches `post`, `latest`, and `stream` with no CORS ceremony — the web client is static files plus `fetch()` against its own origin.

## 2. One mapping, every file shape

The gateway translates HTTP onto the one `FileSystem` contract ([everything is a file](/concepts/everything-is-a-file)):

- **GET of a sized file** returns it whole, by content type.
- **GET of a directory** serves its `index.html`, else a JSON listing — so `docs.localhost` above is a complete static site with zero site-generator anything.
- **POST/PUT** writes the body — `curl -X POST -d 'hi' .../post` is the room's post file.
- **GET of a zero-length device file** streams: chunked transfer for plain clients, SSE `data:` lines under `Accept: text/event-stream` — a browser `EventSource` follows the room's never-EOF `stream` live ([the stream contract](/concepts/blocking-stream-eof-contract), now ending in a `<ul>` on a web page).
- **Errors map honestly**: kill the room and `GET /latest` is `503 Service Unavailable` + `Retry-After: 5` — an outage, never a 404. Restart the room (even on a new port) and the gateway recovers without a re-bind, because the bind names the *peer*, not the route.

Nothing in that list mentions chat. The chatroom is just the demo app; any app whose surface is files gets a web door for free.

## 3. The honest boundary: one key at the door

The gateway dials every mounted room with **its own** dialer key. To the room, all web users are one principal: the gateway's `(shorthex)`, one shared nick, no per-user attribution — and the bundled webapp refuses to fake it (it renders messages exactly as the room records them, only locally tagging its own posts). The room's attribution machinery is not weakened — it is doing exactly its job, telling you truthfully that *the gateway* posted. What's missing is a way for the gateway to speak *for* a browser user, and that is named follow-up work: delegation certs / per-user gateway principals threaded into the attach (`docs/appfs.md` §Identity And Trust). Until then the door is also loopback-only by refusal — an unauthenticated read-write HTTP door does not belong off-loopback (the ADR 0006 rule).

## Where this goes

Run the whole thing — both origins, the SSE feed, the 503, the refusal text — in [Recipe 09](/recipes/09-web-door-gateway). The room behind it is [build a chatroom](/learn/build-a-chatroom); the design note carrying the identity follow-up is `docs/appfs.md`.
