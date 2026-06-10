# Wanix
[![Discord](https://img.shields.io/discord/415940907729420288?label=Discord)](https://discord.gg/nQbgRjEBU4) ![GitHub Sponsors](https://img.shields.io/github/sponsors/progrium?label=Sponsors)

Wanix is a Rust-native environment inspired by Plan 9: processes, devices,
services, namespaces, remote peers, and agents compose through one filesystem
contract. The active implementation in this repository builds the `wanix` CLI.

Historical note: the original Wanix was implemented in Go, targeted running in
the browser, and treated the browser as the runtime host. That project lives at
https://github.com/tractordev/wanix/. In this repository and its docs, plain
`wanix` now refers to the Rust-native CLI/core.

## What It Does

**Runtime.** Wanix owns task identity, namespaces, cwd/env/cmd, stdio/fds, exit
status, and WASI filesystem semantics. `qjs` runs JavaScript through
QuickJS/WASI, and `wasm` runs compiled `wasm32-wasi` modules as a second task
runtime on the same substrate. Terminals (`#term`) drive native and served
`qjs-shell` sessions with filesystem/env builtins, child task launches, resize
tracking, and lifecycle control.

**Devices.** Beyond `#task` and `#term`, service devices are plain filesystems:
`#kv` for key/value state, `#pipe` for byte channels, `#plumb` for pub/sub,
`#cas` for content-addressed storage, and `#agent` for an LLM session as files
with approvals as files.

**Mesh.** Wanix nodes import remote namespaces over the native
FileSystem-over-iroh wire. Each node has a persisted ed25519 identity, attach is
capability-gated, and devices import across the mesh because they are just
filesystems. `#cpu` runs a task on a peer against the caller's reverse-exported
namespace, and `wanix capsule` freezes a world into a portable CAS-backed
snapshot.

**Serve + cockpit.** `wanix serve` exports Wanix filesystems over
stdio/TCP/WebSocket/HTTP with a discovery document, plus direct-v86 and native
QEMU handoffs. The browser cockpit under `workbench/` is a frontend over direct
9P: it inspects service devices, repairs a broken program through `#agent`,
runs a qjs-to-wasm duet on one shared filesystem, serves HTTP apps with
`#kv`-backed state, and self-checks the device set.

## Try It

Build the CLI once from the workspace root:

```sh
cargo build --locked --package wanix-cli
./target/debug/wanix qjs examples/qjs-demo.js
./target/debug/wanix qjs-term --stdin "hello terminal" examples/qjs-term-demo.js
printf 'write note.txt hello\nls\ncat note.txt\nexit\n' | ./target/debug/wanix qjs-shell
```

Serve the browser cockpit:

```sh
mkdir -p /tmp/wanix-root
./target/debug/wanix serve --root /tmp/wanix-root --bundle workbench-fs9p --wanix-services
```

Run the workspace gate:

```sh
just check
```

## Project Map

The workspace crates and active ADR index are documented in [AGENTS.md](AGENTS.md).
The mesh design is in [docs/mesh-blueprint.md](docs/mesh-blueprint.md) and
[docs/mesh-the-missing-half-of-9p.md](docs/mesh-the-missing-half-of-9p.md).
Hands-on recipes live in [docs/recipes/](docs/recipes/), and a longer manual
walkthrough lives in [rust-walkthrough.md](rust-walkthrough.md).

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md), then use `just check` before
sending changes.

## License

MIT
