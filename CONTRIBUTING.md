# Contributing

Wanix in this repository is the Rust-native implementation. The original
Go/browser implementation lives at https://github.com/tractordev/wanix/; it is
useful historical context, but it is not the build system or runtime foundation
for this workspace.

## Prerequisites

- Rust toolchain with the workspace `rust-version`
- `just` for the standard check gate
- Node.js/npm only when working on the browser cockpit under `workbench/`

## Build And Test

Build the CLI:

```sh
cargo build --locked --package wanix-cli
./target/debug/wanix --help
```

Run the workspace gate:

```sh
just check
```

The root Makefile is a thin wrapper around the same commands:

```sh
make build
make test
make check
```

Build the browser cockpit extension bundle when you touch `workbench/`:

```sh
cd workbench
npm install
npm run compile-web
```

## Directory Layout

```text
crates/      Rust workspace crates
docs/        ADRs, recipes, site content, and design notes
examples/    Current qjs/AppFS examples and guest SDK snippets
tools/       Local contributor scripts
v86/         Static direct-v86 browser assets embedded by wanix serve
workbench/   VS Code / Code OSS web extension cockpit
```

See [AGENTS.md](AGENTS.md) for the current crate map, dependency direction, and
implementation notes.
