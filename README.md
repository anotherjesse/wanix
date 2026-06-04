# Wanix
[![Discord](https://img.shields.io/discord/415940907729420288?label=Discord)](https://discord.gg/nQbgRjEBU4) ![GitHub Sponsors](https://img.shields.io/github/sponsors/progrium?label=Sponsors)

A virtual environment toolkit for the web, inspired by Plan 9.

**📺 [Wanix: The Spirit of Plan 9 in Wasm](https://www.youtube.com/watch?v=kGBeT8lwbo0)**

* Run WASI and x86 programs on web pages
* Apply Plan 9 ideas in the browser
* Build a web-native operating system

### Features

* Capability-oriented microkernel architecture ("everything is a file")
* Abstract POSIX process model for generalized compute execution
* Per-process namespaces for security, isolation, and custom environments
* Built-in emulator for x86 support and Linux compatibility

## Try online demo

Play with the Wanix shell bundle at [wanix.run](https://wanix.run).

## Rust-Native Port Status

This repository also contains the active Rust-native Wanix port. The north star
for that work is a Wanix core that can run outside Chrome, with Wasmtime as the
execution substrate and QuickJS/WASI as the first serious task runtime.

The Rust port now has native slices for QuickJS-backed Wanix tasks, terminal
devices with service-file lifecycle control, `qjs-shell` with basic filesystem
commands and child `qjs` task launches, 9P exports over stdio/TCP/WebSocket,
Rust `serve` discovery, browser filesystem and workbench demos, direct-v86
handoff, rootfs preparation with shell/JSON handoffs, trusted-local served
prepared-root handoff discovery with copyable browser commands, and
initrd-aware native QEMU command/JSON handoff.
Wanix owns task identity, namespaces, cwd/env/cmd, stdio/fds, exit status, and
WASI filesystem semantics; QuickJS is the execution engine inside a `qjs` task.

Try the native demo path from the workspace root:

```sh
cargo run --locked --package wanix-cli -- qjs examples/qjs-demo.js
cargo run --locked --package wanix-cli -- \
  qjs-term --stdin "hello terminal" examples/qjs-term-demo.js
printf 'write note.txt hello\nls\ncat note.txt\nexit\n' | cargo run --locked --package wanix-cli -- qjs-shell
cargo test --workspace --locked
```

The Rust workspace crates and active ADR index are documented in
[AGENTS.md](AGENTS.md). For a hands-on path, see
[rust-walkthrough.md](rust-walkthrough.md). For the Go/Rust architecture
comparison, see [docs/rust-vs-go-wanix.md](docs/rust-vs-go-wanix.md). The
QuickJS/Wasmtime engine mechanics live in
[crates/wanix-qjs-engine](crates/wanix-qjs-engine), while Wanix process,
namespace, fd, and WASI policy stay in the Wanix crates above it.


### Install the Wanix Toolchain

Download the Wanix CLI from the [latest release](https://github.com/tractordev/wanix/releases/latest)
or install with Homebrew:

```
brew install progrium/taps/wanix
```

If you want to build from source, see the [CONTRIBUTING.md](CONTRIBUTING.md) doc.

### File Services

Wanix has a number of built-in file services:

* `#task`
* `#term`
* `#vm`
* `#ramfs`
* `#pipe`
* `#signal`
* `#web`
* `#wanix`


### API Reference

For now, see [api/](api/) and [api/handle.js](api/handle.js).

## Contributing

We'd love your contributions! Take a look at our [issues](https://github.com/tractordev/wanix/issues) to see how you can help out. You can also ask questions and participate in [discussions](https://github.com/tractordev/wanix/discussions), however right now most discussion takes place in our [Discord](https://discord.gg/nbrwNXVvVa).

Be sure to read our [CONTRIBUTING.md](CONTRIBUTING.md) doc to get started.

## Older Demos

* [📺 Wasm I/O 2024 Demo](https://www.youtube.com/watch?v=cj8FvNM14T4)
* [📺 Mozilla Rise 25 Demo](https://www.youtube.com/watch?v=KJcd9IckJj8)

## License

MIT
