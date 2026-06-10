---
title: Guest SDK (lib/wanix)
slug: reference/guest-sdk
pageType: reference
oneLiner: The JavaScript helper library a guest task uses on top of qjs:std/os and service files.
audience: [developer]
tags: [reference, cli, shipped, guest-js, caveat]
sourceRefs:
  - examples/lib/wanix/index.js
  - examples/lib/wanix/fs.js
  - examples/lib/wanix/process.js
  - examples/lib/wanix/task.js
  - examples/lib/wanix/bytes.js
  - examples/lib/wanix/wanix-sdk.d.ts
  - examples/lib/wanix/wanix-qjs.d.ts
  - examples/qjs-task-spawn.js
seeAlso:
  - concepts/qjs-task
  - concepts/guest-js-guardrails
  - concepts/wanix-backed-wasi
  - devices/task
prerequisites: []
usedInFlows: []
honestLimits:
  - "globalThis.Wanix helpers are a retired bridge idea; guests use qjs:std/os and service files instead."
  - "The SDK is convenience over the raw runtime; it adds no capabilities a guest could not reach through qjs:os directly."
canonicalCaveatFor: []
---

# Guest SDK (lib/wanix)

The JavaScript helper library a guest task uses on top of `qjs:std`/`qjs:os` and service files.

A `qjs` guest already has everything it needs: `qjs:std` and `qjs:os` for stdio and the filesystem, `scriptArgs` for argv, and the `#task` service files for its own identity. The SDK under `examples/lib/wanix/` is a thin, dependency-free wrapper that turns the repetitive `os.open`/`os.read`/`os.close` dance into named functions, so guest scripts read like intent instead of plumbing. It adds no authority — every call resolves through the same namespace the guest could already reach. Show it first, then name the parts.

## The headline win: spawn a child task

This is what the SDK is for. The following guest, run as `wanix qjs examples/qjs-task-spawn.js`, allocates a child `qjs` task, wires its stdio, starts it, and reads its exit code — all through `#task` service files (`examples/qjs-task-spawn.js:1-26`):

```js
import { spawn } from "lib/wanix/task.js";
import { self } from "lib/wanix/process.js";

const parent = self.id();
const child = spawn("qjs", {
  cmd: "qjs-task-spawn-child.js alpha 'two words'",
  env: { MODE: "spawned" },
  dir: ".",
  binds: [["#task/" + parent + "/fd/1", 1], ["#task/" + parent + "/fd/2", 2]],
});
child.start();
print("child exit: " + child.wait());
```

`spawn(kind, opts)` reads `#task/new/<kind>` to allocate, writes the `cmd`/`env`/`dir` files (each with the trailing newline the device expects), and issues `bind SRC fd/N` verbs through `<id>/ctl` (`examples/lib/wanix/task.js:56-75`). It deliberately does **not** auto-start, matching the examples that print the allocated id first. `Task.start()` writes `start` to `ctl`; `Task.wait()` reads and parses `#task/<id>/exit` (`examples/lib/wanix/task.js:39-44`). That sequence — `new`, configure, `bind`, `start`, read `exit` — is the [`#task` device](/devices/task) contract, named.

## The modules

`index.js` is a barrel: `import * as wanix from "lib/wanix/index.js"` re-exports `bytes`, `fs`, `process`, plus `spawn` and `Task` from `task.js` (`examples/lib/wanix/index.js:5-8`). Each module is also importable directly by its bare specifier, which is how the examples use it.

- **`fs.js`** — namespace filesystem wrappers over `qjs:std`/`qjs:os`. `readText(path)` opens, reads one bounded 4096-byte chunk, and closes — sized so it reads a `#task/...` service file in a single answer the same way it reads an ordinary file (`examples/lib/wanix/fs.js:10-23`). `readBytes(path)` loops to EOF for ordinary files. `writeText` checks for a short write and throws; `createText` adds `O_CREAT|O_TRUNC`. `list`, `exists`, `mkdir`, `remove` wrap `os.readdir`/`os.stat`/`os.mkdir`/`os.remove`, converting the runtime's nonzero error codes into thrown `Error`s (`examples/lib/wanix/fs.js:69-88`).
- **`process.js`** — current-task introspection and stdio. `self.id()`/`self.cmd()`/`self.dir()` read `#task/self/*` and trim (`examples/lib/wanix/process.js:8-21`). `args()`/`arg(i)`/`programArgs()` wrap `scriptArgs`; `getenv(name, fallback)` wraps `std.getenv` with a default; `print`/`write`/`eprint` wrap `std.out`/`std.err` with a flush so output is not buffered past the call.
- **`task.js`** — `spawn` and the `Task` class above. `bindFd(src, n)` quotes a `src` path containing whitespace before emitting the `ctl` verb (`examples/lib/wanix/task.js:9-11,36-38`).
- **`bytes.js`** — `bytesFromString`/`stringFromBytes` using Latin-1 `charCodeAt`, because the QuickJS WASM fixture ships no `TextEncoder`/`TextDecoder` (`examples/lib/wanix/bytes.js:1-13`). Other modules build on these.

## Typed surface for editors

Two `.d.ts` files give the workbench TypeScript service real completion. `wanix-sdk.d.ts` declares the `lib/wanix/*.js` modules — including `SpawnOptions` (`cmd`, `env` as record or string, `dir`, `binds`, `inheritStdio`) and the fluent `Task` class (`examples/lib/wanix/wanix-sdk.d.ts:35-54`). `wanix-qjs.d.ts` is the ambient contract for the runtime itself: `qjs:std`, `qjs:os`, and the `scriptArgs`/`print`/`console` globals, declaring only members the runtime actually exposes — no Node, no DOM (`examples/lib/wanix/wanix-qjs.d.ts:7-87`). When prose and the runtime disagree, these declarations track what `wanix-qjs` really exports.

## Use the runtime, not `globalThis.Wanix`

The SDK is the sanctioned shape of [guest JS](/concepts/guest-js-guardrails): build on `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and service files. The old `globalThis.Wanix.*` browser-bridge helpers are retired — guests should not reach for them, and the runtime does not install them. Because the SDK is plain ES modules with zero non-runtime imports, the same script runs under `wanix qjs` natively, in a served session, or in the browser cockpit, and reaches [Wanix-backed WASI](/concepts/wanix-backed-wasi) identically in each. Nothing here is privileged; it is ergonomics over the one contract.

## See also

- [The qjs task](/concepts/qjs-task) — how a `.js` guest becomes a Wanix task.
- [Guest JS guardrails](/concepts/guest-js-guardrails) — why `qjs:std`/`os` and service files, not `globalThis.Wanix`.
- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the host imports the guest's `os` calls land on.
- [The #task device](/devices/task) — the `new`/`cmd`/`env`/`ctl`/`exit` files `spawn` drives.

## Status / honest limits

- The SDK adds no authority. Every function resolves through `qjs:os`/`qjs:std` and the namespace the guest already has; it cannot reach a file or service the guest could not open directly. It is convenience, not capability.
- `globalThis.Wanix` is a retired bridge idea and is not part of this surface. Guest scripts use `qjs:std`/`qjs:os` and service files (`examples/lib/wanix/process.js:1-5`).
- `readText` reads a single 4096-byte chunk, sized for service files that answer in one bounded read; for ordinary files larger than that, use `readBytes`, which loops to EOF (`examples/lib/wanix/fs.js:10-41`).
- `bytes.js` uses Latin-1 encoding because the QuickJS fixture has no `TextEncoder`/`TextDecoder`; non-ASCII round-trips only within one byte per code unit (`examples/lib/wanix/bytes.js:1-13`).
