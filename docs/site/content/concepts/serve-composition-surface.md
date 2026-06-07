---
title: Serve as One Local Composition Surface
slug: concepts/serve-composition-surface
pageType: concept
oneLiner: wanix-rust serve combines static HTTP, the discovery doc, direct 9P over WebSocket, the qjs-shell WebSocket, and the HTTP-app route on one listener without making the 9P server own HTTP or browser policy.
audience: [developer]
tags: [serve, cli, shipped, caveat, single-frame-serve, loopback-only]
sourceRefs:
  - crates/wanix-cli/src/serve.rs:29-145
  - crates/wanix-cli/src/serve/command.rs:6-169
  - crates/wanix-cli/src/serve/connection.rs:42-80
  - crates/wanix-cli/src/serve/concurrent.rs:14-128
  - crates/wanix-cli/src/serve/http/routes.rs:14-159
  - crates/wanix-cli/src/serve/http/app.rs:22-66
  - docs/adrs/0005-serve-and-client-handoffs.md
seeAlso:
  - concepts/discovery-document
  - concepts/wanix-services-device-set
  - concepts/three-serve-bundles
  - concepts/single-frame-serve-caveat
  - concepts/http-app-route
prerequisites:
  - concepts/the-9p-contract
usedInFlows: []
honestLimits:
  - serve has no shutdown signal; the concurrent loop runs until the process is killed.
  - In normal (non-once) mode serve spawns a detached worker thread per connection with no cap.
  - Each connection handles one 9P frame at a time, so a blocking #plumb recv cannot interleave with a write on the same connection.
  - The HTTP-app route and qjs-shell WebSocket are loopback-only and require --wanix-services.
---

# Serve as One Local Composition Surface

`wanix-rust serve` combines static HTTP, the discovery doc, direct 9P over WebSocket, the qjs-shell WebSocket, and the HTTP-app route on one listener without making the 9P server own HTTP or browser policy.

A browser cockpit, a v86 VM, a QEMU launcher, and a plain `curl` all want different things from the same Wanix world: static assets, a route map, a binary 9P pipe, a terminal socket, an app endpoint. `serve` is the one local listener that multiplexes all of them off a single bound port, dispatching each request to the right adapter. The 9P server underneath never learns what HTTP is, what a bundle is, or which clients are trusted. That separation is the whole point: `serve` owns the composition, the runtime owns the files.

## Show it: one bind, many routes

Run the binary first. The flow's standard build step is `cargo build --package wanix-cli; alias wanix-rust='./target/debug/wanix-rust'`. Then:

```sh
wanix-rust serve --wanix-services
# wanix-rust serve: serving . files with Wanix overlay
# wanix-rust serve: listening on http://127.0.0.1:7654/
```

That single process now answers, on `127.0.0.1:7654`, every route below — and the dispatch order is exactly the `or_else` chain in `crates/wanix-cli/src/serve/http/routes.rs:20-25`:

```sh
curl http://127.0.0.1:7654/.well-known/wanix.json     # the discovery document
curl http://127.0.0.1:7654/.wanix/app/hello           # run apps/hello.js, return its stdout
# /.well-known/export9p   -> binary 9P, WebSocket-upgrade only
# /.well-known/qjs-shell  -> a terminal-backed qjs shell, WebSocket-upgrade only
# /?bundle=fs9p           -> a generated bundle page
```

One listener, five behaviours. Plan 9 called the file server and the connection muxer separate things; `serve` keeps that line. The connection handler peeks the request headers once, and forks on a single question — is this a WebSocket upgrade? (`crates/wanix-cli/src/serve/connection.rs:42-53`). Upgrades route to the 9P or terminal socket handlers; everything else is an HTTP request, resolved against the static root and the well-known/bundle/app/asset chain.

## The four transports, then serve

The 9P server is transport-agnostic, and the CLI exposes that fact directly. Three sibling subcommands export a Wanix filesystem over a bare transport, with no HTTP layered on (`crates/wanix-cli/src/collected.rs:21`, `process_io.rs:168-181`):

- `p9-stdio` — 9P over the process stdin/stdout pipe.
- `p9-listen` — 9P over a raw TCP socket.
- `p9-ws` — 9P over a bare WebSocket.

`serve` is the fourth and richest path: it does not *replace* those transports, it *composes* one of them (the 9P-over-WebSocket route at `/.well-known/export9p`) with HTTP, discovery, the terminal socket, bundles, and the app route. Same 9P server, same filesystem, more adapters in front of it.

## Flags: what serve takes

The parser is small and total (`crates/wanix-cli/src/serve/command.rs`). The defaults make the common case a single word:

- `--root <DIR>` or a bare positional argument — the directory to serve. Default `.` (`command.rs:162`).
- `--addr <ADDR>` or `--listen <ADDR>` — the bind address. Default `127.0.0.1:7654` (`command.rs:6,163`).
- `--bundle <NAME>` — select a generated client bundle page (`fs9p`, `workbench-fs9p`, `direct-v86`); see [three serve bundles](/concepts/three-serve-bundles).
- `--wanix-services` — export the `#`-device set and enable the services-gated routes; see [the wanix-services device set](/concepts/wanix-services-device-set).
- `--once` — serve exactly one connection, then exit. This is the deterministic test/scripting mode.

Each flag rejects duplicates with a usage error, so `serve --once --once` fails cleanly rather than silently winning last-write.

## One filesystem, many adapters

There is no per-route filesystem. `ServeRoots` is built once from the command (`crates/wanix-cli/src/serve.rs:79-84`), wrapped in an `Arc`, and every accepted connection clones that one handle (`concurrent.rs:115`). The discovery document, the binary 9P route, the qjs-shell, and the HTTP-app route all resolve against the *same* `p9_root` namespace. The HTTP-app route, for instance, allocates a `#task`, binds its stdout into the served tree, runs the program, and returns the captured bytes — all by walking the one shared filesystem over the in-process 9P contract (`crates/wanix-cli/src/serve/http/app.rs:104-117`). An adapter is a way *in*; it is not a separate world.

That is why two routes can gate differently without duplicating state. The qjs-shell WebSocket returns `not found` unless `--wanix-services` is set (`connection.rs:60-66`), and the HTTP-app route returns `403` to any non-loopback peer and `404` without services (`app.rs:55-66`) — policy that lives in the adapter, applied to the single underlying namespace. ADR 0005 fixes this as the contract: `serve` composes static HTTP, discovery, and direct protocol routes on one listener "without making the 9P server or core runtime own HTTP, WebSocket, browser isolation, or demo-page policy."

## Normal mode vs --once

Two execution shapes share all of the above (`crates/wanix-cli/src/serve.rs:110-122`):

- **`--once`** accepts a single connection on the listener, serves it, and returns its exit code — deterministic, used by tests and scripted clients.
- **Normal mode** sets the listener non-blocking and loops: accept a connection, spawn a detached worker thread to serve it, busy-wait `10ms` when there is nothing to accept (`concurrent.rs:14-35,83-87`). This is what accepts concurrent HTTP and 9P-WebSocket clients at once — a cockpit's browser and a v86 VM can both be attached to the same `serve` process.

## See also

- [The 9P contract](/concepts/the-9p-contract) — the protocol every transport here carries.
- [The discovery document](/concepts/discovery-document) — `/.well-known/wanix.json`, the route map clients read instead of hard-coding paths.
- [The --wanix-services device set](/concepts/wanix-services-device-set) — what `--wanix-services` exports and gates.
- [Three serve bundles](/concepts/three-serve-bundles) — `fs9p`, `workbench-fs9p`, `direct-v86`.
- [The HTTP-app route](/concepts/http-app-route) — `/.wanix/app/<name>`, the loopback-only app endpoint.
- [The single-frame serve caveat](/concepts/single-frame-serve-caveat) — why blocking reads and writes cannot interleave on one connection.

## Status / honest limits

`serve` is shipped and is the daily local composition surface, but its concurrency story is deliberately minimal:

- **No shutdown signal exists anywhere in `serve/`.** The normal-mode loop runs until the process is killed (`crates/wanix-cli/src/serve/concurrent.rs:28-32`). There is no graceful drain.
- **Normal mode spawns a detached worker thread per connection with no cap**, and busy-polls `accept()` on a fixed `10ms` sleep when idle (`concurrent.rs:85,117`). A connection cap needs the shutdown signal first, so the cap is unshipped and uncapped is the current default.
- **One 9P frame at a time per connection.** Because a connection's worker processes frames sequentially, a blocking read (e.g. `#plumb/<topic>/recv`) cannot interleave with a write on the same connection; live pub/sub needs a second connection or concurrent frame handling. This is the canonical [single-frame serve caveat](/concepts/single-frame-serve-caveat).
- **The services-gated routes are loopback-only.** The HTTP-app route (`/.wanix/app/<name>`) and the qjs-shell WebSocket both require `--wanix-services`, and the app route additionally returns `403` to any non-loopback peer (`crates/wanix-cli/src/serve/http/app.rs:55-66`). The exec devices behind them (`#task`, `#agent`, `#cpu`) are local-trust only and are not exposed to untrusted or public peers.
