---
title: The Browser Cockpit (Operator Surface)
slug: concepts/browser-cockpit
pageType: concept
oneLiner: A VS Code web extension that browses the namespace and drives every service device over direct 9P — inspect, agent-repair, duet, HTTP apps, self-check — as the human operator surface for the Rust runtime.
audience: [visionary, developer]
tags: [shipped, cli, mesh, caveat, browser, cockpit]
sourceRefs:
  - workbench/src/wanix/p9.ts:48-471
  - workbench/src/web/system-view.ts:189-253
  - workbench/src/web/system-view.ts:957-993
  - workbench/src/web/service-inspector.ts:201-387
  - docs/integration/STATUS.md:432-475
seeAlso:
  - concepts/direct-9p-operator-surface
  - concepts/live-stream-vs-one-shot
  - concepts/http-app-route
  - concepts/wanix-as-host
  - concepts/trust-boundary-gaps
prerequisites:
  - concepts/three-serve-bundles
  - concepts/service-devices
usedInFlows: []
honestLimits:
  - The mesh panel (peers, /n/<node> mounts) and browser-as-peer over WebSocket are designed but not yet shipped.
  - The v86 shared-files demo is still a no-op stub in the extension.
  - The self-check probes the #plumb publish path only; a blocking recv cannot interleave with a write on serve's single-frame-at-a-time connection.
  - Earlier STATUS.md FAIL passes are superseded by the working pass; the render-blocking bugs were fixed.
---

# The Browser Cockpit (Operator Surface)

A VS Code web extension that browses the namespace and drives every service device over direct 9P — inspect, agent-repair, duet, HTTP apps, self-check — as the human operator surface for the Rust runtime.

Launch the workbench bundle with services on, open the **Wanix** activity-bar entry, and a tree called **WANIX: SYSTEM** populates itself from the live runtime: Tasks reads "1 shell — running · #task" straight out of the `#task` device, the Explorer shows the served root over 9P, and the Terminal hosts a real qjs-shell (`docs/integration/STATUS.md:336-410`). Nothing in that tree is a mock. Every row is the result of a 9P read or write against the same server a CLI client would talk to. The cockpit is not a separate control protocol bolted onto Wanix; it is one more 9P client that happens to be a human-facing editor.

## What you see, then what it is

Start it the way the demo does:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust serve --root /tmp/world --bundle workbench-fs9p --wanix-services --addr 127.0.0.1:18021
```

Open the served URL, click the **Wanix** icon, and the activity-bar view fills in. The categories are fixed in `system-view.ts` — Actions, Tour, Reports, Data Stores, Checks, Activity, Routes, Route Runs, Agent, Tasks, Terminals, Namespace, Drivers (`workbench/src/web/system-view.ts:173-187`). The Tasks and Terminals rows are populated by reading the `#task` and `#term` service directories; a task row marked `· #task` was observed on the device, not invented by the extension (`workbench/src/web/system-view.ts:1043-1108`). That is the Plan 9 idea showing through the editor chrome: the operator surface inspects files, and the runtime's own service files are the single source of truth.

## Direct 9P, no bridge

Every operation the cockpit performs is a 9P exchange through one class, `WanixP9Handle` (`workbench/src/wanix/p9.ts:48`). `readFile` walks to the path, opens it `O_RDONLY`, reads 64 KiB chunks to EOF, and clunks the fid (`workbench/src/wanix/p9.ts:143-173`). `writeFile` tries to truncate-and-write an existing file, falling back to a `Tlcreate` in the parent (`workbench/src/wanix/p9.ts:229-248`). `readDir` opens the directory and pages `readdir` responses until the offset stops advancing (`workbench/src/wanix/p9.ts:88-111`). These are the four verbs from [everything is a file](/concepts/everything-is-a-file) — open, read, write, list — spoken on the wire.

There is no MessagePort or CBOR bridge in this path. The runtime guardrails are explicit that Rust-served workbench paths pass a discovered direct-9P route into the extension, and the MessagePort/CBOR bridge is browser-embedded compatibility only. `WanixP9Handle.fromDiscovery` fetches `/.well-known/wanix.json`, reads `routes.p9`, and connects the WebSocket, preferring the Google.2 protocol when the route advertises it (`workbench/src/wanix/p9.ts:62-81`, `443-448`). One handle, one wire format, every capability. See [direct-9p operator surface](/concepts/direct-9p-operator-surface).

## The service inspector and its file taxonomy

The Namespace tree lists every bound service device, sourced from discovery's `services.devices` (itself derived from `roots::INSPECTABLE_SERVICE_DEVICES`). Opening one renders the live directory under the `wanix-inspect:` scheme (`workbench/src/web/service-inspector.ts:4`). The inspector is deliberately cautious: it lists paths but does not read files that allocate resources, because reading the wrong service file has side effects.

`unsafeFileReason` is that safety classifier (`workbench/src/web/service-inspector.ts:201-387`). It sorts every service path into four kinds:

- **Allocator** — reading it mints a resource: `#agent/new`, `#pipe/new`, `#cas/ingest`. Linking these would silently spawn sessions, channels, or blob handles, so they stay plain text with a reason.
- **Metadata** — safe one-shot snapshots: `id`, `status`, `kind`, `exit`, `cmd`, `dir`, `env`, `pending`. These become clickable document links.
- **Control** — write-only verbs (`ctl`). A read open is meaningless, so it stays plain.
- **Streams** — blocking or live I/O: `events`, `data`, `prompt`, `reply`, `send`, `recv`, `program`, `winch`. Opening one can consume live data or block forever, so it is left plain with an explicit reason.

For an unrecognized device — `#mesh`, `#cpu`, or some future name — the classifier falls back to safe-by-default: it links the universally safe metadata names and leaves everything else plain (`workbench/src/web/service-inspector.ts:382-387`). The taxonomy is the cockpit's honest model of [service devices](/concepts/service-devices): not every file is a value to peek at; some are verbs.

## Five live-wired features

The Actions category drives the runtime, and five of its features actually exercise the mesh/agent devices over 9P (`docs/integration/STATUS.md:432-475`, `workbench/src/web/system-view.ts:957-993`):

1. **Inspect** — the service inspector above, rendering `#kv`, `#agent`, `#cas`, and the rest over 9P with their allocator/metadata/stream annotations.
2. **Agent repair** — opens a `#agent` session, submits a patch plan as an `approve:` prompt, polls `pending`, resolves the approval through `ctl`, then applies the patch. One click fixes a broken script and the report records the session and approval. The served `#agent` here is a deterministic `FakeEngine`, not a live model.
3. **Duet** — a qjs producer feeds a Rust `.wasm` transform that feeds a qjs verifier, all on one shared filesystem, proving the compiled-vs-interpreted tier on one substrate.
4. **HTTP apps** — the loopback-only, services-gated `/.wanix/app/<name>` route; the counter demo is backed by `#kv/http-counter`, so curl and the cockpit increment one shared counter. See [the HTTP app route](/concepts/http-app-route).
5. **Self-check** — probes drivers, services, the shell, report storage, and `#agent`/`#kv`/`#pipe`/`#cas`/`#plumb`, writing `/.wanix/cockpit-check.{md,json}`.

## Live stream vs one-shot

The split that makes all of this work over 9P is `isLiveServiceStream` (`workbench/src/wanix/p9.ts:456-471`). One-shot helpers (`readFile`/`writeFile`) issue a create and drain to EOF — correct for `#kv` keys and ordinary files, wrong for a terminal or a subscription. So a small set of paths takes the streaming path instead: `#term/*`, `#pipe/<id>/data`, `#plumb/<topic>/send` and `/recv`, and `#agent/<id>/events`. For those, the cockpit walks to the existing fid, opens it, and reads or writes at offset 0 (`workbench/src/wanix/p9.ts:388-439`). The one-shot create would be rejected by these allocator-owned files, and the one-shot read would block forever on a live subscription. Writable streams open `O_WRONLY` because `#pipe` and `#plumb` ends are strictly unidirectional and reject `O_RDWR`, while `#term` tolerates a write-only open (`workbench/src/wanix/p9.ts:420-426`). See [live stream vs one-shot](/concepts/live-stream-vs-one-shot).

## See also

- [Direct 9P operator surface](/concepts/direct-9p-operator-surface) — the no-bridge contract the cockpit rests on.
- [Live stream vs one-shot](/concepts/live-stream-vs-one-shot) — why `#term`/`#pipe`/`#plumb`/`#agent` reads take a different 9P path.
- [The HTTP app route](/concepts/http-app-route) — `/.wanix/app/<name>`, loopback-only, `#kv`-backed.
- [Service devices](/concepts/service-devices) — the `#`-named catalog the inspector classifies.
- [Three serve bundles](/concepts/three-serve-bundles) — `fs9p`, `workbench-fs9p`, `direct-v86`.
- [Wanix as host](/concepts/wanix-as-host) — the runtime the cockpit operates.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — what is not yet safe to expose.
- Recipe: [repair a broken qjs program](/recipes/01-repair-broken-qjs) — the agent-repair feature end to end.

## Status / honest limits

- **The served `#agent` is a `FakeEngine`, not a live LLM.** The agent-repair demo drives a deterministic engine over `#agent`; the real codex engine is the local-trust `wanix agent` CLI path only.
- **`#kv` is in-memory.** The HTTP counter's `#kv/http-counter` lives only as long as the serve process; freeze a world to a capsule to persist it.
- **`#plumb` recv is publish-only in the self-check.** serve handles one 9P frame at a time per connection, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection; end-to-end delivery would need a second connection or concurrent frame handling.
- **The mesh panel and browser-as-peer are queued.** Peers, `/n/<node>` mounts, and a WebSocket peer session are designed but not yet shipped; the cockpit today operates a single served node.
- **The v86 shared-files demo is a stub.** Its row appears in the sidebar but the action is currently a no-op in the extension.
- **Earlier STATUS passes are superseded.** The early FAIL sections recorded a missing static-asset route and a stubbed system-view; both were fixed, and the cockpit renders and drives the devices in the working pass.
