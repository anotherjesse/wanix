---
title: Loopback-Only VM and App Handoffs
slug: concepts/loopback-only-handoffs
pageType: concept
oneLiner: rootfs.json, the HTTP-app route, and direct-v86 are gated to loopback clients and prepared-root boot markers — launch contracts, not a VM supervisor.
audience: [developer]
tags: [serve, cli, caveat, local-trust-only, shipped, trust-boundary]
sourceRefs:
  - crates/wanix-cli/src/serve/discovery.rs:93-152
  - crates/wanix-cli/src/serve/discovery/host.rs:40-42
  - crates/wanix-cli/src/serve/http/app.rs:55-66
  - crates/wanix-cli/src/rootfs/handoff.rs:32-58
  - docs/adrs/0005-serve-and-client-handoffs.md:51-61
seeAlso:
  - concepts/discovery-document
  - concepts/http-app-route
  - concepts/three-serve-bundles
  - concepts/wanix-as-host
  - reference/cli-rootfs-qemu-v86
prerequisites:
  - concepts/discovery-document
usedInFlows: []
honestLimits:
  - "QEMU and direct-v86 are validated launch handoffs, not a VM supervisor: serve does not own rootfs build, boot, or VM lifecycle."
  - "The rootfs manifest carries absolute host paths and a launch argv, so it is only ever returned to loopback clients (403 otherwise)."
  - "Ethernet/vnet is advertised but not implemented; the websocket bridge for it is a future trust-boundary decision."
canonicalCaveatFor: []
---

# Loopback-Only VM and App Handoffs

rootfs.json, the HTTP-app route, and direct-v86 are gated to loopback clients and prepared-root boot markers — launch contracts, not a VM supervisor.

A handoff is the moment Wanix stops being the runtime and hands a local launcher the exact arguments to start something it does not own: a QEMU VM, a v86 browser emulator, an HTTP app. Those handoffs leak host absolute paths and ready-to-exec argv, so `serve` only ever produces them for clients that connect over loopback against a prepared root. This page explains why the gate exists, what it gates, and where the line between "launch contract" and "VM manager" sits.

## Show it: ask for the rootfs handoff

Run `serve` on a prepared guest root and ask for the manifest from the same machine:

```sh
curl http://localhost:7654/.well-known/rootfs.json
```

From loopback against a root whose boot markers are satisfied, you get a `wanix-rootfs.v1` JSON document. Ask for it from any non-loopback address and the server answers `403 Forbidden` with the body `rootfs handoff is available only to loopback clients` (`crates/wanix-cli/src/serve/discovery.rs:93-99`). The gate is the first thing the handler checks, before it ever touches the filesystem.

The Plan 9 name for the directory `.well-known/` is just convention; the contract is the manifest kind, `wanix-rootfs.v1`.

## What the manifest actually contains

The reason for the loopback gate is right there in the payload. The `wanix-rootfs.v1` document (`crates/wanix-cli/src/rootfs/handoff.rs:32-58`) carries:

- `rootPath`, `kernelPath`, `initPath` — **absolute host filesystem paths** to the prepared root, the kernel, and `/bin/init`.
- a `qemu` sub-manifest (`wanix-qemu-virtio9p.v1`) with launch argv and policy.
- a `serveDirectV86.argv` array — a literal command line, `["wanix-rust", "serve", "<root>", "--bundle", "direct-v86", "--wanix-services"]`.

That is launch-grade trust. A document that names host paths and hands you an argv to execute is something you give a launcher you already trust on that machine, not something you broadcast to the network. So the trust model is the simplest one that is honest: `is_loopback_peer` is `peer_addr.ip().is_loopback()` (`crates/wanix-cli/src/serve/discovery/host.rs:40-42`), and only a loopback peer crosses it.

## Two gates, not one: loopback AND boot markers

Loopback is necessary but not sufficient. The discovery document reports `rootfs` status through `serve_rootfs_route_json` (`crates/wanix-cli/src/serve/discovery.rs:114-152`), and the status word tells you which gate you are behind:

- `local-only` — you are not a loopback client; nothing else is evaluated.
- `unprepared` — you are loopback, but `rootfs_handoff_readiness` found missing boot markers (a non-executable or absent `/bin/init`, a missing kernel).
- `invalid` — loopback and prepared, but building the manifest failed.
- `available` — loopback, prepared, and the manifest built cleanly.

Only `available` ever yields a real `wanix-rootfs.v1` body from `/.well-known/rootfs.json`. The boot-marker check is the same validation the `wanix-rust rootfs` and `wanix-rust qemu` subcommands run, so a root that the CLI would refuse to launch is also a root `serve` refuses to hand off. See [the rootfs/qemu/v86 CLI reference](/reference/cli-rootfs-qemu-v86) for how those markers are prepared.

## The HTTP-app route is gated the same way

The HTTP-app route, `/.wanix/app/<name>`, is shipped on this branch and gated by the identical pair of checks (`crates/wanix-cli/src/serve/http/app.rs:55-66`). A request first fails `404` with `wanix app routes require --wanix-services` if services are not enabled, then fails `403` with `wanix app routes are available only to loopback clients` if the peer is not loopback. Only past both does it run `apps/<name>.js` or `apps/<name>.wasm` as a task and return its stdout. The discovery document advertises this route with `"scope":"loopback"` so a client knows the boundary before it asks. The route itself — how a request becomes a `qjs`/`wasm` task with `#kv`-backed state — gets its own page; see [the HTTP-app route](/concepts/http-app-route).

## direct-v86 rides the same discovery

The browser v86 bundle (`serve --bundle direct-v86`) does not get a separate trust story. Its generated page consumes the same discovery document, attaches to the advertised direct-9P route, and surfaces rootfs handoff readiness — exposing the copyable QEMU/direct-v86 serve commands only when the discovery status is `available`, i.e. only to a trusted loopback client. The three serve bundles share one composition surface; see [three serve bundles](/concepts/three-serve-bundles).

## See also

- [The discovery document](/concepts/discovery-document) — the `/.well-known/wanix.json` map every client reads first.
- [The HTTP-app route](/concepts/http-app-route) — `/.wanix/app/<name>`, the services-gated, loopback-only app runner.
- [Three serve bundles](/concepts/three-serve-bundles) — fs9p, workbench, and direct-v86 over one composition surface.
- [Wanix as host](/concepts/wanix-as-host) — serve composes capabilities; it is not a VM manager.
- [CLI: rootfs, qemu, v86](/reference/cli-rootfs-qemu-v86) — preparing a root and validating boot markers.

## Status / honest limits

- **These are validated handoffs, not a VM supervisor.** `serve` (and `wanix-rust qemu`) emit a stable manifest and an explicit foreground launch; rootfs build, boot, background supervision, and VM lifecycle are out of scope and remain separate future decisions (`docs/adrs/0005-serve-and-client-handoffs.md:56-61`).
- **The gate is loopback by IP, plus prepared-root boot markers.** Anything else gets `403` (rootfs/app) or a `local-only`/`unprepared` discovery status — never the host paths and launch argv.
- **Ethernet/vnet is advertised but not implemented.** The discovery document lists a `/.well-known/ethernet` websocket route, but no bridge exists yet; it is a future trust-boundary decision, not a shipped capability.
