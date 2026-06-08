---
title: CLI Command Index
slug: reference/cli-command-index
pageType: reference
oneLiner: Every wanix-rust subcommand at a glance — qjs, qjs-term/-shell/-snapshot/-resume/-restore, wasm, p9-stdio, serve, rootfs, qemu, mesh-serve, mount-*, cpu, agent, capsule — each linked to its dedicated reference or recipe page.
audience: [developer]
tags: [cli, reference, shipped, mesh, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-cli/src/lib.rs:1-32
  - crates/wanix-cli/src/help.rs:3-70
  - crates/wanix-cli/src/mount.rs:25-29
  - rust-walkthrough.md:24-51
seeAlso:
  - reference/serve-and-discovery
  - reference/cli-rootfs-qemu-v86
  - concepts/qjs-task
  - concepts/compiled-wasm-task-driver
  - concepts/import-export-and-n
  - devices/cpu
  - concepts/wanix-capsule
prerequisites: []
usedInFlows: []
honestLimits:
  - "mesh-serve --wanix-services binds the #task/#agent exec devices, which is local-trust-only — it requires --addr IP:PORT and is refused on the public endpoint."
  - "The shipped mount-* commands bind a remote into a single slot, /n/remote; per-peer /n/<peer-id> is a labelled convention, not shipped routing."
  - "The served #agent runs a deterministic FakeEngine; the real codex engine is the local-trust 'wanix agent' CLI path only."
canonicalCaveatFor: []
---

# CLI Command Index

Every `wanix-rust` subcommand at a glance — qjs, qjs-term/-shell/-snapshot/-resume/-restore, wasm, p9-stdio, serve, rootfs, qemu, mesh-serve, mount-*, cpu, agent, capsule — each linked to its dedicated reference or recipe page.

The native binary is one entry point with a flat subcommand surface: a verb, its flags, and (for the task runtimes) a script or module argument. This page is the map of that surface. It groups the commands the way the runtime groups them — task runtimes, the snapshot family, 9P transports, the `serve` composition layer, VM handoffs, the mesh, and the agent — and points each one at the page where the behaviour is actually documented. The single source of truth for the exact spelling of every flag is the `USAGE` string the binary prints (`crates/wanix-cli/src/help.rs:3-62`); run `wanix-rust --help` to see it verbatim.

## Build once, alias once

Every command below is the same binary. Build it and alias it:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust --help
```

`wanix-rust --help` (and bare `wanix-rust` with no args) prints the demo target plus the full usage block (`crates/wanix-cli/src/lib.rs:260-268`, `help.rs:64-70`). The CLI is deliberately only composition and demo plumbing — it drives `wanix-task`, `wanix-vfs`, `wanix-wasi`, and the runtime crates; the core semantics live there, not here (`rust-walkthrough.md:53-55`).

## Task runtimes

These run a guest as a Wanix task and report its exit status.

- **`qjs <script.js> [-- arg ...]`** — run JavaScript outside Chrome as a QuickJS/WASI task. Flags: `--env KEY=VALUE`, `--cwd DIR`, `--stdin TEXT | --stdin-file PATH|-`, `--event-loop-ms N`, `--ready-io-turns N`, `--interrupt-after N`, `--memory-limit-bytes N`, `--mount HOST=GUEST`. See [the qjs task](/concepts/qjs-task) and [walkthrough 1: run JS](/recipes/walkthrough-1-run-js).
- **`qjs-term <script.js>`** — the same task with fd 0/1/2 bound through a `#term` device, plus scripted-input flags for demos (`--feed-after-eval[-file|-lines]`, `--resize-after-eval COLSxROWS`). See [the term device](/devices/term).
- **`qjs-shell [--raw]`** — an interactive terminal-backed shell with a small built-in command set. See [the qjs-shell](/concepts/qjs-shell) and [raw vs. cooked input](/concepts/raw-vs-cooked-input).
- **`wasm FILE.wasm [args...]`** — run a compiled `wasm32-wasi` module as the second WASI task driver. It is a command-style WASI subset (no `poll_oneoff` readiness), and `.wasm` is also a first-class Wanix task kind that auto-starts through `#task/new` (`help.rs:32-34`). See [the compiled-wasm task driver](/concepts/compiled-wasm-task-driver) and [two tiers, one substrate](/concepts/two-tiers-one-substrate).

## Snapshot family

QuickJS snapshots are VM images: freeze a running interpreter, then reattach.

- **`qjs-snapshot --snapshot FILE <script.js>`** — run a script and write a snapshot.
- **`qjs-resume --snapshot FILE <script.js>`** — resume from a snapshot file.
- **`qjs-restore <before.js> <after.js>`** — restore-and-continue in one process, with separate `--before-*`/`--after-*` env and arg flags (`help.rs:19-31`).

See [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images).

## 9P transports

These export a Wanix filesystem over a single transport, with binary protocol traffic kept off the diagnostic channel.

- **`p9-stdio --root DIR`** — 9P over the process's stdin/stdout (the QEMU-v86 console bridge).
- **`serve --root DIR --p9 HOST:PORT [--peer HEX --grant ANAME:PREFIX:RIGHTS ...]`** — 9P over TCP, with capability grants per peer. Raw 9P over TCP is now a `serve` mode, not a standalone subcommand; the websocket door (`/.well-known/export9p`) rides the same `serve` HTTP listener. The retired `p9-listen` and `p9-ws` subcommands folded into `serve` (ADR 0006).

See [the 9P contract](/concepts/the-9p-contract) and [serve and discovery](/reference/serve-and-discovery).

## serve — the composition layer

`serve` is the one command that composes the others into an addressable surface (HTTP, the 9P-over-WebSocket door, and an optional raw 9P-over-TCP door via `--p9`) with a discovery document.

- **`serve [--root DIR | DIR] [--listen HOST:PORT] [--p9 HOST:PORT [--peer HEX --grant ANAME:PREFIX:RIGHTS ...]] [--bundle NAME] [--wanix-services] [--once]`** (`help.rs:59-60`). (`--addr` is a deprecated hidden synonym for `--listen`; prefer `--listen`.)
  - `--wanix-services` exports the service device set (`#task`, `#term`, `#pipe`, `#kv`, `#plumb`, `#cas`, `#agent`) into the served namespace. Because it binds the `#task`/`#agent` exec devices (remote code execution), it is refused if either 9P door — the HTTP/websocket listener or the raw `--p9` door — is bound to a non-loopback address.
  - `--p9 HOST:PORT` binds a raw 9P-over-TCP door (loopback by default) alongside the HTTP listener, with capability grants per peer; the websocket 9P door rides the HTTP listener at `/.well-known/export9p`. Both are thin adapters over one per-connection 9P session core (ADR 0006). Over raw TCP `--peer` is asserted, not cryptographically proven — only the mesh's iroh QUIC transport proves identity.
  - `--bundle NAME` selects a browser launch path: `fs9p`, `workbench-fs9p` (the cockpit), or `direct-v86`.
  - The HTTP-app route `/.wanix/app/<name>` is served on this path — loopback-only and services-gated.

See [serve and discovery](/reference/serve-and-discovery), [the wanix-services device set](/concepts/wanix-services-device-set), [three serve bundles](/concepts/three-serve-bundles), [the browser cockpit](/concepts/browser-cockpit), and [the HTTP-app route](/concepts/http-app-route).

## VM handoffs

These prepare and describe a guest VM boot; none of them owns the VM lifecycle.

- **`rootfs --archive FILE.tgz --out DIR [--json]`** — extract a guest root, reject unsafe archive paths, validate boot markers (including executable `/bin/init`), and emit shell or `wanix-rootfs.v1` JSON handoffs.
- **`qemu --root DIR [--kernel PATH] [--initrd PATH] [--cmdline TEXT] [--append TEXT ...] [--qemu-bin PATH] [--memory-mb N] [--mount-tag TAG] [--security-model MODEL] [--p9-msize N] [--json] [--no-kvm] [--exec]`** — validate the guest root and emit a shell or `wanix-qemu-virtio9p.v1` JSON handoff; `--exec` is an explicit foreground launch, not a supervisor (`help.rs:55-58`).
- **`serve --bundle direct-v86`** — the browser v86 handoff path (a `serve` mode, not a separate verb).

See [CLI: rootfs, qemu, v86](/reference/cli-rootfs-qemu-v86).

## Mesh

The mesh carries 9P over iroh QUIC, keyed off the node's persisted ed25519 identity.

- **`mesh-serve --root DIR [--key FILE] [--addr IP:PORT] [--peer HEX --grant ANAME:PREFIX:RIGHTS ...] [--wanix-services] [--insecure-open]`** — bind an `iroh::Endpoint` and export the namespace under the Wanix ALPN (`help.rs:39-43`).
- **`mount-ls (tcp://HOST:PORT | iroh://PEER[?addr=IP:PORT]) [PATH]`**, **`mount-cat ... PATH`**, **`mount-write ... PATH TEXT`** — dial a 9P server, build a `RemoteFs`, bind it, and run one filesystem op (`help.rs:48-50`). The shipped binding slot is `/n/remote` (`crates/wanix-cli/src/mount.rs:25-29`). The `tcp://` form works against a live `serve --p9` door — e.g. `mount-write tcp://127.0.0.1:9999 '#sites/blog.localhost' 'dir /abs/site'`.
- **`cpu --node iroh://PEER[?addr=IP:PORT] [--cwd DIR] [--write] [--env KEY=VALUE ...] -- KIND PROGRAM [ARG ...]`** — Plan 9 cpu over the mesh: reverse-export `DIR` (read-only by default; `--write` opts into read-write) and run `KIND PROGRAM` on the data node against it (`help.rs:44-47`).
- **`capsule (save DIR | load CAPSULE_ID DIR) [--store DIR]`** — freeze a Wanix world into a portable CAS-backed `.wcap`, or load one back (`help.rs:54`).

See [import, export, and /n](/concepts/import-export-and-n), [9P over iroh QUIC](/concepts/9p-over-iroh-quic), [the cpu device](/devices/cpu), [send the agent to the data](/concepts/send-agent-to-the-data), and [the wanix capsule](/concepts/wanix-capsule). To follow it end to end, see [mount a remote peer](/recipes/02-mount-remote-peer) and [freeze a world to a capsule](/recipes/03-freeze-world-to-capsule).

## Agent

- **`agent [--fake] [--cwd DIR] [--world DIR] <prompt>`** — drive an LLM session against a confined Wanix world (`help.rs:53`). The local-trust CLI path uses the real codex app-server engine; `--fake` selects the deterministic `FakeEngine`, which is also what the served `#agent` device uses.

See [the agent device](/devices/agent), [FakeEngine vs. codex](/concepts/fakeengine-vs-codex), and [repair a broken qjs](/recipes/01-repair-broken-qjs).

## Scaffolding

- **`new (--js NAME | --rust NAME) [--dir DIR]`** — scaffold a JS or Rust task project (`help.rs:52`). See [scaffold a project](/recipes/00-scaffold-a-project).

## See also

- [Crate map and layering](/reference/crate-map-and-layering) — which crate backs each command.
- [Serve and discovery](/reference/serve-and-discovery) — the composition surface in depth.
- [CLI: rootfs, qemu, v86](/reference/cli-rootfs-qemu-v86) — the VM-handoff commands in full.
- [Import, export, and /n](/concepts/import-export-and-n) — the mesh mounting model.
- [Queued follow-ups](/reference/queued-follow-ups) — what is designed-but-unshipped.

## Status / honest limits

- **`mesh-serve --wanix-services` is local-trust only.** Binding `--wanix-services` exposes the `#task`/`#agent` exec devices, which is remote code execution; the CLI therefore requires `--addr IP:PORT` and refuses it on the public endpoint (`help.rs:39-43`). The exec devices give cheap, scalable isolation, not safety for arbitrary untrusted code — there are no hard CPU/memory limits on a guest yet. `--insecure-open` exports the host directory read-write (not the exec devices) to anyone holding the ticket.
- **`mount-*` bind a single slot.** The shipped mount point is `/n/remote` (`crates/wanix-cli/src/mount.rs:25-29`); per-peer `/n/<peer-id>` routing is designed but not shipped. Use `/n/<peer>` only as a labelled convention in prose, not as a real path you can rely on the CLI to produce.
- **`agent` without `--fake` is the only live-LLM path.** The served `#agent` device runs the deterministic `FakeEngine`; the real codex engine is reachable only through this local-trust CLI subcommand.
- **`qemu --exec` is a foreground launch, not a VM supervisor**, and the JSON handoffs (`wanix-rootfs.v1`, `wanix-qemu-virtio9p.v1`) describe a boot rather than performing one.
- **Flag spellings drift; the binary does not.** When this page and `wanix-rust --help` disagree, the `USAGE` string in `crates/wanix-cli/src/help.rs` is authoritative.
