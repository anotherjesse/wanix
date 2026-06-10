---
title: Build & Install the Wanix CLI
slug: reference/build-and-install
pageType: reference
oneLiner: "The one canonical page for building wanix-rust from source: toolchain, the build command, the alias, and the optional wasm32-wasip1 target."
audience: [newcomer, developer]
tags: [reference, cli, shipped]
sourceRefs:
  - Cargo.toml
  - Justfile:3-15
seeAlso:
  - learn/js-outside-chrome
  - recipes/walkthrough-1-run-js
  - reference/contributor-landing
  - reference/cli-command-index
prerequisites: []
usedInFlows: []
honestLimits:
  - "There is no packaged Rust-port release yet; the Homebrew/release install in the repo README is the legacy Go toolchain. The Rust CLI is built from source."
canonicalCaveatFor: []
---

# Build & Install the Wanix CLI

Every flow and recipe on this site runs the native CLI, `wanix-rust`. This page is the one place its build steps live; the other pages link here instead of repeating them.

## 1. Prerequisite: a Rust toolchain

Install Rust via [rustup](https://rustup.rs) (stable channel). `cargo` and `rustc` on `PATH` is all the CLI itself needs.

## 2. Build once

From the workspace root:

```sh
cargo build --locked --package wanix-cli
```

The first build compiles the whole workspace (a few minutes is normal); after that, rebuilds are fast. The binary lands at `./target/debug/wanix-rust`.

## 3. Alias it

The docs write `wanix-rust ...` everywhere. Make that work in your shell:

```sh
alias wanix-rust='./target/debug/wanix-rust'
```

(Or use `WANIX=./target/debug/wanix-rust` and `$WANIX ...` — same thing.) The alias assumes you stay in the workspace root; use the absolute path if you wander.

## 4. Check it worked

```sh
wanix-rust qjs examples/qjs-demo.js
```

```text
outside Chrome: true
task id: 1
Wanix ES module loader
hello from a Wanix namespace
```

`wanix-rust --help` lists every subcommand; the [CLI command index](/reference/cli-command-index) maps each one to its docs.

## 5. Optional: the `wasm32-wasip1` target

Running the CLI — including the checked-in `.wasm` fixtures — needs nothing beyond step 2. You only need the wasm target when you **compile your own guest** (the `wanix new --rust` starter, or the `wanix-sh` shell source):

```sh
rustup target add wasm32-wasip1
```

## Two kinds of paths (read once, saves an hour)

CLI flags mix two path worlds, and every early stumble is one of these:

- **Host paths** name files on your disk: the script argument to `qjs`, `serve`'s root directory (which must already exist — `serve` does not create it), and the left side of `--mount HOST_DIR=NAME`.
- **Namespace paths** name files inside the task's Wanix namespace: `--cwd`, `#kv/<key>`, `#task/self/id`, and everything a guest opens. `--cwd /tmp/something` is *not* a host directory; to work against a host dir, bind it in first: `--mount /tmp/something=world --cwd world`.

## Status / honest limits

- There is no packaged release of the Rust port; build from source as above. The Homebrew/release install in the repo `README.md` is the legacy Go toolchain.
- `cargo build` (debug) is what the docs assume; `--release` works and is faster at runtime but slower to build.

## See also

- First flow: [JavaScript outside Chrome](/learn/js-outside-chrome)
- First recipe: [Walkthrough 1 — run JS](/recipes/walkthrough-1-run-js)
- Contributors: [the contributor landing](/reference/contributor-landing) and [quality gates](/reference/quality-gates)
