---
title: Scaffold a project with wanix new
slug: recipes/00-scaffold-a-project
pageType: recipe
oneLiner: Generate a JS or Rust Wanix project with the guest SDK wired in.
audience: [newcomer, developer]
tags: [recipe, cli, shipped]
sourceRefs:
  - crates/wanix-cli/src/new.rs:14-200
  - examples/lib/wanix/process.js:1-58
  - examples/lib/wanix/fs.js:1-97
  - examples/tsconfig.wanix-qjs.json:1-16
seeAlso:
  - concepts/qjs-task
  - concepts/compiled-wasm-task-driver
  - concepts/guest-js-guardrails
  - concepts/http-app-route
  - recipes/walkthrough-1-run-js
  - recipes/04-tiny-http-app-with-kv
  - reference/guest-sdk
prerequisites: []
usedInFlows:
  - {flow: http-app-with-kv, step: 1}
honestLimits:
  - "wanix new refuses to write into a non-empty target directory; it scaffolds, it does not merge into an existing project."
  - "The Rust starter targets wasm32-wasip1; building it needs rustup target add wasm32-wasip1 if that target is missing."
canonicalCaveatFor: []
---

# Scaffold a project with wanix new

Generate a JS or Rust Wanix project with the guest SDK wired in.

**What & why.** Before you write a Wanix program you want the same thing every toolchain gives you: a directory that already builds and runs, with the imports resolved and an editor that understands them. `wanix new` does exactly that and nothing more. One command writes a hello-world program, a README that names the run command, and — for JavaScript — the guest SDK under `lib/wanix/` plus a `tsconfig.json` that points your editor's TypeScript service at the SDK's type definitions. You get a working `main.js` (or `src/main.rs`) you can run in the next breath, then point at the HTTP-app route once it does something useful.

## Generate a JavaScript project

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust new --js counter
# created js project ./counter
# next: wanix-rust qjs ./counter/main.js
```

The CLI prints the next step itself (`crates/wanix-cli/src/new.rs:169`). The kind flag is required and exclusive — pass exactly one of `--js NAME` or `--rust NAME`, or you get a usage error (`new.rs:143-148`). Add `--dir DIR` to choose the parent directory; the default is `.` (`new.rs:113-118`). The target `<parent>/<name>` must be empty or non-existent, or the command refuses (`new.rs:206-222`).

## The generated layout

For `--js`, `wanix new` writes ten files (`new.rs:178-191`):

```text
counter/
  main.js              # hello-world: print args, write then read hello.txt
  README.md            # names `wanix-rust qjs main.js`
  tsconfig.json        # checkJs over lib/wanix/*.d.ts
  lib/wanix/index.js   # SDK barrel
  lib/wanix/bytes.js
  lib/wanix/fs.js      # readText/writeText over service AND ordinary files
  lib/wanix/process.js # self.id/cmd/dir, args(), print/eprint
  lib/wanix/task.js
  lib/wanix/wanix-sdk.d.ts
  lib/wanix/wanix-qjs.d.ts
```

The starter `main.js` imports the SDK rather than poking `qjs:std` inline — `import * as fs from "lib/wanix/fs.js"` and `import { print, args } from "lib/wanix/process.js"` (`new.rs:14-24`). Those helpers are pure functions over `qjs:std`/`qjs:os` and the service files; there is no `globalThis.Wanix` (`examples/lib/wanix/process.js:1-7`). `fs.createText` and `fs.readText` route through the namespace, so the same call reads an ordinary file or a `#task/self/*` service file (`examples/lib/wanix/fs.js:1-23`). The `tsconfig.json` turns on `checkJs` and includes `lib/wanix/*.d.ts` (`examples/tsconfig.wanix-qjs.json`), so your editor type-checks the SDK without a build step.

## Generate a Rust project

```sh
wanix-rust new --rust upper
# created rust project ./upper
# next: cd ./upper && cargo build --release --target wasm32-wasip1
```

The Rust starter is a standalone Cargo project (`new.rs:193-200`): `Cargo.toml` declares its own empty `[workspace]` so it detaches from any parent workspace, `.cargo/config.toml` sets the default build target to `wasm32-wasip1`, and `src/main.rs` reads an input file and writes an uppercased copy through ordinary `std::fs` (`new.rs:33-44`). The point of the `.wasm` tier is that you write normal Rust against `std::fs`, and Wanix-backed WASI makes those syscalls land in the task's namespace.

## Run it

```sh
wanix-rust qjs ./counter/main.js
# hello from a Wanix qjs task
# args: ["main.js"]
# hello.txt = hello world

# Rust: build to wasm first, then run the module as a task
cd ./upper && cargo build --release --target wasm32-wasip1
echo "make me loud" > in.txt
wanix-rust wasm target/wasm32-wasip1/release/upper.wasm /in.txt /out.txt
# wrote /in.txt -> /out.txt
cat out.txt
# MAKE ME LOUD
```

Two things worth knowing about that run. The wasm task's namespace root maps to your **host working directory**, so `/in.txt` and `/out.txt` are `./in.txt` and `./out.txt` next to you — that is where the output lands. And the arguments are optional: with no `in.txt` (or no args at all) the starter silently falls back to transforming the built-in `"hello world\n"`, which is why it prints `wrote /in.txt -> /out.txt` even when no input file exists. If the wasm target is missing, `rustup target add wasm32-wasip1` (the generated README says so too, `new.rs:65-70`). Both paths run as a Wanix task: a `.js` file becomes a [qjs task](/concepts/qjs-task), a `.wasm` file runs through the [compiled-wasm task driver](/concepts/compiled-wasm-task-driver) — two tiers on one substrate.

## Next: serve it via /.wanix/app/&lt;name&gt;

A standalone program is the floor, not the ceiling. The same `main.js`, dropped under `apps/<name>.js` in a served root, becomes an HTTP handler: `wanix-rust serve --wanix-services` binds the device set, and a loopback `GET /.wanix/app/<name>` spawns a fresh qjs task per request and returns its stdout. Back the handler's state with `#kv` and you have a tiny web app whose state is just a file. [Recipe 04](/recipes/04-tiny-http-app-with-kv) walks the counter end to end.

## See also

- [Walkthrough 1 — Run JavaScript outside Chrome](/recipes/walkthrough-1-run-js) — the smallest proof a qjs task reads from a Wanix namespace.
- [The qjs task](/concepts/qjs-task) — how a `.js` file becomes a Wanix task.
- [The compiled wasm task driver](/concepts/compiled-wasm-task-driver) — how the `--rust` starter's `.wasm` runs.
- [Guest JS guardrails](/concepts/guest-js-guardrails) — why the SDK avoids `globalThis.Wanix`.
- [The HTTP-app route](/concepts/http-app-route) — the `/.wanix/app/<name>` contract this scaffold feeds into.
- [The guest SDK](/reference/guest-sdk) — reference for the `lib/wanix/` helpers `wanix new` writes.

## Status / honest limits

These are engineering boundaries, stated once.

- **`wanix new` scaffolds, it does not merge.** The target directory must be empty or absent; a non-empty target returns `new: target … must be empty` (`crates/wanix-cli/src/new.rs:206-222`). Point it at a fresh path.
- **The Rust starter targets `wasm32-wasip1`.** Building needs that Rust target installed; run `rustup target add wasm32-wasip1` if it is missing (`new.rs:65-70`).
- **The HTTP-app route is loopback-only and services-gated.** Serving a scaffolded program at `/.wanix/app/<name>` requires `--wanix-services` and refuses non-loopback peers. It is not a public endpoint; see [Recipe 04](/recipes/04-tiny-http-app-with-kv) for the boundary.
