---
title: Reference
slug: reference/index
pageType: overview
oneLiner: Flat specs, contracts, and indices — the expert's fast path to a precise answer.
audience: [developer, visionary]
tags: [reference, cli]
sourceRefs: []
seeAlso: [reference/adr-index, reference/attach-and-capability-contract, reference/build-and-install, reference/cli-command-index, reference/cli-rootfs-qemu-v86, reference/contributor-landing, reference/crate-map-and-layering, reference/extension-points, reference/filesystem-trait, reference/guest-sdk, reference/performance, reference/quality-gates, reference/queued-follow-ups, reference/serve-and-discovery]
prerequisites: []
usedInFlows: []
honestLimits: []
canonicalCaveatFor: []
---

# Reference

Flat specs, contracts, and indices — the expert's fast path to a precise answer.

The reference section is the no-narrative layer: signatures, defaults, contracts, and indices. If you already know what you want — a trait method, a CLI flag, the attach-and-capability contract — start here, or just press `/` to search.

## Pages

- [**ADR Index & Workflow**](/reference/adr-index) — The active ADRs 0001-0005 plus the ADR-vs-commit-message workflow — review the related ADRs before you touch any boundary.
- [**Attach & capability contract**](/reference/attach-and-capability-contract) — The flat spec for how a peer attaches and what a capability grant authorizes.
- [**Build & install the CLI**](/reference/build-and-install) — The one canonical build page: rustup, `cargo build --locked --package wanix-cli`, the alias, and the optional `wasm32-wasip1` target.
- [**CLI Command Index**](/reference/cli-command-index) — Every wanix-rust subcommand at a glance — qjs, qjs-term/-shell/-snapshot/-resume/-restore, wasm, p9-*, serve, rootfs, qemu, mesh-serve, mount-*, cpu, agent, capsule — each linked to its dedicated reference or recipe page.
- [**CLI: rootfs, qemu, direct-v86 (VM Handoffs)**](/reference/cli-rootfs-qemu-v86) — rootfs prepares and validates a guest root, qemu emits a wanix-qemu-virtio9p.v1 argv handoff, and direct-v86 generates a browser emulator page — launch contracts, not a VM supervisor.
- [**Contributor Landing — This Is the Rust Port**](/reference/contributor-landing) — Orient: this is the Rust port, not the Go tree — ignore the Go Makefile/CONTRIBUTING, build with cargo, and gate with just check.
- [**Crate Map & Dependency Direction**](/reference/crate-map-and-layering) — How the Rust-native Wanix workspace is layered so dependencies point downward — core fs/namespace/protocol crates stay free of Wasmtime, runtime engines sit above the substrate, service devices are plain filesystems, and all async/iroh is confined to the single wanix-mesh edge crate.
- [**Extending Wanix from the Edges**](/reference/extension-points) — Three extension points — a service device is a FileSystem, a task driver is a TaskDriver, a transport is a 9P adapter — so you do not need to fork the core.
- [**The FileSystem / File Trait Reference**](/reference/filesystem-trait) — The trait at hand: method-by-method, the defaults, device-aware semantics, and which methods default to NotSupported.
- [**Guest SDK (lib/wanix)**](/reference/guest-sdk) — The JavaScript helper library a guest task uses on top of qjs:std/os and service files.
- [**Performance & scaling**](/reference/performance) — The rooms-not-houses cost model and what the bench actually measures.
- [**Quality Gates (just check)**](/reference/quality-gates) — fmt, module-lines, clippy -D warnings, test, and the composite just check; stay under the 250-350 line module limit; use explicit newtypes over raw i32 flags.
- [**Queued Follow-Ups (Pick a First Cleanup)**](/reference/queued-follow-ups) — The current backlog — split the over-limit modules, port the v86-shared-demo stub, wire #plumb live recv, add a serve shutdown signal plus connection cap, type the discovery JSON, and grow the per-principal namespace seam.
- [**Serve, Discovery, and Handoff JSON**](/reference/serve-and-discovery) — The serve command surface, the discovery document shape, the well-known routes, and the rootfs/qemu/v86 handoff JSON contracts.

## See also

- [Learn — guided flows](/learn/index)
- [Concept index](/find/concepts)
- [Search & glossary](/find/index)
