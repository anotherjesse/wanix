---
title: ADR Index & Workflow
slug: reference/adr-index
pageType: reference
oneLiner: The active ADRs 0001-0005 plus the ADR-vs-commit-message workflow — review the related ADRs before you touch any boundary.
audience: [developer]
tags: [reference, workflow, shipped, mesh, caveat]
sourceRefs:
  - docs/adrs/0001-rust-native-wasmtime-runtime.md
  - docs/adrs/0002-quickjs-wasi-task-runtime.md
  - docs/adrs/0003-terminal-device-and-shell-lifecycle.md
  - docs/adrs/0004-rust-9p-protocol-and-server-contract.md
  - docs/adrs/0005-serve-and-client-handoffs.md
seeAlso:
  - reference/crate-map-and-layering
  - concepts/the-9p-contract
  - concepts/wasmtime-as-substrate
  - concepts/serve-composition-surface
  - concepts/trust-boundary-gaps
prerequisites: []
usedInFlows: []
honestLimits:
  - The mesh and agent layer (9P-over-QUIC, ed25519 identity, capability binds, #agent, and the #kv/#pipe/#plumb/#cas/#cpu device contracts) has no ADRs yet; its design lives in docs/mesh-blueprint.md and the integration docs.
  - ADRs record boundaries and consequences, not implementation status — they intentionally point milestone proofs at tests, examples, and commit messages, so they will never list "what works today."
canonicalCaveatFor: []
---

# ADR Index & Workflow

The active ADRs 0001-0005 plus the ADR-vs-commit-message workflow — review the related ADRs before you touch any boundary.

Wanix keeps exactly five Architecture Decision Records, numbered consecutively, each pinning one durable contract: the runtime boundary, the WASI task boundary, terminals, 9P, and serve. They are deliberately terse and deliberately few. An ADR here records a boundary and its consequences and then points implementation status at tests, examples, and commit messages — so reading one tells you *what is owned where and why*, never *what shipped last Tuesday*. This page is the index, the one-line shape of each record, and the rule for when you add or revise one. Before you change anything at a trust boundary — task identity, fd semantics, the 9P wire, the serve contract — read the ADR that owns it first, and revise that record in the same change. (The canonical index lives in `AGENTS.md`; the five files are in `docs/adrs/`.)

## ADR 0001 — Rust-native Wasmtime runtime

`docs/adrs/0001-rust-native-wasmtime-runtime.md` is the root decision and the broadest. Wanix began as a browser-centered Go and JavaScript runtime; the Rust port rebuilds the contracts that make Wanix useful *outside* Chrome rather than translating the old tree line-by-line. The decision names four ownership rules that everything else inherits:

- **Wanix owns the kernel semantics** — filesystem behavior, namespace binding and resolution, task identity, task service files, fd tables, terminal devices, and externally visible process state.
- **Wasmtime is the guest execution substrate, not the host.** It "is not allowed to inherit host filesystem or process semantics by default."
- **Host files enter only through explicit rooted mounts with escape checks**; ambient host paths are not part of the default namespace. See [host, not ambient, authority](/concepts/host-not-ambient-authority).
- **Go is a semantic oracle, not a layout to copy.** When Rust behavior is ambiguous, Go behavior and tests are the reference; the crate boundaries follow the contracts being rebuilt, not the old package structure.

The same record states the layering rule that the [crate map](/reference/crate-map-and-layering) draws out in full: core filesystem, namespace, terminal, and task crates "must remain free of Wasmtime and QuickJS," and protocol codecs must stay independent of server, CLI, and browser transport policy. ADR 0001 deliberately stays broad — crate inventories and milestones live elsewhere — and tells you to add narrower ADRs only for durable runtime, task-runtime, terminal, protocol, serve, handoff, trust-boundary, or workflow decisions. The other four ADRs are exactly those narrower records.

## ADR 0002 — WASI task runtime boundary (qjs + wasm)

`docs/adrs/0002-quickjs-wasi-task-runtime.md` began as the QuickJS boundary and now governs *any* WASI task runtime; the filename is kept for index stability. The durable line it draws is between **Wanix-owned process state** and the **runtime crate's engine mechanics.** Wanix owns task identity, namespace, cwd, argv/env, stdio, fd tables, service files, terminal attachments, exit state, WASI syscall semantics, and host execution policy — for every WASI runtime. The runtime crate owns instantiation, guest-memory decoding, fixture/module loading, snapshots, and provider plumbing. `qjs` (`.js`) and `wasm` (`.wasm`) are two [task drivers](/concepts/task-drivers) on the Wanix-owned side of that boundary; both auto-start through their driver's `check`/`start` and share one `Namespace`/VFS.

Three contracts in this ADR are load-bearing and worth knowing by name:

- **The fd-mirroring contract.** When a WASI task exposes a guest fd as Wanix-observable state, the adapter mirrors it through the Wanix task fd table and releases it on guest close — and the wasm driver builds its live config through the *same* `wanix-wasi` path as qjs, not a reimplementation. See [the shared WASI fd contract](/concepts/shared-wasi-fd-contract).
- **Snapshots are VM images, not task checkpoints.** A [QuickJS snapshot](/concepts/quickjs-snapshots-are-vm-images) must explicitly reattach Wanix host state on restore; open dynamic descriptors block snapshot.
- **The compiled-artifact cache as a trust boundary.** Cranelift-compiling the bundled QuickJS fixture dominates cold start, so artifacts are cached on disk keyed by `sha256(wasm-bytes)`. The key authenticates the *input* wasm, not the cached bytes, and `deserialize` is `unsafe` — so the cache directory is owner-private (`0o700`), fd-verified with `O_NOFOLLOW`, and a hostile cache is ignored (fresh compile), never deserialized. See [the compiled-artifact cache](/concepts/compiled-artifact-cache).

## ADR 0003 — Terminal device and shell lifecycle

`docs/adrs/0003-terminal-device-and-shell-lifecycle.md` records that Wanix terminals are a service contract, not xterm plumbing. One Rust-native [`#term` device](/devices/term) (the `wanix-term` crate, depending only on `wanix-fs` in production) backs native, served, editor, and VM clients alike. `new` allocates an incrementing resource id; `<id>/data` is the terminal/client side; `<id>/program` is the task side (lone `\n` mapped to `\r\n`); `<id>/winch` broadcasts `columns rows\n` resize payloads; and `<id>/ctl` accepts lifecycle commands — the initial one is `close`, which removes the resource and invalidates handles.

The trust-boundary clause: in raw interactive modes the guest-side shell owns echo, editing, newline handling, Ctrl-C line cancellation, Ctrl-D exit, and command dispatch. Browser, editor, and VM clients may translate key events into bytes like `0x03`/`0x04`, but "those translations are terminal input, not Wanix task cancellation or signal delivery." Disposal writes `close` to `ctl` when the client owns the resource — a cleanup signal, not a process-group signal. See [raw vs. cooked input](/concepts/raw-vs-cooked-input) and the [resize/winch lifecycle](/concepts/resize-winch-lifecycle).

## ADR 0004 — Rust 9P protocol and server contract

`docs/adrs/0004-rust-9p-protocol-and-server-contract.md` splits the wire from the server. `wanix-protocol` owns dependency-free frame splitting, tag extraction, version negotiation, and typed operation codecs (the server-facing 9P2000.L surface plus selected compatibility codecs). `wanix-9p` owns the server state that maps fids to Wanix filesystem objects and translates results into 9P replies and Linux-ish errno errors, leaving listener, socket, stdio, and browser policy to adapters. See [protocol vs. server split](/concepts/protocol-vs-server-split) and [the 9P contract](/concepts/the-9p-contract).

The honesty clause is explicit and worth quoting against overclaim: Rust Wanix "should not fake auth, special-file, extended attribute, ownership, inode-link, or device semantics beyond what the Wanix filesystem contract can actually provide." Unsupported features return deliberate protocol errors. This is why **Tauth stays ENOSYS** — there is no 9P auth handshake — and why the trust boundary is built from capability binds rather than a protocol-level login. See [Tauth is ENOSYS](/concepts/tauth-is-enosys). Stdio, TCP, WebSocket, and `serve` transports are adapters over the one server contract and must preserve binary frame boundaries with diagnostics kept off the binary stream.

## ADR 0005 — Serve and client handoffs

`docs/adrs/0005-serve-and-client-handoffs.md` makes `serve` the local composition surface for browser filesystem, workbench, VS Code, v86, QEMU-launcher, and local-tool clients — without the 9P server or core runtime owning HTTP, browser isolation, or demo-page policy. It reserves the route names: `/.well-known/export9p` for the direct binary 9P WebSocket, `/.well-known/wanix.json` for the [discovery document](/concepts/discovery-document), and `/.well-known/rootfs.json` for the prepared-root handoff. The handoff documents are [loopback-only](/concepts/loopback-only-handoffs): `rootfs.json` returns `wanix-rootfs.v1` "only to loopback clients" because the manifest carries absolute local paths and launch argv.

Service mode exports a namespace containing the served root plus the service devices over direct 9P from a service-root task context — the [browser cockpit](/concepts/browser-cockpit) is a frontend over exactly this. Native QEMU and direct-v86 are validated *command handoffs* over a shared prepared-root shape, with stable manifest kinds (`wanix-rootfs.v1`, `wanix-qemu-virtio9p.v1`); the ADR is firm that this is "not evidence that Wanix has become a VM supervisor or that the browser has become the runtime foundation again." Future auth, remote exposure, Ethernet/vnet, and daemon mode are flagged as separate decisions because they move the serve/client trust boundary. See [serve as a composition surface](/concepts/serve-composition-surface).

## The ADR workflow

Treat ADRs like code. Add or update one *only* for a durable architecture, API, format, trust-boundary, or workflow decision. The active set is intentionally small and consecutive, so prefer revising one of these five over adding a sixth. When you touch a topic, review the related ADRs in the same change and consolidate, delete, or clearly retire records that no longer describe the direction.

The discipline that keeps the set small: **if an ADR draft reads like a good commit message, keep it as the commit message instead.** Milestone proofs, fixture-rebuild notes, per-syscall coverage, CLI/demo slices, and smoke-test progress belong in tests, examples, current-state docs (such as `rust-walkthrough.md`), and commit messages — not in an ADR. Crate-local invariants belong in crate docs (for example `crates/wanix-qjs-engine/docs/architecture.md`); nested crates should not grow a separate Wanix ADR series. The result is a decision set you can read end-to-end in a few minutes, where every record still describes a live boundary.

## See also

- [Crate map & dependency direction](/reference/crate-map-and-layering) — the layering rule from ADR 0001, drawn out across all 22 crates.
- [The 9P contract](/concepts/the-9p-contract) and [protocol vs. server split](/concepts/protocol-vs-server-split) — what ADR 0004 governs.
- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — the execution boundary from ADR 0001.
- [Serve as a composition surface](/concepts/serve-composition-surface) — what ADR 0005 governs.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — the unshipped work the ADRs flag as future decisions.
- [Queued follow-ups](/reference/queued-follow-ups) and [quality gates](/reference/quality-gates) — the engineering backlog and the `just check` gate.

## Status / honest limits

- **The mesh and agent layer has no ADRs yet.** The 9P-over-iroh-QUIC transport, ed25519 identity and default-deny capability binds, the [`#agent` device](/devices/agent), and the [`#kv`](/devices/kv)/[`#pipe`](/devices/pipe)/[`#plumb`](/devices/plumb)/[`#cas`](/devices/cas)/[`#cpu`](/devices/cpu) service contracts are not yet recorded as ADRs. Their design lives in `docs/mesh-blueprint.md`, `docs/mesh-the-missing-half-of-9p.md`, and the cockpit↔mesh integration docs under `docs/integration/`. Promote the durable boundaries (mesh transport + identity, agent-as-device, the service-device contracts) into consecutive ADRs when those contracts stabilize.
- **ADRs are boundaries, not status.** By design they push milestone proofs to tests and commit messages, so no ADR lists "what works today." For current capability, read the capability map in `AGENTS.md` and the reference pages here; for the exact crate graph, read the Cargo manifests, which win over any prose when they disagree.
- **The served exec devices are local-trust only.** Nothing in these five ADRs claims `#task`/`#agent`/`#cpu` are safe for arbitrary untrusted peers; they are not, and there are no hard CPU/memory limits yet. ADR 0005 explicitly lists remote exposure and auth as future, trust-boundary-moving decisions.
