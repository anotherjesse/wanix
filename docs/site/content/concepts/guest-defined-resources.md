---
title: Guest-Defined Resources
slug: concepts/guest-defined-resources
pageType: concept
oneLiner: An app IS the resource — a small guest program decides what its files mean, one JSON event at a time, while the host owns identity, concurrency, streams, and presence (the file2chan split).
audience: [developer, visionary]
tags: [apps, mesh, appfs, qjs, shipped, caveat]
sourceRefs:
  - docs/appfs.md
  - crates/wanix-appfs/src/lib.rs:1-40
  - crates/wanix-appfs/src/service.rs:63-99
  - crates/wanix-fs/src/buffer.rs:27-30
  - crates/wanix-cli/src/app/serve.rs:185-202
  - examples/chatroom/main.js
  - examples/chatroom/app.wanix.json
seeAlso:
  - concepts/everything-is-a-file
  - concepts/jobs-are-files
  - concepts/service-devices
  - concepts/key-is-the-address
  - learn/build-a-chatroom
  - recipes/07-chatroom-over-the-mesh
prerequisites:
  - concepts/everything-is-a-file
usedInFlows:
  - {flow: build-a-chatroom, step: 1}
honestLimits:
  - "v0 runs qjs guests only (rust-wasm is a later adapter), and there is no auto-restart: when the guest exits, discrete ops fail Unreachable and blocked stream readers are released with EOF."
  - "CAS-pinned guest code (`cas:` main) is refused with a message naming docs/appfs.md §Provenance as deferred — running identity is not yet cryptographically pinned."
  - "Stream subscriptions are bounded and lossy by design (1 MiB, drop-oldest): a slow reader loses the oldest backlog rather than blocking the app."
canonicalCaveatFor: []
---

# Guest-Defined Resources

An app **is** the resource: a small guest program decides what its files mean, one JSON event at a time, while the host owns identity, concurrency, streams, and presence — the file2chan split (`docs/appfs.md`, `crates/wanix-appfs`).

Wanix already had two ways to put behavior behind a mounted path. A *service device* (`#kv`, `#pipe`, `#plumb`) is a host-implemented `FileSystem` — fixed semantics, written in Rust. A *tool* ([jobs are files](/concepts/jobs-are-files)) is one host-approved operation wrapped in the job protocol — the host chooses the executable, the caller only supplies data. A **guest-defined resource** inverts the second half: the *host* chooses authority, identity, limits, and lifecycle, and the *guest app* chooses behavior. A chatroom, a queue, a game session — anything whose meaning is application logic — becomes a mounted filesystem whose semantics are a few dozen lines of JavaScript.

The mechanism is Inferno's `file2chan` idea rebuilt on the Wanix substrate (`wanix-appfs`): filesystem requests arrive at the guest as messages.

- **Discrete ops go to the guest, one at a time.** A `read`/`write`/`stat` on a declared guest file becomes one newline-JSON event on the guest's stdin; the guest replies one line on stdout. The adapter serializes request/reply under one lock (`service.rs`), so the guest is a single actor that never sees concurrency.
- **Streams never touch the guest.** Paths declared as streams are host-owned: each open registers a bounded, lossy, never-EOF subscription buffer (the `#plumb` `LineBuffer` discipline). The guest *feeds* a stream by emitting `{"publish":...}` lines; the host fans them out. A wedged guest cannot block a reader, and a slow reader cannot block the app — this is the byte-pump trap `docs/appfs.md` warns about, avoided structurally.
- **Presence is host session state.** The reserved `who` file lists the principals currently holding open stream subscriptions. The guest is not consulted.
- **Identity rides the transport.** The serve edge binds each connection's verified peer key into a principal-scoped view (`AppAttachPolicy`, `crates/wanix-cli/src/app/serve.rs`) — the ToolFS pattern — and the adapter stamps that principal into every guest event. A payload claiming to be someone else changes nothing.
- **Durability is an explicit mount.** Guest memory is a cache; the room's history lives in the `--state` directory the host mounts at `/state`. Kill the serve, restart it, and the resource still remembers.

The slogan from the design note holds as shipped: *guest decides; host moves; namespace grants; transport identifies.* And because the result is a plain `FileSystem`, it [imports across the mesh for free](/concepts/devices-import-for-free) — one `iroh://` ticket names one running app.

The worked proof is the chatroom: a ~120-line qjs program (`examples/chatroom/main.js`) plus a 8-line manifest (`app.wanix.json`) served by `wanix-rust app serve`. See the [build-a-chatroom flow](/learn/build-a-chatroom) and [Recipe 07](/recipes/07-chatroom-over-the-mesh).

## Status / honest limits

- v0 is qjs-only and has **no auto-restart**: a dead guest fails discrete ops with `Unreachable` ("resource down, not missing") and releases blocked stream readers with EOF.
- Code provenance is deferred: `cas:`-pinned `main` is refused, naming `docs/appfs.md` §Provenance.
- Streams are bounded and lossy (drop-oldest at 1 MiB) — the explicit slow-reader policy, not an accident.
- Every ticket holder may attach (an open room); allow-lists are the ADR 0007 Layer 2 follow-up.
