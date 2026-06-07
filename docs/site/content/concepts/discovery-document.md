---
title: Discovery Document (/.well-known/wanix.json)
slug: concepts/discovery-document
pageType: concept
oneLiner: A single JSON contract advertising the p9, rootfs, qjsShell, httpApp, and ethernet routes, the v86 block, services, and selected bundle, so clients consume it instead of hard-coding routes.
audience: [developer]
tags: [serve, discovery, shipped, caveat, cli]
sourceRefs:
  - crates/wanix-cli/src/serve/discovery.rs:35-91
  - crates/wanix-cli/src/serve/discovery.rs:154-188
  - crates/wanix-cli/src/serve/http/app.rs:35-47
  - crates/wanix-cli/src/serve/roots.rs:119-120
  - docs/adrs/0005-serve-and-client-handoffs.md:25-42
seeAlso:
  - concepts/serve-composition-surface
  - concepts/wanix-services-device-set
  - concepts/three-serve-bundles
  - concepts/loopback-only-handoffs
  - concepts/browser-cockpit
prerequisites:
  - concepts/serve-composition-surface
usedInFlows: []
honestLimits:
  - The discovery JSON is still hand-built with format! string interpolation; converting it to typed structs with shape-pinning tests is a queued cleanup.
  - The ethernet route is advertised as status not-implemented; there is no Ethernet/vnet bridge behind it yet.
  - qjsShell, httpApp, and the services block report status disabled (or null) unless serve runs with --wanix-services.
  - The httpApp route and the rootfs.json handoff are loopback-only; non-loopback clients see local-only or disabled.
---

# Discovery Document (/.well-known/wanix.json)

A single JSON contract advertising the p9, rootfs, qjsShell, httpApp, and ethernet routes, the v86 block, services, and selected bundle, so clients consume it instead of hard-coding routes.

A browser cockpit, a v86 page, a filesystem client — none of them should ship with `ws://host/.well-known/export9p` baked into their source. They should *ask* the server what it offers and adapt. That is the whole job of the discovery document: one well-known URL, one JSON object, every route and capability the running `serve` actually exposes. Hit it once, read the fields you need, and you never have to guess a path or a protocol string again.

## Show it first

Start a services-enabled serve and curl the well-known document:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust serve --wanix-services --bundle workbench-fs9p &

curl -s http://localhost:7654/.well-known/wanix.json | jq .routes.p9
```

```json
{
  "websocket": "ws://localhost:7654/.well-known/export9p",
  "transport": "direct-binary-websocket",
  "protocol": "9p2000.L",
  "supportedProtocols": ["9P2000.L", "9P2000.L.Google.2"]
}
```

That object is produced by `serve_discovery_json` (`crates/wanix-cli/src/serve/discovery.rs:35-91`). The reserved URL is `/.well-known/wanix.json`; ADR 0005 names it "the reserved discovery document for routes, selected bundle metadata, service availability, boot hints, and explicit placeholders for unimplemented routes such as Ethernet/vnet" (`docs/adrs/0005-serve-and-client-handoffs.md:25-42`). The Plan 9 lineage here is the *service registry*: a client attaches once to a known name and discovers everything else from inside it.

## The routes block

`routes` is the heart of the document. Every URL is built from the request's `Host` header so the document advertises itself with the same address the client used to reach it (`discovery.rs:40-45`):

- **`routes.p9`** — the direct binary 9P WebSocket at `/.well-known/export9p`, with `transport: "direct-binary-websocket"`, `protocol: "9p2000.L"`, and a `supportedProtocols` list that includes `9P2000.L.Google.2` so a client can negotiate `walkgetattr` when it is available. This is the route the cockpit and filesystem clients dial to browse `wanix:/`.
- **`routes.rootfs`** — a `wanix-rootfs.v1` handoff pointer at `/.well-known/rootfs.json`. Its `status` field is computed per request: `available`, `unprepared` (boot markers missing), `invalid` (manifest error), or `local-only` for a non-loopback peer (`discovery.rs:114-152`).
- **`routes.qjsShell`** — the raw-bytes qjs shell WebSocket, protocol `wanix-qjs-shell.v1`, carrying its own `cwdQuery`, `resize` formats, `exitMessage`, and `terminalLifecycle` contract so a terminal client wires itself without reading the source (`discovery.rs:173-188`). It reports `{"status":"disabled"}` unless `--wanix-services` is set.
- **`routes.httpApp`** — the HTTP-app route, protocol `wanix-http-app.v1`, with `route: "/.wanix/app/<name>"`, `source: "apps/<name>.js|apps/<name>.wasm"`, `method: "GET"`, and `scope: "loopback"` (`crates/wanix-cli/src/serve/http/app.rs:35-47`). It is `{"status":"disabled"}` without services.
- **`routes.ethernet`** — a placeholder: `{"websocket": "ws://host/.well-known/ethernet", "status": "not-implemented"}` (`discovery.rs:45,65`). The URL exists; nothing answers it yet. Advertising the not-yet-implemented route honestly is the point — a client reads `status` and skips it.

## The v86 block, services, and bundle

Three more top-level keys round out the document.

**`v86`** carries the emulator boot story: an `assets` map (module, mod, offscreen, wasm, bios, vgaBios paths), a `boot` object derived from the static root, and tuning knobs — `defaultCmdline`, `p9Msize`, `memorySize`, `vgaMemorySize`, and `virtioConsole: true` (`discovery.rs:67-69`). A direct-v86 page reads these instead of hard-coding asset paths or a kernel command line.

**`services`** is `null` unless serve runs with `--wanix-services`. When enabled it reports `{"task": "#task", "term": "#term", "drivers": [...], "devices": [...]}` (`discovery.rs:154-171`). The `drivers` list is derived from the task registry, not hand-typed, so it cannot drift from what `#task/new/<kind>` will actually launch — today `["auto","noop","qjs","wasm"]`. The `devices` list is the single `INSPECTABLE_SERVICE_DEVICES` source: `#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent` (`crates/wanix-cli/src/serve/roots.rs:119-120`). The cockpit reads `services.devices` to know which devices to inspect; see [the wanix-services device set](/concepts/wanix-services-device-set).

**`bundle`** is the selected bundle name (`workbench-fs9p`, `direct-v86`, `fs9p`) or `null` when no bundle was requested (`discovery.rs:46-50`). A client uses it to pick the right frontend behavior; see [the three serve bundles](/concepts/three-serve-bundles).

## The other well-known routes

Discovery names them, but they are separate endpoints with their own access rules:

- **`/.well-known/export9p`** — the raw 9P WebSocket itself, not JSON. Discovery only advertises its URL.
- **`/.well-known/rootfs.json`** — the actual `wanix-rootfs.v1` handoff, returned only to loopback clients because the manifest embeds absolute local paths and launch argv (`discovery.rs:93-112`). A non-loopback peer gets `403 Forbidden`. See [loopback-only handoffs](/concepts/loopback-only-handoffs).

The `/.wanix/app/<name>` HTTP-app route is likewise loopback-and-services gated: it runs a `qjs`/`wasm` task and returns its stdout (`http/app.rs:49-66`). Discovery advertises it under `routes.httpApp`; the route enforces the boundary.

## Why clients consume discovery

ADR 0005 is explicit: "Generated bundle pages and browser clients should consume the discovery document rather than hard-coding route assumptions" (`docs/adrs/0005-serve-and-client-handoffs.md`). The payoff is decoupling. The host can be a name, an IPv6 literal, or a forwarded port; ports and asset paths can change between builds; a route can be present-but-disabled. A client that reads `status` fields and built URLs adapts to all of that without a code change. The cockpit bootstrap fetches `/.well-known/wanix.json`, reads `routes.p9.websocket`, negotiates the best `supportedProtocols` entry, and only inspects the devices listed in `services.devices` — it learns the server's shape at runtime. See [the browser cockpit](/concepts/browser-cockpit).

## See also

- [Serve composition surface](/concepts/serve-composition-surface) — the one listener that combines static HTTP, 9P, and these routes.
- [The wanix-services device set](/concepts/wanix-services-device-set) — what `services.devices` enumerates and why it is a single source.
- [The three serve bundles](/concepts/three-serve-bundles) — how `bundle` selects a frontend.
- [Loopback-only handoffs](/concepts/loopback-only-handoffs) — why rootfs.json and the HTTP-app route are loopback-gated.
- [The browser cockpit](/concepts/browser-cockpit) — the operator surface that consumes this document.

## Status / honest limits

- **Hand-built JSON.** `serve_discovery_json` interpolates the document with `format!` strings, not typed structs. Converting discovery, rootfs, qemu, and direct-v86 handoffs to typed structs with shape-pinning tests is a queued cleanup; until then the shape is pinned only by the `serve_*` tests in `crates/wanix-cli/src/serve/tests.rs`.
- **Ethernet is a placeholder.** `routes.ethernet` always reports `status: "not-implemented"`; there is no Ethernet/vnet bridge behind the advertised URL.
- **Service-gated routes report disabled.** Without `--wanix-services`, `qjsShell`, `httpApp`, and the `services` block are `{"status":"disabled"}` or `null`. The route fields exist regardless so a client can branch on `status`.
- **Loopback boundary holds at the route, not the document.** Discovery will advertise `httpApp` and `rootfs` to any client, but those endpoints serve only loopback peers (and the HTTP-app route requires services). The document is a catalog, not an authorization.
