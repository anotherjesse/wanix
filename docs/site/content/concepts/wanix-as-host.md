---
title: Wanix-as-Host, not Browser-as-Host
slug: concepts/wanix-as-host
pageType: concept
oneLiner: The Rust port moves the runtime boundary out of Chrome — Wanix is a native OS core, and the browser becomes one excellent client of it.
audience: [developer, visionary]
tags: [architecture, runtime, wasmtime, shipped, caveat]
sourceRefs:
  - docs/rust-vs-go-wanix.md:13-17
  - docs/rust-vs-go-wanix.md:104-135
  - docs/rust-vs-go-wanix.md:227-256
  - docs/rust-vs-go-wanix.md:643-652
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - concepts/wasmtime-as-substrate
  - concepts/host-not-ambient-authority
  - concepts/missing-half-of-9p
  - concepts/browser-cockpit
prerequisites:
  - concepts/wasmtime-as-substrate
usedInFlows:
  - {flow: plan9-ideas-tour, step: 2}
honestLimits:
  - Go Wanix is still broader as a browser web-OS; the Rust port is narrower but deeper at the native runtime boundary — there is no parity claim.
  - The runtime can run without Chrome, but orchestration, public auth, and multi-tenant isolation remain future work.
  - Exec devices (#task/#agent/#cpu) are local-trust only; cheap isolation is not the same as safe-for-untrusted, and there are no hard CPU/memory limits yet.
---

# Wanix-as-Host, not Browser-as-Host

The Rust port moves the runtime boundary out of Chrome — Wanix is a native OS core, and the browser becomes one excellent client of it.

Wanix began as a web-native OS: Plan 9 ideas, per-task namespaces, file-like services, and an interactive UI living *inside the page*. The Go implementation proved that the browser could be the host, not just a viewer. The Rust port keeps the north star and changes one thing — the host boundary. That single move is what this page is about, and it is "not cosmetic. It changes the runtime substrate, the module boundaries, the meaning of WASI, the command-line surface, the protocol work" (`docs/rust-vs-go-wanix.md:18-21`). Get this boundary right and durable agents, scheduled jobs, scale-out, and reachable terminals become possible. Get it wrong and you are back inside a tab.

## Two questions, one boundary

The whole shift fits in two questions the project poses against each other (`docs/rust-vs-go-wanix.md:13-17`):

- Go Wanix asks: *what if the browser can be the operating environment?*
- Rust Wanix asks: *what if Wanix can be a native operating environment, and the browser is one excellent client of it?*

Both answers are real systems. The difference is where the kernel lives.

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

# The kernel is this process, not a tab.
wanix qjs examples/qjs-demo.js
```

That command runs JavaScript outside Chrome as a Wanix task: it reads and writes files through a namespace, prints through Wanix stdio, exits with an observable status, and can read `#task/self/id` to prove task context crossed into QuickJS (`docs/rust-vs-go-wanix.md:182-190`). No DOM, no service worker, no `syscall/js`. The runtime is the process you launched. That is "the minimum useful proof that the host boundary moved."

## If the tab is the kernel, the runtime dies with the tab

The Go version's center of gravity is the browser, and that is its strength *and* its ceiling. "If the laptop closes, the runtime stops. If the tab dies, the environment dies. If you want long-running work, scaling, cloud deployment, multi-user hosting, or jobs that continue without an active browser, you eventually run into the fact that the browser is the kernel" (`docs/rust-vs-go-wanix.md:104-107`).

Name the things that get hard when the page is the host: a long-running agent that should keep working after you close the laptop; a scheduled job; scale-out across machines; a terminal that survives a refresh; an externally reachable 9P endpoint. None of these are impossible in a browser, but each fights the substrate. Move the kernel into a native process and they stop being heroics. The runtime outlives any one client because no client *is* the runtime.

## Cloud when you need durability, local when you need immediacy

Moving the kernel out of Chrome does not banish the browser — it demotes it to a client, and that demotion is what lets the same runtime ideas flow both ways (`docs/rust-vs-go-wanix.md:638-652`). A native Wanix core can run in the cloud, attach explicit storage mounts, run QuickJS/WASI tasks, export a namespace over 9P, and let clients attach through terminal, browser, editor, or v86 routes. The same ideas come back down to your machine: a local terminal attaches to a Wanix terminal; a browser boots v86 against a direct 9P route; a VS Code-style frontend discovers a namespace and edits it.

That is the loop the architecture is built for: *cloud when you need durability and scale, local/browser interaction when you need immediacy.* The [browser cockpit](/concepts/browser-cockpit) is the clearest expression of the new role — a full operator surface that drives a served namespace over direct 9P, with the runtime running somewhere else and surviving the cockpit's lifetime.

## WASI inverted: an adapter *into* Wanix, not a delegation *out* to the host

The most consequential consequence of the boundary move shows up in what WASI means. In most systems WASI is a way to *delegate* filesystem and fd behavior to the host. Rust Wanix deliberately goes the other direction: "WASI becomes an adapter into Wanix-owned semantics" (`docs/rust-vs-go-wanix.md:227-241`).

When a QuickJS or `.wasm` guest opens a file, reads a directory, writes stdout, creates a symlink, or polls readiness, that call does not reach the host filesystem. It flows through Wanix namespace and fd policy. The guest sees familiar APIs — `qjs:std`, `qjs:os`, `scriptArgs`, stdin/stdout/stderr, environment — but *the authority boundary is Wanix*. This is why host directories arrive only through explicit mounts:

```sh
# The guest sees a Wanix path. The host path is not ambient authority.
wanix qjs --mount /host/project=workspace main.js
```

The guest can name `workspace/...`; it cannot name `/host/project` or anything else on the machine. That distinction matters for local demos and matters far more for cloud execution, where you want to say *this task gets this namespace, these mounts, these fds, this terminal, this export* — and nothing leaks in because it was convenient for a demo. The inversion has its own page; see [host, not ambient authority](/concepts/host-not-ambient-authority). It rests on Wasmtime being the substrate that lets every guest call be intercepted at the boundary; see [Wasmtime as substrate](/concepts/wasmtime-as-substrate).

## Why the host boundary unlocks the mesh

There is a second payoff. Once the kernel is a native process that exports a namespace over 9P, importing *another* node's namespace is symmetric — the same protocol, the other direction. That is [the missing half of 9P](/concepts/missing-half-of-9p): export was always there, and a native host makes import a first-class peer operation rather than a browser bridge. Devices, agents, and compute compose across machines through one contract precisely because the runtime no longer needs a tab to stay alive.

## See also

- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — the execution engine that makes the WASI inversion enforceable at the boundary.
- [Host, not ambient authority](/concepts/host-not-ambient-authority) — why guests get an explicit namespace, never the host filesystem.
- [The missing half of 9P](/concepts/missing-half-of-9p) — export and import as the symmetric mesh contract a native host unlocks.
- [The browser cockpit](/concepts/browser-cockpit) — the browser in its new role: one excellent client of a runtime that outlives it.
- [The Plan 9 ideas tour](/learn/plan9-ideas-tour) — the guided flow this page threads into.

## Status / honest limits

- **No parity claim.** "The Go version is still broader as a browser system. The Rust version is narrower, but it is deeper at the native runtime boundary" (`docs/rust-vs-go-wanix.md:25-27`). The Go side still owns the fuller web-component authoring model, the broad v86/workbench integration, and the web-native filesystem toolkit. Read this page as a deliberate boundary move, not a feature-complete replacement.
- **Native, but not yet operationally complete.** The runtime can run without Chrome, but "orchestration/auth remain future work" — public auth, writable export policy, HTTPS, Ethernet/vnet, and multi-tenant isolation are unimplemented (`docs/rust-vs-go-wanix.md:153, 668-669`).
- **Cheap isolation is not untrusted isolation.** The exec devices (`#task`, `#agent`, `#cpu`) are local-trust only and are not exposed to untrusted or public peers; there are no hard CPU or memory limits yet.
- **The mesh slot is a labelled convention.** The shipped CLI `mount` binds a single slot at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` paths are designed but unshipped. Use `/n/<peer>` only as a naming convention, not as a live multi-peer mount table.
