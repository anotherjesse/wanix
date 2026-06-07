---
title: Queued Follow-Ups (Pick a First Cleanup)
slug: reference/queued-follow-ups
pageType: developer
oneLiner: The current backlog — split the over-limit modules, port the v86-shared-demo stub, wire #plumb live recv, add a serve shutdown signal plus connection cap, type the discovery JSON, and grow the per-principal namespace seam.
audience: [developer]
tags: [cli, mesh, caveat, code-quality, contributor]
sourceRefs:
  - AGENTS.md:350-392
  - docs/integration/plan.md:337-373
  - docs/integration/plan.md:465-501
  - workbench/src/web/extension.ts:15
  - crates/wanix-cli/src/serve/concurrent.rs
  - crates/wanix-9p/src/session.rs:31-69
  - crates/wanix-cli/src/serve/discovery.rs:41-43
seeAlso:
  - reference/quality-gates
  - reference/crate-map-and-layering
  - concepts/trust-boundary-gaps
  - concepts/single-frame-serve-caveat
  - concepts/browser-cockpit
  - reference/adr-index
prerequisites:
  - reference/quality-gates
  - reference/crate-map-and-layering
usedInFlows: []
honestLimits:
  - "Each item here is queued with a reason, not abandoned; the reason is usually a missing prerequisite, not lack of value."
  - "The line counts below drift: AGENTS.md records a snapshot (307/283/273) that the source has already grown past (377/342/301 today). Run just module-lines for the live numbers."
  - "The serve connection cap can't land before a shutdown signal exists; doing the cap alone would leak JoinHandles forever."
  - "Live #plumb recv over a single serve connection deadlocks; this is the single-frame-at-a-time caveat, not a bug to patch in place."
---

# Queued Follow-Ups (Pick a First Cleanup)

The current backlog — split the over-limit modules, port the v86-shared-demo stub, wire #plumb live recv, add a serve shutdown signal plus connection cap, type the discovery JSON, and grow the per-principal namespace seam.

This page is the curated to-do list a new contributor should read before picking a first change. It is not a wishlist: every item is queued for a concrete engineering reason — most often a missing prerequisite that would make the obvious fix unsafe or dead code. The canonical source is the **Queued Follow-ups** section of `AGENTS.md:350-392`; this page expands each item, points at the exact file, and says plainly why it is waiting. Read [Quality gates](/reference/quality-gates) and the [crate map](/reference/crate-map-and-layering) first — both are prerequisites for landing any of this cleanly.

## Module-line health: three files over the warn limit

The guardrail is 250 lines (warn) / 350 lines (hard) of non-test code per module, checked by `just module-lines` ([Quality gates](/reference/quality-gates)). `AGENTS.md:354-356` flags three files: `wanix-agent/src/codex.rs`, `wanix-agent/src/exec_server.rs`, and `wanix-cli/src/serve/http/app.rs` — the last one restored from the cockpit work.

Note the drift before you start: the AGENTS.md snapshot reads ~307/~283/~273, but the live files measure **377 / 342 / 301**. The first two have crossed the *hard* 350-line limit since that note was written, so they are no longer "above warn" — they are over budget and `tools/module-line-baseline.txt` (the grandfather registry) is empty, so nothing exempts them. Always run `just module-lines` for the current number rather than trusting the prose. This is a good first cleanup precisely because it is mechanical: split a cohesive helper out of `codex.rs` into a sibling module, keep the public surface identical, and watch the gate go green.

## Cockpit follow-ups

Three threads, all in the [browser cockpit](/concepts/browser-cockpit) (`workbench/`).

**The v86-shared-demo is a stub.** `workbench/src/web/extension.ts:15` carries a single line — `// STUB: v86-shared-demo not yet ported — see docs/integration/plan.md` — and the surrounding code only arms a browser-side watcher and reports synthetic "v86 shared file changed" activity. The open question in `docs/integration/plan.md:472-478` is which device backs it: a flat 9P-shared directory, or a `#cas` + `#plumb` cross-VM exchange. The plan's default is to fold it into the CAS+plumb story unless a real v86 boot scenario needs the flat-shared pattern.

**`#plumb` live receive only probes the publish path.** The cockpit self-check writes to `#plumb/<topic>/send` and confirms the publish succeeds, but it cannot block on `#plumb/<topic>/recv` to verify end-to-end delivery. The reason is structural, not lazy: `serve` handles one 9P frame at a time per connection, so a blocking `recv` would never yield the connection back for the matching `send` write — it would deadlock. This is the [single-frame serve caveat](/concepts/single-frame-serve-caveat) in the flesh. The fix is a second 9P connection (or concurrent frame handling), not a patch to the recv call.

**Slice 8: rename `--bundle workbench-fs9p` to `--bundle cockpit`.** `docs/integration/plan.md:337-373` describes making the cockpit the default served bundle, keeping `workbench-fs9p` as a backward-compat alias, and retiring the `workbench/code/` vscode-web vendor dependency (the ~80 MB VS Code download) by loading `workbench/dist/web/extension.js` directly.

## Serve concurrency: shutdown signal + connection cap, together

`crates/wanix-cli/src/serve/concurrent.rs` spawns a detached worker thread per accepted connection with no cap, and in unbounded mode busy-polls `accept()` on a fixed sleep. The obvious fix — a connection cap with thread accounting — cannot land alone, and `AGENTS.md:365-371` is explicit about why: there is **no shutdown signal anywhere in `serve/` today**. Without one, a cap is untestable (you can't deterministically drain workers to assert the limit held), and retaining every `JoinHandle` to count threads would leak handles forever, since nothing ever signals a worker to stop.

So this is one cycle, not two: introduce a shutdown signal first, then build the cap on top of it. Do it before HTTP workers, remote serve, or multi-user land, because all three multiply the connection count this guards.

## Typed discovery / handoff JSON

The discovery document and the rootfs / qemu / direct-v86 handoffs are still hand-built `format!` strings — `crates/wanix-cli/src/serve/discovery.rs:41-43` shows the pattern (`format!("ws://{host}/.well-known/export9p")` and friends). The driver-list drift is already fixed: discovery now derives the driver list from the task registry rather than a hard-coded literal. The remaining cleanup is to convert the rest into typed structs with shape-pinning tests, so the JSON the cockpit parses cannot silently change shape. `docs/integration/plan.md:484-489` raises the urgency: several cockpit slices extend `discovery.rs`, and the plan asks for the typed cleanup as a prep step rather than accreting more `format!` JSON across slices.

## The 9P session / namespace seam

`AGENTS.md:376-380` describes `handle_attach` as decoding `uname`/`aname` and discarding both, with every fid resolving through one shared `P9Server.root`. That snapshot has partly aged out: `crates/wanix-9p/src/session.rs:31-69` now evaluates `aname` against an `AttachPolicy` (`install_attach_root` calls `policy.evaluate(context.peer, aname)` and installs a scoped root on a match, returning `EACCES` on denial — this is the mesh's [capability-is-a-bind](/concepts/capability-is-a-bind) gate). What remains true is that **`uname` is still ignored**, and the root is per-session, not per-fid.

The remaining work is a per-principal `NamespaceProvider` plus per-fid root storage. The guardrail: do not add the trait as a no-op seam. A `NamespaceProvider` whose result is discarded is a dead abstraction, so it only lands cleanly alongside per-fid root storage **and a first real consumer** — an HTTP worker or genuine per-user namespaces. This connects to the broader [trust boundary gaps](/concepts/trust-boundary-gaps): per-principal namespaces and grant lifecycle are the headline unshipped trust work.

## How to pick

Lowest-friction first change: split one over-limit module (no behaviour change, immediate green gate). Highest-leverage with a clear scope: the serve shutdown signal, because it unblocks the connection cap and every later networking feature. Most coupled to other work: the discovery typing and the namespace seam — both want a concrete downstream consumer to land against, so check the [cockpit integration plan](/concepts/browser-cockpit) before starting either.

## See also

- [Quality gates](/reference/quality-gates) — the `just module-lines` and `just check` gates these items are measured against.
- [Crate map & dependency direction](/reference/crate-map-and-layering) — where each touched file lives in the layering.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — the per-principal namespace and grant-lifecycle work the seam item feeds.
- [Single-frame serve caveat](/concepts/single-frame-serve-caveat) — why live `#plumb` recv deadlocks on one connection.
- [Browser cockpit](/concepts/browser-cockpit) — the operator surface the v86 stub and Slice 8 belong to.
- [ADR index](/reference/adr-index) — the active decision set these cleanups must stay consistent with.

## Status / honest limits

Every item on this list is queued with a reason, not abandoned, and the reason is usually a missing prerequisite rather than missing value:

- **The line counts drift.** AGENTS.md records 307/283/273; the live source measures 377/342/301. Two of the three have crossed the hard 350-line limit since the note was written. Run `just module-lines` for ground truth; never quote the prose figure.
- **The serve cap cannot precede the shutdown signal.** With no shutdown path in `serve/`, a connection cap is untestable and unconditional `JoinHandle` retention leaks handles. Land the signal and the cap as one cycle.
- **Live `#plumb` recv genuinely deadlocks** on a single serve connection because `serve` processes one 9P frame at a time. The cockpit probe checking only the publish path is correct given that constraint; the resolution is a second connection or concurrent frame handling, not a code patch to the recv site.
- **The namespace seam is half-shipped.** `aname`-scoped attach already gates the mesh; `uname` is still discarded and roots are per-session. A `NamespaceProvider` trait should not be added as a no-op seam — it needs per-fid storage and a first consumer or it is dead code.
