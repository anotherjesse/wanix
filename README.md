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
is a Wanix core that runs outside Chrome, with Wasmtime as the execution
substrate and QuickJS/WASI as the first serious task runtime — and that reaches
across machines through a Plan 9-style mesh.

**Runtime.** Wanix owns task identity, namespaces, cwd/env/cmd, stdio/fds, exit
status, and WASI filesystem semantics. `qjs` runs JavaScript (QuickJS is the
engine inside the task) and `wasm` runs compiled `wasm32-wasi` modules as a
second task runtime on the same substrate. Terminals (`#term`) drive native and
served `qjs-shell` sessions with filesystem/env builtins, child task launches,
resize tracking, and lifecycle control.

**Devices.** Beyond `#task` and `#term`, the port exposes service devices as
plain filesystems: `#kv` (key/value state), `#pipe` (byte channels), `#plumb`
(plumber pub/sub bus), `#cas` (content-addressed store), and `#agent` (an LLM
session as files, with approvals as files).

**Mesh.** `wanix-9p-client` mounts a remote 9P export as a local filesystem (the
import half of Plan 9), and `wanix-mesh` carries 9P over [iroh](https://iroh.computer)
QUIC. Each node has a persisted ed25519 identity; attach is capability-gated
(default-deny grants); peers mount at `/n/<peer>`. Because every device is just a
filesystem, `#kv`/`#cas`/`#agent` and friends import across the mesh for free.
`#cpu` runs a task on a peer against the caller's reverse-exported namespace
(cpu(1) over the mesh), and `wanix capsule` freezes a world into a portable,
CAS-backed `.wcap`.

**Serve + cockpit.** The Rust `serve` exports Wanix filesystems over
stdio/TCP/WebSocket/HTTP with a discovery document, plus direct-v86 and native
QEMU handoffs and rootfs preparation. A browser cockpit (a Code OSS / VS Code
web extension under `workbench/`) operates the whole namespace over direct 9P:
inspect the service devices, repair a broken program through `#agent`, run a
qjs→wasm→qjs duet on one shared filesystem, serve HTTP apps at
`/.wanix/app/<name>` with `#kv`-backed state, and self-check the device set.

Try the native demo path from the workspace root (the first command compiles the
workspace, which takes a few minutes once):

```sh
cargo run --locked --package wanix-cli -- qjs examples/qjs-demo.js
cargo run --locked --package wanix-cli -- \
  qjs-term --stdin "hello terminal" examples/qjs-term-demo.js
printf 'write note.txt hello\nls\ncat note.txt\nexit\n' | cargo run --locked --package wanix-cli -- qjs-shell
# serve the browser cockpit (open the printed URL); serve needs an existing root:
mkdir -p /tmp/wanix-root
cargo run --locked --package wanix-cli -- \
  serve --root /tmp/wanix-root --bundle workbench-fs9p --wanix-services
just check
```

The Rust port's service devices, exposed as plain filesystems: `#task`, `#term`,
`#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`, `#cpu`.

The Rust workspace crates and active ADR index are documented in
[AGENTS.md](AGENTS.md). The mesh design is in
[docs/mesh-blueprint.md](docs/mesh-blueprint.md) and
[docs/mesh-the-missing-half-of-9p.md](docs/mesh-the-missing-half-of-9p.md);
hands-on recipes live in [docs/recipes/](docs/recipes/). For a hands-on path,
see [rust-walkthrough.md](rust-walkthrough.md). For the Go/Rust architecture
comparison, see [docs/rust-vs-go-wanix.md](docs/rust-vs-go-wanix.md). The
QuickJS/Wasmtime engine mechanics live in
[crates/wanix-qjs-engine](crates/wanix-qjs-engine), while Wanix process,
namespace, fd, and WASI policy stay in the Wanix crates above it.


### Install the Wanix Toolchain (legacy Go CLI)

These releases are the original Go toolchain from upstream; the Rust port has no
packaged release yet and is built from source as shown above. Download the Go
Wanix CLI from the [upstream latest release](https://github.com/tractordev/wanix/releases/latest)
or install with Homebrew:

```
brew install progrium/taps/wanix
```

If you want to build from source, see the [CONTRIBUTING.md](CONTRIBUTING.md) doc.

### Go Wanix File Services (legacy)

The original Go/browser Wanix has these built-in file services (the Rust port's
device set is listed in the Rust-port section above):

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

We'd love your contributions! Take a look at our [issues](https://github.com/tractordev/wanix/issues) to see how you can help out. You can also ask questions and participate in [discussions](https://github.com/tractordev/wanix/discussions), however right now most discussion takes place in our [Discord](https://discord.gg/nQbgRjEBU4).

Be sure to read our [CONTRIBUTING.md](CONTRIBUTING.md) doc to get started.

## Older Demos

* [📺 Wasm I/O 2024 Demo](https://www.youtube.com/watch?v=cj8FvNM14T4)
* [📺 Mozilla Rise 25 Demo](https://www.youtube.com/watch?v=KJcd9IckJj8)

## License

MIT
