---
title: Guest-JS Guardrails
slug: concepts/guest-js-guardrails
pageType: concept
oneLiner: Guest code uses qjs:std, qjs:os, scriptArgs, stdio, env, and service files — never globalThis.Wanix helpers or a read-only virtual-file bridge.
audience: [developer]
tags: [qjs, shipped, caveat, local-trust-only]
sourceRefs:
  - crates/wanix-qjs/src/host_api.rs:12-35
  - crates/wanix-qjs/src/driver.rs:54-63
  - crates/wanix-qjs-engine/README.md:455-489
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:140-149
  - AGENTS.md:313-315
seeAlso: [concepts/qjs-task, concepts/wanix-backed-wasi, concepts/raw-vs-cooked-input, devices/task, devices/term, concepts/bounded-execution-policy]
prerequisites: [concepts/qjs-task]
usedInFlows: []
honestLimits:
  - Wanix runtime paths must use live WASI providers; the engine's read-only virtual files are fixture support only and carry no namespace authority.
  - QuickJS interrupt/memory limits are a CPU-bound task-driver policy, not a general task cancellation or signal mechanism.
canonicalCaveatFor: []
---

# Guest-JS Guardrails

Guest code uses `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and service files — never `globalThis.Wanix` helpers or a read-only virtual-file bridge.

A QuickJS task in Wanix is just JavaScript that imports two standard modules and reads files. There is no Wanix-specific JavaScript API to learn, no injected helper object, and no special projection of the host. This page pins down exactly what a guest is allowed to reach, and — just as importantly — what it must not, so that the same script behaves identically whether it runs from the native CLI or over a served namespace.

## The allowed surface

Start a task and look at what the script actually touches:

```js
import * as std from "qjs:std";
import * as os from "qjs:os";

// argv as a frozen array
std.out.puts("args " + scriptArgs.slice(1).join("|") + "\n");

// task identity and knobs are files
std.out.puts("id "  + std.loadFile("#task/self/id").trim()  + "\n");
std.out.puts("cwd " + std.loadFile("#task/self/dir").trim() + "\n");

// env, stdio, and ordinary files
std.out.puts("mode " + std.getenv("MODE") + "\n");
std.writeFile("#kv/greeting", "hello");          // a device, opened by name
const meta = os.lstat("out.txt");                 // live WASI fs helper
```

That is the entire vocabulary: the QuickJS standard modules `qjs:std` and `qjs:os`, the frozen `scriptArgs`, process stdio (`std.out`/`std.err`/`std.in`), environment variables, and the file tree — including the `#`-named service devices, which a guest opens by path like any other file (`crates/wanix-qjs/src/lib.rs:904-913`). The only Wanix-specific global is `scriptArgs`, installed by a tiny prelude that freezes a JSON-encoded argv (`crates/wanix-qjs/src/host_api.rs:12-35`). Everything else is plain QuickJS reaching live, Wanix-backed WASI. The Plan 9 framing: the guest has no ambient authority beyond the file view it was handed — see [host, not ambient authority](/concepts/host-not-ambient-authority).

## Why the globalThis.Wanix bridge is gone

Earlier prototypes injected a `globalThis.Wanix` object with bespoke methods for storage, tasks, and the rest. That is a retired idea, by decision: the QuickJS/WASI task runtime ADR states flatly that JavaScript "reaches Wanix-owned task, filesystem, fd, and lifecycle semantics without `globalThis.Wanix` helpers and without a read-only virtual projection bridge" (`docs/adrs/0002-quickjs-wasi-task-runtime.md:140-149`).

The reason is the one this whole codebase keeps returning to: if a capability is reachable as a *file*, then it is reachable over 9P, across the mesh, and from any other task — for free. A `globalThis.Wanix.kvGet()` helper would be a second, JavaScript-only API that none of that machinery understands. `std.writeFile("#kv/greeting", ...)` is the same operation any 9P client, any shell, or any peer can perform. One contract, no parallel surface.

## Engine read-only virtual files are fixture support only

The `wanix-qjs-engine` crate has a `with_read_only_virtual_file` knob that attaches immutable bytes at a guest path under a single root preopen (`crates/wanix-qjs-engine/README.md:455-476`). It is tempting to reach for it. Do not, in a runtime path.

The crate README is explicit: "Wanix runtime paths must not depend on these read-only virtual files for namespace behavior. They are engine fixture support only" (`crates/wanix-qjs-engine/README.md:483-489`). The virtual-file surface has no directory listing, no mutation, no symlinks, no service paths — it exists so engine tests can pin a config blob without a host mount. Real Wanix tasks get mutable files, directory listing, service paths, stdio, argv/env, and process exit through live `QuickJsWasiHost` providers that `wanix-qjs` wires to the task's namespace. If a guest is reading config from a virtual fixture, it is testing the engine, not running on Wanix.

## Control bytes are terminal input, not task cancellation

When a `qjs` task is terminal-backed, bytes like `0x03` (Ctrl-C) and `0x04` (Ctrl-D) are *input to the guest*, delivered through `#term`, and the guest shell owns what they mean — line cancellation, EOF, command dispatch. Terminal clients in the browser, editor, or VM forward those bytes; they "should not invent task cancellation semantics around them" (`AGENTS.md:313-315`). See [raw vs cooked input](/concepts/raw-vs-cooked-input).

This is a deliberate boundary against a tempting shortcut. Wanix *does* have a bounded-execution knob — the QuickJS interrupt-poll budget that stops CPU-bound JavaScript — but its own doc comment calls it "a bounded task-driver policy for CPU-bound JavaScript, not a general Wanix cancellation or signal mechanism" (`crates/wanix-qjs/src/driver.rs:54-63`). A `^C` typed at a terminal is not a kill signal routed through that budget; it is a character the guest reads. Keeping the two separate is what lets a guest shell handle interrupts the way a Plan 9 shell does, while the host retains a distinct, explicit ceiling on runaway compute.

## See also

- [The qjs task](/concepts/qjs-task) — the task this page constrains: identity, fds, exit status.
- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the live providers that back `qjs:os` and stdio.
- [Raw vs cooked input](/concepts/raw-vs-cooked-input) — where `0x03`/`0x04` actually go.
- [The #task device](/devices/task) — the service files a guest reads under `#task/self`.
- [The #term device](/devices/term) — the terminal that delivers control bytes.
- [Bounded execution policy](/concepts/bounded-execution-policy) — the interrupt/memory ceiling that is not cancellation.

## Status / honest limits

- The engine's read-only virtual files are fixture support only and carry no namespace authority. Runtime paths must use live `QuickJsWasiHost` providers; a guest that depends on a virtual fixture is exercising the engine, not Wanix (`crates/wanix-qjs-engine/README.md:483-489`).
- The QuickJS interrupt-poll and heap limits are a CPU-bound task-driver policy, not a general task cancellation or signal mechanism (`crates/wanix-qjs/src/driver.rs:54-63`). There are no hard, enforced CPU/memory limits across the task surface yet, and exec devices are local-trust only — this guardrail constrains what a guest *reaches*, not what it is *allowed to consume* against an untrusted-peer threat model.
- `scriptArgs` is the only Wanix-specific global, and it is frozen at start (`crates/wanix-qjs/src/host_api.rs:12-16`). There is no `globalThis.Wanix` object to discover; everything else is standard QuickJS plus the file tree.
