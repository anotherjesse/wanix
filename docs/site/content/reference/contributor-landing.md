---
title: Contributor Landing — This Is the Rust Port
slug: reference/contributor-landing
pageType: developer
oneLiner: "Orient: this is the Rust-native Wanix workspace — build with cargo and gate with just check."
audience: [developer]
tags: [contributor, onboarding, cli, shipped, caveat]
sourceRefs:
  - AGENTS.md
  - README.md:23-93
  - rust-walkthrough.md:1-55
  - CONTRIBUTING.md
  - justfile:3-15
  - crates/wanix-cli/src/mount.rs:26
  - Cargo.toml
seeAlso:
  - reference/crate-map-and-layering
  - reference/quality-gates
  - reference/adr-index
  - reference/queued-follow-ups
  - concepts/wanix-as-host
  - learn/contribute-to-core
prerequisites: []
usedInFlows: []
honestLimits:
  - "The original Go/browser Wanix lives upstream at https://github.com/tractordev/wanix/; this workspace is the Rust-native implementation."
  - "The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is designed but unshipped."
canonicalCaveatFor: []
---

# Contributor Landing — This Is the Rust Port

Orient: this is the Rust-native Wanix workspace — build with cargo and gate with `just check`.

The active codebase is a Cargo workspace that runs Wanix outside Chrome, with
Wasmtime as the execution substrate and a Plan 9-style mesh on top. The original
Wanix was a Go/browser implementation hosted at
https://github.com/tractordev/wanix/; it is historical context, not a second
tree in this workspace. This page tells you how to build the Rust workspace,
where the semantics live, and how to pick a first task.

## The one-liner

Use Cargo for the core and `just check` for the merge gate. `make build`,
`make test`, and `make check` are thin wrappers around those Rust commands; the
workbench bundle uses npm/esbuild through `make workbench`.

The Rust port's canonical map is `AGENTS.md` (the project instructions, checked in at the repo root). It owns the crate shape, the dependency-direction rules, the capability map, the ADR index, and the queued follow-ups. When you start a change, read `AGENTS.md` first; it is the source of truth for "what is shipped" versus "what is designed but unbuilt."

## Build: cargo, then the wanix binary

The whole port builds from the workspace root with one command:

```sh
cargo build --locked --package wanix-cli
alias wanix='./target/debug/wanix'
```

That produces `wanix`, the aliased binary used throughout these docs and the walkthrough (`rust-walkthrough.md:24-28`). Confirm the surface:

```sh
wanix --help
```

You should see subcommands for `qjs`, `qjs-term`, `qjs-shell`, the snapshot/resume/restore trio, `p9-stdio`, `rootfs`, `qemu`, the `mount-*` verbs, `mesh-serve`, and `serve` (`rust-walkthrough.md:38-51`). Raw 9P over TCP is no longer a standalone subcommand — the retired `p9-listen`/`p9-ws` folded into `serve --p9 ADDR` and the serve websocket door (ADR 0006). The fastest "is my build alive" check is a native QuickJS task:

```sh
wanix qjs examples/qjs-demo.js
```

JavaScript runs as a Wanix task — outside Chrome, on Wasmtime — with live Wanix-backed WASI, stdio, env, and an observable exit status (`README.md:31-35`). If that prints, your environment is sound.

## Where core semantics live vs. where composition lives

The single most useful map of the codebase is the crate layering. Dependencies point **downward**: the base — filesystem, namespace, task, and protocol — knows nothing about Wasmtime, tokio, or iroh, and the async network edge lives in exactly one crate (`AGENTS.md`, "Dependency Direction"). The full diagram and the no-upward-deps invariants are in [the crate map](/reference/crate-map-and-layering); the short version for orientation:

- **Core contracts** rebuild what a file, namespace, task, and 9P frame *are*: `wanix-fs` (the zero-dependency root), `wanix-vfs`, `wanix-task`, `wanix-protocol`, `wanix-9p`, `wanix-term`.
- **Runtime tier** is the only place Wasmtime is linked: `wanix-module-cache`, `wanix-qjs-engine`, `wanix-wasi-host`, `wanix-wasm` (plus `wanix-wasi`, `wanix-qjs`).
- **Service devices** are plain `FileSystem`s — `wanix-kv`, `wanix-pipe`, `wanix-plumb`, `wanix-cas`, `wanix-agent`, `wanix-id` — which is exactly why they import across the mesh for free.
- **Import half + mesh edge**: `wanix-9p-client`, `wanix-cpu`, and `wanix-mesh` (the only async/iroh crate).

Composition is **not** semantics. `wanix-cli` is intentionally just plumbing: it drives `wanix-task`, `wanix-vfs`, `wanix-wasi`, and `wanix-qjs`, but it is "not where core task or filesystem semantics live" (`rust-walkthrough.md:53-55`). If you find yourself encoding a rule about how tasks or files behave inside `wanix-cli`, it belongs a layer down. The CLI owns `serve`, the mesh subcommands, `capsule`, and the demo runners — orchestration, not contracts.

## The walkthrough path and the recipes

To feel the system before changing it, follow [rust-walkthrough.md](/recipes/walkthrough-1-run-js) from the workspace root. It is written from a user's seat: run a command, look at the visible result, then read a short aside naming the architecture piece you just exercised — `qjs` outside Chrome, terminal-backed `qjs-shell`, the `serve` 9P exports, the browser cockpit. The numbered hands-on recipes go deeper into single flows: [repair a broken qjs program through `#agent`](/recipes/01-repair-broken-qjs), [mount a remote peer](/recipes/02-mount-remote-peer), [freeze a world to a capsule](/recipes/03-freeze-world-to-capsule), [a tiny HTTP app backed by `#kv`](/recipes/04-tiny-http-app-with-kv), and [two agents collaborating](/recipes/05-two-agents-collaborate). When you want the end-to-end "land a change in core" loop, see [contribute to core](/learn/contribute-to-core).

## How to pick a first task

The honest backlog lives in `AGENTS.md` under "Queued Follow-ups," mirrored in [queued follow-ups](/reference/queued-follow-ups). Good starting points there, in roughly ascending depth:

- **Module-line health.** `just module-lines` is green against the hard 350-line limit, but three modules sit above the 250-line warn limit and should be split before they grow: `wanix-agent/src/codex.rs` (~307), `wanix-agent/src/exec_server.rs` (~283), and `wanix-cli/src/serve/http/app.rs` (~273). Self-contained, no new behaviour.
- **Cockpit coverage.** `v86-shared-demo` is still a no-op stub in `workbench/src/web/extension.ts` (marked `// STUB:`).
- **Typed handoff JSON.** Discovery, rootfs, qemu, and direct-v86 handoffs are still hand-built `format!` JSON; converting fragments to typed structs with shape-pinning tests is a contained cleanup.

Bigger, named work — the serve shutdown signal + connection cap, the per-principal namespace seam, and the single-connection `#plumb` recv limitation — is described with its constraints in the same section. Read the constraint before you start; several of these are blocked on a prerequisite (e.g. the connection cap needs a shutdown signal first).

## Quality gate

Run the full gate before committing any cycle (`justfile:3-15`):

```sh
just check   # fmt + module-lines + clippy -D warnings + test
```

`just check` formats every workspace crate with `--check`, enforces the module-line baseline, runs `cargo clippy --workspace --all-targets -- -D warnings`, and runs `cargo test --workspace --locked`. Clippy warnings are errors here; treat the gate as the merge bar. The full rules — module-line limits and the lock-discipline guardrail — are in [quality gates](/reference/quality-gates). Architecture decisions you must respect when touching a contract are indexed in the [ADR index](/reference/adr-index).

## See also

- [Crate map & dependency direction](/reference/crate-map-and-layering) — the full layered diagram and no-upward-deps rules.
- [Quality gates](/reference/quality-gates) — `just check`, module-line limits, lock discipline.
- [ADR index](/reference/adr-index) — the active architecture contracts.
- [Queued follow-ups](/reference/queued-follow-ups) — the honest backlog and first-task list.
- [Wanix as host](/concepts/wanix-as-host) — what the Rust core owns and why.
- [Contribute to core](/learn/contribute-to-core) — the end-to-end change-landing flow.

## Status / honest limits

The original Go/browser Wanix is not part of this workspace anymore. When a Plan
9 behaviour is unclear, treat https://github.com/tractordev/wanix/ as historical
reference material, not as a directory layout to mirror. The Rust implementation
is organized around the downward-dependency rule, so mapping old Go packages
onto Rust crates one-to-one will mislead you.

One shipped-vs-designed boundary worth knowing early as a contributor: the CLI
mesh mount binds a single slot at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`).
Per-peer `/n/<peer-id>` mounts are designed but unshipped — treat `/n/<peer>` in
docs and recipes as a labelled convention, not a working multi-peer path.
