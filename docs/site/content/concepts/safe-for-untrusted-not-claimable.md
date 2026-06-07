---
title: Safe-for-Untrusted-Code Is Not Claimable Yet
slug: concepts/safe-for-untrusted-not-claimable
pageType: concept
oneLiner: Per-room memory isolation is real today, but Wasmtime CPU/memory hard limits (epoch/fuel preemption, linear-memory caps) are not wired up — so do not claim "safe for arbitrary untrusted code."
audience: [developer, visionary]
tags: [runtime, substrate, trust-boundary, caveat, local-trust-only]
sourceRefs:
  - docs/scaling-eli5.md:89-95
  - performance.md:287-289
  - crates/wanix-qjs/src/driver.rs:54-102
seeAlso:
  - concepts/rooms-not-houses
  - concepts/bounded-execution-policy
  - concepts/wasmtime-as-substrate
  - concepts/trust-boundary-gaps
prerequisites:
  - concepts/rooms-not-houses
usedInFlows: []
honestLimits:
  - Per-room (per-instance) memory isolation is real; hard CPU/memory limits are not wired.
  - Wasmtime epoch/fuel preemption and ResourceLimiter/StoreLimits linear-memory caps are unbuilt; today's limits are QuickJS-level only.
  - Exec devices (#task, #agent, #cpu) are local-trust only and not exposed to untrusted or public peers.
  - The honest public claim is "cheap, scalable isolation," not "safe for arbitrary untrusted code."
canonicalCaveatFor: [safe-for-untrusted]
---

# Safe-for-Untrusted-Code Is Not Claimable Yet

Per-room memory isolation is real today, but Wasmtime CPU/memory hard limits (epoch/fuel preemption, linear-memory caps) are not wired up — so do not claim "safe for arbitrary untrusted code."

This page is the single honest boundary for the whole scaling-and-agents story. Wanix runs thousands of programs cheaply, each in its own WebAssembly sandbox — that part is true and load-bearing. What is *not* yet true is the harder claim people reach for next: that you can drop arbitrary, hostile code into one of those sandboxes and trust the room to contain it under load. The memory walls are real; the hard CPU and memory *limits* that would make a room safe for code you do not trust are not built. Until they are, the correct framing is "cheap, scalable isolation," not "safe for arbitrary untrusted code."

## What IS true: the wasm sandbox boundary

Each task — an interpreted `qjs` script or a compiled `wasm32-wasi` program — runs as a WebAssembly instance, ~0.25 MB instead of a ~10 MB OS process. A wasm instance cannot read another instance's linear memory; that is the WebAssembly memory-safety guarantee, enforced by the substrate, not by Wanix politeness. And Wanix hands each guest *no* ambient host authority: a guest sees only the files its namespace binds, so a task with no capability to a peer's `#kv` simply cannot reach it. The room model from [rooms, not houses](/concepts/rooms-not-houses) is honest about *memory* and *reach*. A bad neighbor cannot read your bytes or open files you did not grant.

## What is NOT wired: hard CPU/memory preemption

The gap is *resource* containment under hostile load. Two specific Wasmtime mechanisms are absent:

- **Epoch / fuel preemption** — hard CPU interruption that stops a guest after a budget regardless of what it is doing. Without it, a `while(true){}` owns its host thread.
- **`ResourceLimiter` / `StoreLimits`** — hard linear-memory caps enforced by the substrate as the guest grows its heap.

What exists today is *soft, in-engine* policy, and only on the qjs path. The qjs driver carries an `interrupt_poll_budget` and a `memory_limit_bytes` (`crates/wanix-qjs/src/driver.rs:54-73`), threaded into the run (`crates/wanix-qjs/src/driver.rs:87-102`). But the interrupt handler only fires when QuickJS *chooses* to poll it, and the memory limit is QuickJS's own heap accounting — not a Wasmtime store cap. As `performance.md:287-289` states plainly: before running untrusted code in-process, wire Wasmtime epoch/fuel and a `ResourceLimiter`/`StoreLimits`; "today's limits are QuickJS-level only." See [bounded execution policy](/concepts/bounded-execution-policy) for the four knobs and exactly why they are not a scheduler. Wasmtime is where these caps will eventually live — see [Wasmtime as substrate](/concepts/wasmtime-as-substrate).

## Why this matters for the agent / untrusted-code story

This is the reason the exec devices are gated. `#task`, `#agent`, and `#cpu` run guest code, and a node will not expose them to untrusted or public peers — they are **local-trust only**. You can send an agent to a peer's files over `#cpu`, but the peer is choosing to run *your* compute on *its* CPU with no hard ceiling on how much it consumes. That is fine between machines you both control; it is not fine against a hostile caller. The capability binds of [attach policy](/concepts/attach-policy) control *what files* a peer can reach, not *how much CPU or memory* its code may burn once it is running. Those are different boundaries, and only the first one is enforced.

## The correct public copy

State the claim at its true strength. "Cheap, scalable isolation": thousands of memory-isolated rooms, scaling with your cores, each with no ambient authority. Do **not** upgrade that to "safe for arbitrary untrusted code," "a secure multi-tenant sandbox," or "run code you do not trust." The team note in `docs/scaling-eli5.md:89-95` is explicit: per-room memory isolation is real today; the hard CPU and memory limits are not wired up, so keep public copy to "cheap, scalable isolation."

## See also

- [Rooms, not houses](/concepts/rooms-not-houses) — the isolation model this caveat constrains.
- [Bounded execution policy](/concepts/bounded-execution-policy) — the four soft qjs knobs and why they are not a scheduler.
- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — where epoch/fuel and linear-memory caps will land.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — the full list of unbuilt trust-boundary work.

## Status / honest limits

- Per-room (per-instance) **memory** isolation is real and substrate-enforced; guests carry no ambient host authority.
- Hard CPU/memory preemption is **not built**: no Wasmtime epoch/fuel interruption and no `ResourceLimiter`/`StoreLimits` linear-memory caps (`performance.md:287-289`). Today's limits are QuickJS-level only and apply on the qjs path alone (`crates/wanix-qjs/src/driver.rs:54-73`).
- Exec devices (`#task`, `#agent`, `#cpu`) are **local-trust only**, not exposed to untrusted or public peers.
- The honest claim is **"cheap, scalable isolation,"** not "safe for arbitrary untrusted code" (`docs/scaling-eli5.md:89-95`).
