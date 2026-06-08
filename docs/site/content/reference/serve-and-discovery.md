---
title: Serve, Discovery, and Handoff JSON
slug: reference/serve-and-discovery
pageType: developer
oneLiner: The serve command surface, the discovery document shape, the well-known routes, and the rootfs/qemu/v86 handoff JSON contracts.
audience: [developer]
tags: [serve, discovery, cli, shipped, loopback-only, caveat]
sourceRefs:
  - crates/wanix-cli/src/serve/command.rs:6-18
  - crates/wanix-cli/src/serve/command.rs:144-169
  - crates/wanix-cli/src/serve/discovery.rs:35-91
  - crates/wanix-cli/src/serve/discovery.rs:93-152
  - crates/wanix-cli/src/serve/discovery.rs:154-188
  - crates/wanix-cli/src/serve/roots.rs:119-172
  - crates/wanix-cli/src/serve/http/routes.rs:96-144
  - crates/wanix-cli/src/serve/http/app.rs:35-66
  - crates/wanix-cli/src/rootfs/handoff.rs:32-58
  - crates/wanix-cli/src/qemu/json.rs:5-25
seeAlso:
  - concepts/serve-composition-surface
  - concepts/discovery-document
  - concepts/wanix-services-device-set
  - concepts/loopback-only-handoffs
  - concepts/three-serve-bundles
  - reference/cli-rootfs-qemu-v86
  - concepts/single-frame-serve-caveat
prerequisites:
  - concepts/serve-composition-surface
  - concepts/discovery-document
usedInFlows: []
honestLimits:
  - Discovery and the rootfs/qemu/v86 handoffs are still hand-built format! strings, not typed structs; a typed-struct + shape-pinning-test cleanup is queued.
  - The /.wanix/app/<name>, /.well-known/rootfs.json, and POST /agent routes are loopback-only and (for app/agent) require --wanix-services.
  - The served #agent on every serve path is a deterministic FakeEngine, never a live LLM.
  - serve handles one 9P frame at a time per connection, so a blocking #plumb recv cannot interleave with a write on the same connection.
  - The ethernet websocket route is advertised but returns not-implemented.
---

# Serve, Discovery, and Handoff JSON

The serve command surface, the discovery document shape, the well-known routes, and the rootfs/qemu/v86 handoff JSON contracts.

`wanix-rust serve` is the one HTTP front door for the Rust runtime: it exports a Wanix namespace as 9P, optionally composes the service devices onto it, and publishes a machine-readable description of itself at `/.well-known/wanix.json` so browsers, the cockpit, direct-v86, and VM launchers can discover the routes instead of hard-coding them. This page is the reference for the flags, the discovery JSON, the well-known routes, and the two handoff document kinds. Every claim cites the file that produces it.

## serve flags and modes

The command parser accepts a positional root directory plus a handful of options (`crates/wanix-cli/src/serve/command.rs:6-18`):

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust serve ./root --bundle workbench-fs9p --wanix-services
```

- **root** (positional, or `--root DIR`) — the directory exported as the served filesystem. Defaults to `.` (`command.rs:160-168`).
- **`--addr` / `--listen ADDR`** — the bind address. Default is `127.0.0.1:7654` (`command.rs:6`, `DEFAULT_SERVE_ADDR`). The default is loopback, which is what makes the loopback-only routes below the common case.
- **`--bundle NAME`** — selects a browser bundle page: `fs9p`, `workbench-fs9p`, or `direct-v86` (the three bundle names live in `serve.rs:26-27` and `direct_v86.rs:10`). See [three serve bundles](/concepts/three-serve-bundles).
- **`--wanix-services`** — binds the service-device set onto the namespace and flips the services-gated routes from `disabled` to `available`.
- **`--once`** — accept exactly one connection then exit, for scripted tests (`command.rs:144-150`).

- **`--p9 HOST:PORT [--peer HEX --grant ANAME:PREFIX:RIGHTS ...]`** — binds a raw 9P listener over TCP alongside HTTP (default loopback), exporting the same served namespace as the websocket door over one per-connection session core. Capability-gated by `--peer`/`--grant`. Refused with `--wanix-services` off-loopback (ADR 0006).
- **`--listen HOST:PORT`** — the HTTP/websocket listener address (`--addr` is the deprecated synonym).

Each flag is single-shot; passing it twice is a usage error (`command.rs:127-158`). `p9-stdio` remains a separate subcommand for raw 9P over the process pipe (the QEMU-v86 bridge); raw 9P over TCP (`--p9`) and the binary 9P websocket (`/.well-known/export9p`) are both `serve` doors over one session core. The retired `p9-listen`/`p9-ws` subcommands folded into `serve`.

## The discovery document: `/.well-known/wanix.json`

A GET to `/.well-known/wanix.json` returns the discovery JSON built in `serve_discovery_json` (`discovery.rs:35-91`). The host in every advertised URL is taken from the request `Host` header (falling back to the local addr), so the document is correct behind a proxy. The top-level shape is:

```json
{
  "version": 1,
  "runtime": "wanix-rust",
  "routes": {
    "p9":      { "websocket": "ws://HOST/.well-known/export9p",
                 "transport": "direct-binary-websocket",
                 "protocol": "9p2000.L",
                 "supportedProtocols": ["9P2000.L", "9P2000.L.Google.2"] },
    "rootfs":  { "url": "...", "kind": "wanix-rootfs.v1", "status": "...", "ready": false },
    "qjsShell":{ "status": "disabled" | "available", ... },
    "httpApp": { "status": "disabled" | "available", ... },
    "ethernet":{ "websocket": "ws://HOST/.well-known/ethernet", "status": "not-implemented" }
  },
  "v86": { "assets": {...}, "boot": {...}, "defaultCmdline": "...", "p9Msize": N, ... },
  "services": null | { "task": "#task", "term": "#term", "drivers": [...], "devices": [...] },
  "bundle": null | "fs9p" | "workbench-fs9p" | "direct-v86"
}
```

The `p9` route is the always-present anchor: the WebSocket export and the two negotiable protocol levels (base 9P2000.L and the Google.2 `walkgetattr` extension). The `v86` block carries the direct-v86 asset paths, boot hints, and 9P `msize`/memory knobs regardless of bundle. `ethernet` is advertised but its handler returns `501 not-implemented` (`routes.rs:150-155`) — it is a placeholder, not a working bridge.

`qjsShell` and `httpApp` are **services-gated**: with `--wanix-services` they report `"status":"available"` and the route details; without it they report `"status":"disabled"` (`discovery.rs:173-188`, `http/app.rs:35-46`). The `httpApp` route advertises `"route":"/.wanix/app/<name>"`, `"source":"apps/<name>.js|apps/<name>.wasm"`, and `"scope":"loopback"` — the HTTP-app surface, served from a `.js`/`.wasm` program in the namespace and reachable only from loopback.

## The well-known routes

The route table is matched in `routes.rs:96-144`:

- **`/.well-known/wanix.json`** — discovery (above), open to any client.
- **`/.well-known/export9p`** — the 9P-over-WebSocket export. A plain GET returns `400 websocket upgrade required`; real clients send an Upgrade.
- **`/.well-known/qjs-shell`** — the raw-bytes qjs shell WebSocket, only routed when `--wanix-services` is set (`routes.rs:130-143`).
- **`/.well-known/rootfs.json`** — the rootfs handoff, **loopback-only** (`discovery.rs:93-99`); a non-loopback peer gets `403`.
- **`/.well-known/ethernet`** — reserved, `501`.

Two more routes live outside `.well-known`: `/.wanix/app/<name>` (the HTTP-app route, services-gated and loopback-only — `http/app.rs:49-66`) and `POST /agent` (the agent-as-service endpoint, loopback-gated — `http.rs:92-97`). Both are loopback by design because they execute tasks. See [loopback-only handoffs](/concepts/loopback-only-handoffs).

## The `--wanix-services` bind set

With `--wanix-services`, serve binds a fixed device set onto the namespace and advertises it under `services.devices`. The single source of truth is `INSPECTABLE_SERVICE_DEVICES` (`roots.rs:119-120`):

```rust
pub(super) const INSPECTABLE_SERVICE_DEVICES: &[&str] =
    &["#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent"];
```

That constant is exactly what the cockpit lists and what the `serve_wanix_services_*` tests probe over 9P, so the advertised set can't drift from the bound set. The matching binds are in `bind_host_and_terminal` (`roots.rs:122-172`): `#term`, `#pipe`, `#kv`, `#plumb` (a single-node `LocalPlumbPort`), `#cas` (the owner-private on-disk store), and `#agent`. `services.drivers` is derived from the task registry (`discovery.rs:154-167`), so it lists exactly what `#task/new/<kind>` can launch — today `["auto","noop","qjs","wasm"]`. See [the --wanix-services device set](/concepts/wanix-services-device-set).

Note the served `#agent` is bound to a deterministic `FakeEngine` (`roots.rs:163-170`): the real codex bridge is local-trust only and stays on the `wanix agent` CLI path. Discovery never advertises a live model. And `#kv` here is in-memory — its state lives only for the serve process lifetime; freeze a [capsule](/concepts/wanix-capsule) to persist.

## Handoff kinds

Two JSON document kinds describe how to boot a guest from a prepared root. They are produced by the `rootfs` and `qemu` subcommands and re-served at `/.well-known/rootfs.json`.

**`wanix-rootfs.v1`** (`rootfs/handoff.rs:32-58`) — describes a prepared guest root: `rootPath`, `kernelRoute`/`kernelPath`, `initRoute`/`initPath`, an embedded `qemu` handoff, and a `serveDirectV86` block (`argv`, `bundle: "direct-v86"`, `wanixServices: true`, `p9Msize`). The discovery `routes.rs` rootfs entry reports `status` as `available`, `unprepared`, `invalid`, or `local-only` depending on prepared-root readiness and whether the peer is loopback (`discovery.rs:114-152`).

**`wanix-qemu-virtio9p.v1`** (`qemu/json.rs:5-25`) — the QEMU virtio-9p launch contract: `qemuBin`, the full `argv`, `rootPath`, `kernelPath`, `initrdPath`, `cmdline`, `memoryMb`, `kvm`, `mountTag`, `securityModel`, `p9Msize`, `console: "hvc0"`, `rootFilesystem: "9p"`. The CLI surface that emits these is documented in [CLI: rootfs / qemu / v86](/reference/cli-rootfs-qemu-v86).

## See also

- [The serve composition surface](/concepts/serve-composition-surface)
- [The discovery document](/concepts/discovery-document)
- [The --wanix-services device set](/concepts/wanix-services-device-set)
- [Loopback-only handoffs](/concepts/loopback-only-handoffs)
- [Three serve bundles](/concepts/three-serve-bundles)
- [The HTTP-app route](/concepts/http-app-route)
- [CLI: rootfs / qemu / v86](/reference/cli-rootfs-qemu-v86)
- [The single-frame serve caveat](/concepts/single-frame-serve-caveat)

## Status / honest limits

- **Discovery and handoffs are hand-built `format!` strings, not typed structs.** The driver-list drift is already fixed (drivers derive from the registry), but the remaining fragments in `discovery.rs`, `rootfs/handoff.rs`, and `qemu/json.rs` are string templates; converting them to typed structs with shape-pinning tests is a queued cleanup ([queued follow-ups](/reference/queued-follow-ups)).
- **The execution and handoff routes are loopback-only.** `/.wanix/app/<name>` and `POST /agent` require loopback and `--wanix-services`; `/.well-known/rootfs.json` requires loopback. They are not exposed to untrusted peers.
- **The served `#agent` is a deterministic `FakeEngine`,** not a live LLM; the real codex engine is the local-trust `wanix agent` CLI path only.
- **`#kv` is in-memory** for the serve process lifetime; persist via a capsule.
- **serve handles one 9P frame at a time per connection,** so a blocking `#plumb` recv cannot interleave with a write on the same connection — use a second connection for live pub/sub.
- **The `ethernet` route is advertised but returns `not-implemented`.**
