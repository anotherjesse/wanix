# Rust Wanix And The Original Go Wanix

Wanix began as a Go/browser implementation. The original system treated the
browser as the runtime host: Go compiled to WebAssembly, browser APIs backed
parts of the namespace, and UI surfaces such as terminals, v86, and workbench
demos lived close to the page.

That original project lives at https://github.com/tractordev/wanix/. It remains
the historical reference for many Wanix ideas, but its Go runtime, browser
custom elements, and Go build pipeline are no longer carried in this workspace.

In this repository, plain `wanix` means the Rust-native implementation:

- a native CLI binary built by `cargo build --locked --package wanix-cli`;
- Wasmtime as the execution substrate;
- QuickJS/WASI and compiled `wasm32-wasi` task drivers;
- Wanix-owned task identity, namespace, fd, stdio, env/cwd/cmd, and exit
  semantics;
- service devices such as `#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`,
  `#agent`, and `#cpu`;
- a native FileSystem-over-iroh mesh for Wanix-to-Wanix imports;
- 9P, HTTP, direct-v86, QEMU, and the browser cockpit as frontends or foreign
  edges, not as the runtime foundation.

The host-boundary shift is the core architectural change:

- The Go/browser Wanix asked whether the browser could be the operating
  environment.
- Rust Wanix asks whether Wanix can be the operating environment, with the
  browser as one useful client.

That difference drives the crate boundaries in [AGENTS.md](../AGENTS.md), the
ADR set under [docs/adrs/](adrs/), and the current command examples throughout
the docs: use the plain `wanix` CLI name.
