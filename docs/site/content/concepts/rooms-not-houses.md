---
title: Rooms, Not Houses (Cheap Scalable Isolation)
slug: concepts/rooms-not-houses
pageType: concept
oneLiner: Each program runs in a ~0.25 MB WebAssembly sandbox instead of a ~10 MB OS process — roughly 40x smaller — so thousands of isolated programs run cheaply, scaling with your cores.
audience: [newcomer, developer, visionary]
tags: [runtime, substrate, wasmtime, isolation, scaling, shipped, caveat, local-trust-only]
sourceRefs:
  - docs/scaling-eli5.md:9-66
  - docs/scaling-eli5.md:89-95
seeAlso:
  - concepts/wasmtime-as-substrate
  - concepts/two-tiers-one-substrate
  - concepts/safe-for-untrusted-not-claimable
  - concepts/bounded-execution-policy
prerequisites:
  - concepts/wasmtime-as-substrate
  - concepts/two-tiers-one-substrate
usedInFlows: []
honestLimits:
  - Per-room (per-instance) memory isolation is real today; hard CPU and memory limits (Wasmtime epoch/fuel preemption, linear-memory caps) are NOT wired up yet.
  - The honest public claim is "cheap, scalable isolation," not "safe for arbitrary untrusted code." Exec devices (#task, #cpu, #agent) are local-trust only.
  - The throughput and size numbers are from one 18-core dev machine and vary by hardware.
canonicalCaveatFor: []
---

# Rooms, Not Houses (Cheap Scalable Isolation)

Each program runs in a ~0.25 MB WebAssembly sandbox instead of a ~10 MB OS process — roughly 40x smaller — so thousands of isolated programs run cheaply, scaling with your cores.

The usual way to run many programs safely is to give each one its own operating-system process. Processes are isolated, but they are expensive: a thousand of them is gigabytes of overhead before any of them does work. Wanix takes the other path. It runs each program in a tiny WebAssembly sandbox on a shared Wasmtime substrate, so the same thousand programs fit in a few hundred megabytes. This page is the mental model — and the honest boundary on what that isolation does and does not yet buy you.

## A locked room inside a shared building

The image is a building full of locked rooms (`docs/scaling-eli5.md:9-23`). The old way is one house per program: a separate operating-system process. Houses are safe — nobody walks into yours — but each one costs roughly **10 MB** of memory just to exist, before it runs a line of code. A thousand small programs is therefore about **10 GB**, most of it spent on empty houses.

Wanix runs each program in a WebAssembly instance instead — a locked room inside one shared building. Each room is private: code in one room cannot see or touch another, and it can only use the doors — files, network — you explicitly hand it. But a room costs about **0.25 MB** rather than 10 MB, roughly **40x smaller**. The same thousand programs fit in one building using a few hundred MB, not 10 GB of houses (`docs/scaling-eli5.md:17-23`).

Now name the parts. The "room" is a Wasmtime instance — its own linear memory, its own sandbox. The "doors" are WASI syscalls, which in Wanix are an adapter *into* the task's namespace rather than a delegation to the host (see [Wasmtime as the substrate](/concepts/wasmtime-as-substrate)). The "building" is a single Wasmtime engine inside one host process. The reason memory isolation holds is structural: each instance has its own linear memory, so one guest cannot read another's heap. That part is real today.

## Does it stay fast when the programs actually work?

Sleeping programs prove nothing, so the project measured them under load (`docs/scaling-eli5.md:25-39`). Two numbers matter, both from one 18-core dev machine — they will vary by hardware.

**Compute scales with cores, then holds flat.** A thousand programs were each given a heavy math task at the same time. The work spread across all the cores, throughput climbed to match the hardware, and then stayed flat. Going from a handful of programs to a thousand did not make each one fall apart; extra programs simply wait their turn in an orderly line instead of dragging each other down. One program finishes a unit of work in about a second, and 18 of them — one per core — also finish in about the same time, together (`docs/scaling-eli5.md:30-34`).

**File operations are fast, and they parallelize.** A single program reads and writes files through Wanix at about **500,000 operations per second**. Because every program has its own private files — its own namespace — fifty programs together reach about **9 million operations per second** (`docs/scaling-eli5.md:36-39`). The per-process namespace is what makes that scale: there is no shared global file table for fifty programs to contend on.

The honest counterweight: Wanix does not make the *language* fast (`docs/scaling-eli5.md:41-46`). The QuickJS engine in a room is an interpreter, so heavy number-crunching runs slower than native code. That is the trade for tiny, instant, secure rooms. Wanix's superpower is not making one program fast; it is running thousands of isolated programs cheaply at once, scaling smoothly up to whatever your hardware allows.

## Same room, a faster engine inside

The room does not care what engine sits inside it. A room can hold the QuickJS interpreter, or a compiled `wasm32-wasi` module, or — at the heavy end — a full virtual machine. Pick the engine per task; the isolation story is identical across all three (`docs/scaling-eli5.md:48-66`):

- **Quick and easy:** write JavaScript, no build step. Good for glue code and small or AI-generated snippets. Slower for heavy math.
- **Fast when it matters:** compile Rust, Go, C, or Zig to WebAssembly. Same secure room, near-native speed. Drop your hot paths here.
- **Heavy-duty:** a full program in its own building, or a complete VM, when you need a whole operating system or maximum isolation.

Because the security story is the same across all three, the tier is a pure cost decision, not a fork in the architecture. You start a feature in qjs for iteration speed and move its inner loop into a `.wasm` task later without re-reasoning about the sandbox, the namespace, or who can read your files. That equivalence — interpreted and compiled tasks on one substrate sharing one filesystem — is its own page; see [two tiers, one substrate](/concepts/two-tiers-one-substrate).

## Why have more than one building?

Buildings (host processes) are chosen for safety, not one per user (`docs/scaling-eli5.md:69-85`). A small number of buildings, each using all your cores, each holding thousands of cheap private rooms. You add a building for **blast radius** (a problem in one building only affects that building's tenants), for **bad neighbors** (a program stuck in a loop is easier to contain), or for **extra-sensitive tenants** (untrusted or high-value code can get its own building, or a fully fortified VM). The number of buildings tracks how much isolation you need, not how many users show up.

## Status / honest limits

The isolation here is cheap and real, but it is not yet the full story — state the boundary flatly.

- **Per-room memory isolation is real; hard CPU/memory limits are not.** Each Wasmtime instance is its own linear memory, so one guest cannot read another's heap today. But the preemption and capping that make a room safe for *arbitrary untrusted* code — Wasmtime epoch/fuel interruption and linear-memory caps — are not wired up yet (`docs/scaling-eli5.md:89-95`). A runaway guest can still hog CPU or grow memory. Say "cheap, scalable isolation," not "safe for arbitrary untrusted code." See [safe-for-untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable) and [bounded execution policy](/concepts/bounded-execution-policy).
- **The exec tiers are local-trust only.** The devices that *run* code — `#task`, `#cpu`, `#agent` — are not exposed to untrusted or public peers. The substrate gives cheap isolation per instance; it does not yet give the resource bounds you would need before letting a hostile peer start tasks on your node.
- **The numbers are from one machine.** The ~10 MB/process, ~0.25 MB/room, ~500k single / ~9M-at-50 file ops, and core-scaling figures all come from one 18-core dev machine (`docs/scaling-eli5.md:1-4`, `36-39`). Treat them as the shape of the curve, not a guaranteed benchmark on your hardware.

## See also

- [Wasmtime as the substrate](/concepts/wasmtime-as-substrate) — the engine that hosts the rooms and where the substrate/foundation line sits.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — interpreted qjs and compiled wasm sharing one filesystem in identical rooms.
- [Safe-for-untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable) — the honest boundary on running hostile code.
- [Bounded execution policy](/concepts/bounded-execution-policy) — where the CPU/memory caps will live when they land.
