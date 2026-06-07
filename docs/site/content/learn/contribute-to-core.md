---
title: Contribute to the core
slug: learn/contribute-to-core
pageType: flow
oneLiner: "Land a clean Rust-core PR: the crate map, dependency rules, the active ADRs, and the quality gates."
audience: [developer]
tags: [contributing, cli, mesh, shipped, caveat]
sourceRefs:
  - AGENTS.md
  - docs/adrs/0001-rust-native-wasmtime-runtime.md
  - crates/wanix-fs/src/lib.rs:14-20
  - crates/wanix-fs/src/traits.rs:66-150
  - crates/wanix-fs/src/traits.rs:152-334
  - crates/wanix-kv/src/lib.rs:120-177
  - Justfile:3-15
  - tools/check-module-lines.sh:4-6
  - tools/module-line-baseline.txt
  - crates/wanix-cli/src/mount.rs:26
  - CONTRIBUTING.md
seeAlso:
  - concepts/the-filesystem-trait
  - concepts/two-tiers-one-substrate
  - concepts/service-devices
  - reference/crate-map-and-layering
  - reference/adr-index
  - reference/quality-gates
  - reference/extension-points
  - reference/queued-follow-ups
prerequisites: []
usedInFlows: []
honestLimits:
  - "The repo root CONTRIBUTING.md and the Makefile describe the Go tree; the Rust port uses the workspace crates and `just check`, not `make build`."
  - "`#kv` is in-memory: a contributor's test state lives only as long as the process. Freeze a world to a capsule to persist it."
  - "The shipped CLI mount binds one slot at /n/remote; per-peer /n/<peer-id> is designed, not shipped."
canonicalCaveatFor: []
---

# Contribute to the core

Land a clean Rust-core PR: the crate map, dependency rules, the active ADRs, and the quality gates.

This flow is for someone who has a change in hand and wants it to pass review on the first pass. It is a route across the contributor reference pages in the order you actually need them: orient on the Rust tree (not the Go one), learn the dependency direction the layering enforces, pick the ADR that governs your area, read the one trait every device implements, study `#kv` as the smallest worked example, then run the gate. The goal is a green `just check` and a diff that respects the boundaries the codebase already protects.

## Step 1 — You are in the Rust port, not the Go tree

Build the binary once and alias it, exactly as every other flow does:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

If you read the repo-root `CONTRIBUTING.md`, you will find a Docker/`make build`/TinyGo workflow and a directory layout with `api/`, `gojs/`, and `web/`. That document describes the original Go-and-JavaScript Wanix and is not the runtime you are changing. The Rust-native port lives in `crates/`, builds with cargo, and gates with `just`. ADR 0001 records this directly: the Rust port rebuilds Wanix-owned contracts and is "not a line-by-line translation of the old tree" (`docs/adrs/0001-rust-native-wasmtime-runtime.md`). Use the Go tree only as a semantic oracle when behavior is ambiguous; ignore its build system.

## Step 2 — The crate map and the dependency direction

The layering in `AGENTS.md` is law, not advice. The shape, lowest to highest: `wanix-fs` owns filesystem traits, paths, and metadata; `wanix-vfs` and `wanix-task` build on it; `wanix-protocol` and `wanix-9p` own the wire format; the runtime engines (`wanix-qjs-engine`, `wanix-wasm`, `wanix-wasi-host`) sit above and depend on Wasmtime; the service devices (`wanix-kv`, `wanix-pipe`, `wanix-plumb`, `wanix-cas`, `wanix-agent`) are plain `FileSystem`s over `wanix-fs`.

Two rules catch most review failures:

- **No upward dependencies.** `wanix-task` must never depend on a runtime engine (`wanix-wasi`, `wanix-qjs`, `wanix-wasm`); core filesystem and namespace crates stay free of Wasmtime.
- **Async and iroh are confined to `wanix-mesh`.** It is the single network edge — the only crate that pulls in tokio and iroh. Keep them out of the service devices and out of the synchronous 9P core the mesh reuses. A `tokio` import in `wanix-kv` is an automatic no.

The payoff for obeying this: because every service device is a synchronous `FileSystem`, it imports across the mesh for free — bind it into the served namespace and it is reachable as a remote file with no transport code. See [service devices](/concepts/service-devices) and [the crate map](/reference/crate-map-and-layering).

## Step 3 — Pick the ADR that governs your area

The active set is small and consecutive (`docs/adrs/`):

- **0001** — Rust is the host/microkernel runtime, Wasmtime is the substrate, Go is an oracle, crates stay layered.
- **0002** — the WASI task-runtime boundary (`.js` and `.wasm`): Wanix owns task identity and live WASI; the engine crate owns mechanics.
- **0003** — `wanix-term` and the shell/terminal lifecycle.
- **0004** — `wanix-protocol`/`wanix-9p` own the 9P frame, codec, fid, and compatibility contract.
- **0005** — `serve`, the browser/workbench, direct-v86, rootfs, and QEMU share explicit discovery and handoff contracts.

The workflow rule: treat ADRs like code, but add one only for a durable architecture, API, format, trust-boundary, or workflow decision. Milestone proofs, per-syscall coverage, and demo slices belong in tests, examples, and commit messages. "If an ADR draft reads like a good commit message, keep it as the commit message instead." Revise an existing ADR before adding a new one. See [the ADR index](/reference/adr-index).

## Step 4 — The one trait every device implements

`FileSystem` and `File` live in `crates/wanix-fs/src/traits.rs` and are re-exported from `lib.rs`. `File` is the open-handle trait — `read`, `write`, `seek`, `metadata`, plus `read_ready`/`write_ready` for device readiness (`traits.rs:66-150`). `FileSystem` is the namespace-facing trait: `open`, `metadata`, `read_dir`, and the mutation methods (`traits.rs:152-334`). Most methods default to `Err(FsError::NotSupported)`, so a read-only device implements almost nothing. Two hooks carry the trust boundary: `confine_to_prefix` (the confinement check a re-rooting export consults before following symlinks) and `content_hash` (the only place the 9P control plane offloads bulk bytes to the content-addressed data plane). Read [the FileSystem trait](/concepts/the-filesystem-trait) and [its reference](/reference/filesystem-trait).

## Step 5 — A worked example: `#kv`

`wanix-kv` is the smallest real device and the model to copy. `KvDevice` holds an `Arc<RwLock<BTreeMap<String, Vec<u8>>>>` and implements `FileSystem` (`crates/wanix-kv/src/lib.rs:120-177`): `open` on `#kv/<key>` returns a read or write file, `read_dir` lists keys, `remove_file` deletes one. No async, no Wasmtime, no transport — which is exactly why it imports across the mesh as `/n/<peer>/#kv/...` for free. Two facts to keep honest in any test you write against it: `#kv` is **in-memory**, so state lives only as long as the serve process (freeze to a capsule to persist), and the HTTP counter demo's key is `#kv/http-counter`, served loopback-only behind `--wanix-services` at `/.wanix/app/<name>`. See [#kv](/devices/kv) and [extension points](/reference/extension-points).

## Step 6 — Quality gates

One command runs everything before a cycle commit (`Justfile:3-15`):

```sh
just check    # fmt --check, module-lines, clippy -D warnings, cargo test --workspace --locked
```

Module-line health is enforced: `tools/check-module-lines.sh` warns at 250 non-test lines and hard-fails at 350. A module over 250 needs a clear reason to keep growing; over 350 must be split before new feature work lands. Existing over-limit modules are pinned in `tools/module-line-baseline.txt` — they may shrink but must not grow. Use explicit Rust types for public contracts; avoid raw `i32` flags or fds where a newtype makes the trust boundary clearer. Never hold a namespace or filesystem lock while calling into another filesystem. See [quality gates](/reference/quality-gates).

## Step 7 — Good first cleanups

`AGENTS.md` keeps a "Queued follow-ups" list — these are scoped, honest starting points: split the three modules sitting above the 250-line warn limit (`wanix-agent/src/codex.rs`, `exec_server.rs`, `wanix-cli/src/serve/http/app.rs`); convert the hand-built `format!` discovery/handoff JSON to typed structs with shape-pinning tests; or wire the second 9P connection that `#plumb` live-receive needs. One boundary worth knowing before you touch the mesh CLI: the shipped mount binds a single slot at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` is designed-but-unshipped, so treat `/n/<peer>` as a labelled convention only. See [queued follow-ups](/reference/queued-follow-ups).

## See also

- Concepts: [the FileSystem trait](/concepts/the-filesystem-trait) · [service devices](/concepts/service-devices) · [two tiers, one substrate](/concepts/two-tiers-one-substrate)
- Reference: [crate map and layering](/reference/crate-map-and-layering) · [the ADR index](/reference/adr-index) · [quality gates](/reference/quality-gates) · [extension points](/reference/extension-points) · [queued follow-ups](/reference/queued-follow-ups)
- Devices: [#kv](/devices/kv)
- Other flows: [add a service device](/learn/add-a-service-device) · [add a driver or transport](/learn/add-driver-or-transport)

## Status / honest limits

- The repo-root `CONTRIBUTING.md` and `Makefile` describe the **Go tree** (Docker, `make build`, TinyGo). The Rust port builds with cargo and gates with `just check`; ignore the Go build instructions for core work.
- `#kv` is **in-memory**: any state a test or demo writes lives only as long as the serve process. Freeze a world to a capsule (`wanix-rust capsule`) to persist it.
- The shipped CLI mount binds **one slot at `/n/remote`** (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is designed, not shipped — use `/n/<peer>` only as a labelled convention.
- Exec devices (`#task`, `#agent`, `#cpu`) are **local-trust only**; the served `#agent` runs a deterministic `FakeEngine`, not a live LLM (real codex is the `wanix agent` CLI path). Do not write contributor docs or code that implies they are safe for arbitrary untrusted peers.
