---
title: Crate Map & Dependency Direction
slug: reference/crate-map-and-layering
pageType: reference
oneLiner: How the Rust-native Wanix workspace is layered so dependencies point downward — core fs/namespace/protocol crates stay free of Wasmtime, runtime engines sit above the substrate, service devices are plain filesystems, and all async/iroh is confined to the single wanix-mesh edge crate.
audience: [developer]
tags: [crate-map, layering, dependency-direction, architecture, mesh, cli, shipped, caveat]
sourceRefs:
  - AGENTS.md:100-138
  - docs/adrs/0001-rust-native-wasmtime-runtime.md:44-48
  - Cargo.toml
  - crates/wanix-mesh/Cargo.toml
  - crates/wanix-9p-client/Cargo.toml
  - tools/module-line-baseline.txt
seeAlso:
  - concepts/async-sync-bridge
  - concepts/task-drivers
  - concepts/protocol-vs-server-split
  - reference/extension-points
  - reference/quality-gates
prerequisites: []
usedInFlows: []
honestLimits:
  - "AGENTS.md:100-138 is the human-maintained dependency map and lags the Cargo manifests in spots (e.g. it shows wanix-9p-client -> wanix-9p + wanix-kv and omits wanix-9p's dependency on wanix-id); when prose and Cargo.toml disagree, the manifest wins."
  - "wanix-9p-client's wanix-9p and wanix-kv references are dev-dependencies for tests, not in the shipped runtime graph."
  - "The no-upward-deps rules are enforced only by what compiles, not by a dedicated lint."
  - "tools/module-line-baseline.txt is currently empty; three modules sit above the 250-line warn threshold (codex.rs 307, exec_server.rs 283, serve/http/app.rs 273) but under the 350-line hard limit."
canonicalCaveatFor: []
---

# Crate Map & Dependency Direction

Wanix's Rust core is a Cargo workspace of 22 crates, and the whole thing is organized around one rule: dependencies point **downward**. The base — filesystem, namespace, task, and protocol semantics — knows nothing about Wasmtime, nothing about tokio, nothing about iroh. The execution substrate sits on top of that base; the service devices are plain filesystems hanging off it; and every byte of async networking lives in exactly one crate at the very edge. If you keep the arrows pointing down, you can reason about any layer in isolation, swap an engine or a transport without touching the kernel, and trust that a `wanix-fs` change can't accidentally drag Wasmtime into a crate that should never see it. This page is the map. ([ADR 0001](/reference/adr-index) records the ownership rule; `AGENTS.md:100-138` is the canonical text.)

## The layered diagram

Read it bottom-up. Each crate depends only on crates below it.

```text
                         wanix-cli            <- orchestration / demos
   ──────────────────────────────────────────────────────────────────
                         wanix-mesh           <- THE async/iroh edge
            wanix-cpu        wanix-9p-client   <- Plan 9 import half
   ──────────────────────────────────────────────────────────────────
   wanix-kv  wanix-pipe  wanix-plumb          <- service devices
   wanix-agent  wanix-cas  wanix-id              (plain FileSystems)
   ──────────────────────────────────────────────────────────────────
   wanix-qjs    wanix-wasm                    <- WASI task runtimes
   wanix-qjs-engine  wanix-wasi-host          <- Wasmtime-hosted
   wanix-wasi   wanix-module-cache
   ──────────────────────────────────────────────────────────────────
   wanix-term            wanix-9p             <- terminals / 9P server
   ──────────────────────────────────────────────────────────────────
   wanix-vfs  wanix-task  wanix-protocol      <- namespace / task / wire
   ──────────────────────────────────────────────────────────────────
                         wanix-fs             <- THE base (zero deps)
```

`wanix-fs` has no `wanix-*` dependencies at all — its `[dependencies]` table is empty. That is the foundation everything else is allowed to lean on.

## Core crates

These are the contracts the Rust port rebuilds, and per [ADR 0001](/reference/adr-index) they "must remain free of Wasmtime and QuickJS." They define what a file, a namespace, a task, and a 9P frame *are* — independent of how guests execute or how bytes cross a network.

- **`wanix-fs`** — filesystem traits, metadata, errors, path rules, in-memory fixtures, and host-directory-backed filesystems. The root of the graph; it depends on nothing in the workspace.
- **`wanix-vfs`** — Plan 9-style namespace binding and resolution (`-> wanix-fs`). This is the everything-is-a-file glue: bind `#kv` here, mount a remote export there, and resolution walks one tree.
- **`wanix-task`** — the task model, the `#task` device, the fd table, and the driver registry (`-> wanix-fs + wanix-vfs`). Crucially, it does **not** know about any specific runtime. See [task drivers](/concepts/task-drivers).
- **`wanix-protocol`** — dependency-free wire helpers: 9P frame splitting, tag extraction, version negotiation, and the server-facing 9P2000.L/Google compat codecs. No server policy, no transport — see [protocol vs. server split](/concepts/protocol-vs-server-split).
- **`wanix-9p`** — 9P server adapters backed by Wanix filesystems: fid state, metadata/mutation mapping, compatibility probes (`-> wanix-fs + wanix-protocol`, plus `wanix-id` for capability checks).

`wanix-term` sits alongside these (`-> wanix-fs + wanix-task + wanix-vfs`), owning the `#term/<id>/{ctl,data,program,winch}` terminal device.

## Runtime crates above Wasmtime

This is the only tier allowed to link Wasmtime, and only four crates do: `wanix-module-cache`, `wanix-qjs-engine`, `wanix-wasi-host`, and `wanix-wasm`. Everything below this line compiles without a WASM engine in the build graph.

- **`wanix-wasi`** — custom WASI Preview 1 imports backed by Wanix namespaces and task fds (`-> wanix-fs + wanix-task + wanix-vfs`). Note it depends on `wanix-task`, not the other way around.
- **`wanix-module-cache`** — the shared, audited compiled-artifact cache (fd-based trust-boundary verification, atomic write, owner-private dirs). Used by both engines (`-> Wasmtime`).
- **`wanix-qjs-engine`** — Wasmtime-hosted QuickJS/WASI mechanics (`-> Wasmtime + wanix-module-cache`). It deliberately depends on no core Wanix crates beyond the cache; task semantics live a layer up.
- **`wanix-qjs`** — the QuickJS/WASI task **driver** that adapts the engine to Wanix tasks (`-> wanix-fs + wanix-vfs + wanix-task + wanix-wasi + wanix-qjs-engine + wanix-module-cache`). It reaches Wasmtime transitively through the engine, never directly.
- **`wanix-wasi-host`** — a standalone Wasmtime WASI P1 linker for the compiled wasm runner, generic over a `WasiHost` backing, with no task coupling (`-> wanix-fs + wanix-wasi + Wasmtime`).
- **`wanix-wasm`** — the compiled-`wasm32-wasi` task driver (`-> wanix-fs + wanix-vfs + wanix-task + wanix-wasi + wanix-wasi-host + wanix-module-cache + Wasmtime`).

## Service devices as plain FileSystems

The mesh's quiet superpower lives here. Each service device is just a `FileSystem` over `wanix-fs` — which is exactly why every one of them imports across the mesh for free: a remote `#kv` is reachable at `/n/A/#kv/<key>` with no device-specific networking code, because mounting a remote export is the same as binding any other filesystem.

- **`wanix-kv`** — `#kv` key/value store; `#kv/<key>` reads/writes a value (`-> wanix-fs`).
- **`wanix-pipe`** — `#pipe` in-memory byte channels; `#pipe/new` allocates, `<id>/data` is the read/write end (`-> wanix-fs`).
- **`wanix-plumb`** — `#plumb` plumber bus; `<topic>/send` publishes newline-JSON, `<topic>/recv` drains (`-> wanix-fs`).
- **`wanix-agent`** — `#agent` device fronting an LLM session as files: `new`, `prompt`, `events`, `pending`, `ctl`, `reply`, `status` (`-> wanix-fs + wanix-vfs`).
- **`wanix-cas`** — content-addressed store (venti): `<hash>` read, `ingest` write-then-hash, `have/<hash>` probe (`-> wanix-fs + wanix-module-cache`).
- **`wanix-id`** — node identity (persisted ed25519) and default-deny capability grants (`-> wanix-fs + wanix-vfs`). This is the crate the trust boundary leans on; `wanix-9p` depends on it to gate attach.

Adding a new one? See [add a service device](/learn/add-a-service-device) and [extension points](/reference/extension-points) — the pattern is "implement `FileSystem`, bind it into the namespace, get mesh reach for free."

## The 9P import half + the mesh edge

Plan 9's other half is *import*: mounting a remote namespace as local files. Three crates realize it.

- **`wanix-9p-client`** — `RemoteFs` mounts a remote 9P export as a local `FileSystem`, sharing the synchronous 9P core with `wanix-9p`. Its runtime dependencies are only `wanix-fs + wanix-protocol`; `wanix-9p` and `wanix-kv` appear as **dev-dependencies** for tests, not in the shipped graph. (`AGENTS.md` lists them inline; the Cargo.toml is the precise word.)
- **`wanix-cpu`** — Plan 9 `cpu(1)` over the mesh: a `#cpu` acceptor runs a task against the caller's reverse-exported namespace (`-> wanix-9p + wanix-9p-client + wanix-fs + wanix-task + wanix-vfs`).
- **`wanix-mesh`** — the network edge, and the **only async crate in the workspace.** It binds one `iroh::Endpoint` per node from the `wanix-id` secret key, exports a namespace as 9P over QUIC, and dials peers to import theirs. Its deps include `iroh`, `iroh-blobs`, `iroh-gossip`, and `tokio` alongside `wanix-9p`, `-9p-client`, `-cas`, `-cpu`, `-plumb`, `-kv`, `-agent`, `-id`, `-task`, `-term`, and `-vfs` (`crates/wanix-mesh/Cargo.toml`).

How a single async crate drives a synchronous 9P core is its own subject — see [the async/sync bridge](/concepts/async-sync-bridge).

Above all of this, **`wanix-cli`** orchestrates: `serve`, the mesh subcommands, `capsule`, and the demo runners. It depends on runtime crates for orchestration and otherwise stays out of the contracts.

## No-upward-deps rules

Two invariants keep the graph honest. Both are checked in CI by what compiles.

1. **`wanix-task` must not depend on `wanix-wasi`, `wanix-qjs`, or `wanix-wasm`.** Tasks are a kernel concept; runtimes are drivers that plug *into* the task model. The arrow always runs runtime → task, never the reverse — that is what lets you add a third task driver without editing the task crate.

2. **Keep tokio and iroh out of every crate but `wanix-mesh`.** Verified directly: grepping the workspace, `iroh` appears in exactly one `Cargo.toml` (`wanix-mesh`), and so does `tokio`. Likewise `wasmtime` appears in exactly four crates, all in the runtime tier. The synchronous 9P core that the mesh reuses stays synchronous; the mesh is where the world becomes async, and nowhere else.

These aren't style preferences. They are why you can run the entire core — tasks, namespaces, devices, 9P server — in a plain synchronous test with no runtime engine and no network stack linked in.

## Status and honest limits

The workspace is green against the hard guardrail and carries a small, known debt against the softer one. Modules should stay under 250-350 non-test lines; 350 is the hard limit, 250 is a warning.

```sh
just module-lines
```

Today that prints `module-lines ok: production Rust modules fit current baseline and hard limit 350`, with three modules flagged above the 250-line *preferred* limit:

```text
crates/wanix-agent/src/codex.rs        307 lines
crates/wanix-agent/src/exec_server.rs  283 lines
crates/wanix-cli/src/serve/http/app.rs 273 lines
```

`tools/module-line-baseline.txt` — the registry of grandfathered over-limit production modules — is currently **empty** (comments only). Nothing is exempted above the hard limit; those three files are above the *warn* threshold and should be split before they grow, not before any other work. The general guardrail also bans holding a namespace or filesystem lock while calling into another filesystem, and prefers explicit newtypes over raw `i32` flags at trust boundaries. Run the full gate before committing a cycle:

```sh
just check   # fmt + module-lines + clippy -D warnings + test
```

One drift to know about: `AGENTS.md:100-138` is the human-maintained map and lags the Cargo manifests in a couple of spots (e.g. it shows `wanix-9p-client -> wanix-9p + wanix-kv` and omits `wanix-9p`'s dependency on `wanix-id`). When the prose and the `Cargo.toml` disagree, the manifest wins.

## See also / next

- [The async/sync bridge](/concepts/async-sync-bridge) — how `wanix-mesh` drives the synchronous 9P core.
- [Task drivers](/concepts/task-drivers) — why `wanix-qjs` and `wanix-wasm` plug into `wanix-task` from above.
- [Protocol vs. server split](/concepts/protocol-vs-server-split) — the `wanix-protocol` / `wanix-9p` boundary.
- [Extension points](/reference/extension-points) and [Add a service device](/learn/add-a-service-device) — building a new `FileSystem` device that meshes for free.
- [Quality gates](/reference/quality-gates) — the module-line rules and `just check` gate in full.
- [Contribute to core](/learn/contribute-to-core) — the end-to-end flow for landing a change in these crates.
