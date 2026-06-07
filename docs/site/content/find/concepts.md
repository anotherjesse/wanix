---
title: Concept index
slug: find/concepts
pageType: overview
oneLiner: Every concept page, grouped by cluster and listed A-Z — the guaranteed inbound link for every graph node.
audience: [newcomer, developer, visionary]
tags: [find, index, navigation, concept-graph]
sourceRefs: [docs/site/02-information-architecture.md]
seeAlso: [find/index, find/glossary, find/tags, concepts/everything-is-a-file, concepts/missing-half-of-9p, concepts/service-devices]
prerequisites: []
usedInFlows: []
honestLimits: ["This is a navigation index, not a tutorial; each concept page is the canonical home of its facts and caveats.", "The clustered visual map is a client-side widget; the A-Z list below is the link-complete fallback."]
canonicalCaveatFor: []
---

# Concept index

Every concept page, grouped by cluster and listed A-Z — the guaranteed inbound link for every graph node.

A concept is one idea explained once, completely, in its own canonical page. This index is the flat door to all of them: scan a cluster to learn an area top-down, or jump straight to a node by name. Concepts own the truth; flows under [/learn/index](/learn/index) only sequence them. If you arrived by search and want a definition rather than a page, the [glossary](/find/glossary) is terser; if you want to filter by status (`shipped`, `local-trust-only`, `exploratory`, `caveat`), use the [tag browser](/find/tags).

## Core: files, namespaces, tasks

The root is [everything is a file](/concepts/everything-is-a-file), which composes [the FileSystem trait](/concepts/the-filesystem-trait) over [NormalizedPath](/concepts/normalizedpath). It is prerequisite to [service devices](/concepts/service-devices) and to [per-process namespaces](/concepts/per-process-namespaces), which in turn is built from [namespace binding](/concepts/namespace-binding) and [namespace resolution](/concepts/namespace-resolution). On top of namespaces sit [tasks own process identity](/concepts/tasks-own-process-identity), [the fd table](/concepts/the-fd-table), and [task drivers](/concepts/task-drivers).

## Runtimes: Wasmtime, qjs, and wasm

[Wasmtime as substrate](/concepts/wasmtime-as-substrate) runs both tiers, fed by [Wanix-backed WASI](/concepts/wanix-backed-wasi) and the [shared WASI/fd contract](/concepts/shared-wasi-fd-contract). The interpreted tier is the [qjs task](/concepts/qjs-task); the compiled tier is the [compiled wasm task driver](/concepts/compiled-wasm-task-driver) over a [command-style wasm linker](/concepts/command-style-wasm-linker). They meet at [two tiers, one substrate](/concepts/two-tiers-one-substrate), share the [compiled-artifact cache](/concepts/compiled-artifact-cache), and are governed by [Wasmtime guest WASI as host-not-ambient authority](/concepts/host-not-ambient-authority), [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images), [bounded execution policy](/concepts/bounded-execution-policy), and [guest JS guardrails](/concepts/guest-js-guardrails).

## Terminals and shells

[The #term device](/devices/term) is prerequisite to [qjs-shell](/concepts/qjs-shell), which composes [raw vs cooked input](/concepts/raw-vs-cooked-input) and the [resize/winch lifecycle](/concepts/resize-winch-lifecycle).

## Mesh: 9P, import, and transport

[The 9P contract](/concepts/the-9p-contract) splits into [protocol vs server](/concepts/protocol-vs-server-split) and yields [RemoteFs, the import half](/concepts/remotefs-import-half) — [the missing half of 9P](/concepts/missing-half-of-9p) realized as [import/export and /n/](/concepts/import-export-and-n). The transport is [9P over iroh QUIC](/concepts/9p-over-iroh-quic), built from the [async/sync bridge](/concepts/async-sync-bridge), the [streaming import filesystem](/concepts/streaming-import-fs), and the [five hostile-peer corrections](/concepts/five-hostile-peer-corrections). Because every device is a filesystem, [devices import for free](/concepts/devices-import-for-free) across [one identity, two planes](/concepts/one-identity-two-planes), with the [content-addressed data plane](/concepts/content-addressed-data-plane) carrying blobs.

## Trust boundary

[The key is the address](/concepts/key-is-the-address) rests on a [persisted ed25519 identity](/concepts/persisted-ed25519-identity); the QUIC handshake authenticates peers, so [Tauth is ENOSYS](/concepts/tauth-is-enosys). [A capability is a bind](/concepts/capability-is-a-bind) — a re-rooted [SubtreeFs that confines to a prefix](/concepts/subtreefs-confine-to-prefix), evaluated by the default-deny [attach policy](/concepts/attach-policy). The honest edges live at [safe-for-untrusted is not claimable](/concepts/safe-for-untrusted-not-claimable), [rooms not houses](/concepts/rooms-not-houses), and [trust-boundary gaps](/concepts/trust-boundary-gaps).

## Devices and their concepts

[#kv](/devices/kv) — [the smallest database](/concepts/kv-smallest-database); [#pipe](/devices/pipe) and [#plumb](/devices/plumb) — [best-effort epidemic delivery](/concepts/best-effort-epidemic-delivery) and the [blocking-stream EOF contract](/concepts/blocking-stream-eof-contract); [#cas](/devices/cas) — [end-to-end hash verification](/concepts/end-to-end-hash-verification) and the [wanix capsule](/concepts/wanix-capsule); [#cpu](/devices/cpu) — [send the agent to the data](/concepts/send-agent-to-the-data); [#agent](/devices/agent) — [approvals as files](/concepts/approvals-as-files), [agents as operators](/concepts/agents-as-operators), and [FakeEngine vs codex](/concepts/fakeengine-vs-codex).

## Serve, cockpit, and philosophy

[Wanix as host](/concepts/wanix-as-host) (not browser-as-host) is the [serve composition surface](/concepts/serve-composition-surface), composed of the [discovery document](/concepts/discovery-document), the [--wanix-services device set](/concepts/wanix-services-device-set), [three serve bundles](/concepts/three-serve-bundles), and the [/.wanix/app/<name> HTTP route](/concepts/http-app-route). The [browser cockpit](/concepts/browser-cockpit) is a [direct-9P operator surface](/concepts/direct-9p-operator-surface) that distinguishes [live-stream vs one-shot access](/concepts/live-stream-vs-one-shot) against the [single-frame serve caveat](/concepts/single-frame-serve-caveat) and [loopback-only handoffs](/concepts/loopback-only-handoffs). The provenance vision is [traceable namespaces](/concepts/traceable-namespaces), and the unifying root is [everything is a file](/concepts/everything-is-a-file).

## The visual concept map

This index has a sibling view: a client-side graph widget that renders the same nodes as a map, colored by audience (newcomer, developer, visionary), with `prerequisite-of`, `composed-of`, and `related-to` edges drawn between them. Hover a node to see its in- and out-edges; click to open its canonical page. The map is the spatial door; the A-Z list above is the link-complete one. Both reach the identical set of pages, so the index stays usable even where the widget does not load.

## See also

- [Find hub](/find/index) — full-text typed search, hotkey `/`.
- [Glossary](/find/glossary) — terse A-Z definitions of every term.
- [Tags](/find/tags) — filter by audience, cluster, and status.
- [Devices index](/devices/index) — the device-contract reference.
- Roots worth starting from: [everything is a file](/concepts/everything-is-a-file) · [service devices](/concepts/service-devices) · [the missing half of 9P](/concepts/missing-half-of-9p).

## Status / honest limits

- This page is navigation, not instruction. Every behavioural claim — what `#kv` persists, whether the served `#agent` is a live model, how `serve` frames traffic — lives on the linked concept page, which is the single home of that fact and its caveat. Do not treat the one-line cluster summaries here as the contract.
- The clustered visual map is a client-side widget. The A-Z cluster list above is the link-complete fallback and always reflects the full concept set.
