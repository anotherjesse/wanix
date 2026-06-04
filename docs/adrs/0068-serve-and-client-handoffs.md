# ADR 0068: Serve And Client Handoffs

## Status

Accepted

## Context

`wanix-rust serve` is the local composition surface for browser, v86,
workbench, VS Code, and local tool clients. It needs to serve static assets and
protocol routes from one listener without making the 9P server or core runtime
own HTTP, WebSocket, browser isolation, or demo-page policy.

Native QEMU is the corresponding non-browser VM handoff. It shares the same
guest-root and 9P-root assumptions as direct-v86 but remains an inspectable
command handoff rather than a VM manager.

## Decision

Rust `serve` combines static HTTP, discovery, and direct protocol routes on one
local listener:

- `wanix-rust serve [DIR] [--listen HOST:PORT] [--once] [--bundle NAME]
  [--wanix-services]` serves a selected root, defaulting to the current
  directory and a demo-friendly local address.
- Static HTTP responses use the browser headers needed by local v86 and
  workbench demos.
- `/.well-known/export9p` is the reserved direct binary 9P WebSocket route.
- `/.well-known/wanix.json` is the reserved discovery document for direct 9P,
  selected bundles, service availability, direct-v86 boot hints, and explicit
  placeholders for unimplemented routes such as Ethernet/vnet.
- `--once` remains a deterministic single-connection mode for tests and
  scripted clients; normal serve accepts concurrent HTTP and 9P WebSocket
  clients.
- `--wanix-services` exports a Wanix namespace containing `#task` and `#term`
  through direct 9P from a service-root task context. Service mode may register
  task drivers such as `noop` and `qjs`, and may expose terminal-backed shell
  routes that are still backed by `#task` and `#term`.

Generated bundle pages and browser clients should consume the discovery document
rather than hard-coding route assumptions.

Browser filesystem and workbench clients are frontend integrations over that
serve contract:

- `serve --bundle fs9p` exposes a browser filesystem page for direct 9P
  browse/read/write behavior against the served root.
- The workbench extension can back its `wanix:` filesystem provider with direct
  9P operations, plus bounded client-side search over `wanix:/`.
- `serve --bundle workbench-fs9p` is a local generated VS Code web workbench
  launch path that points the extension at Rust serve discovery.
- The legacy MessagePort/CBOR workbench bridge is a compatibility path for
  browser-embedded Wanix systems; Rust serve discovery and direct 9P are the
  Rust-hosted workbench direction.
- Workbench tasks and terminals should use the same `#task` command/env/dir/fd
  files and `#term` service files as native clients.

Direct v86 is a browser/emulator handoff over Rust serve:

- `serve --bundle direct-v86` consumes discovery and wires the advertised
  direct 9P WebSocket into v86 `filesystem.proxy_url`;
- serve owns or advertises the v86 module, wasm, BIOS, and helper asset routes
  needed by the generated page; and
- discovery reports boot readiness and explicit launch hints without making the
  browser page infer filesystem layout.

Native QEMU is a validated command handoff:

- `wanix-rust rootfs --archive FILE.tgz --out DIR` extracts a guest root into a
  missing or empty directory, rejects unsafe archive paths, validates shared VM
  boot markers, and prints ready-to-run QEMU and direct-v86 handoffs.
- `wanix-rust rootfs --archive FILE.tgz --out DIR --json` prints the same
  prepared root as a machine-readable `wanix-rootfs.v1` manifest, including
  root path, boot marker routes, a nested default native QEMU handoff, and
  direct-v86 serve argv.
- `wanix-rust qemu --root DIR` canonicalizes and validates the guest root,
  discovers VM boot markers, applies explicit command overrides, validates local
  9P security policy, and prints a shell-quoted QEMU/KVM virtio-9p command by
  default.
- `wanix-rust qemu --json` prints the same validated handoff as
  `wanix-qemu-virtio9p.v1`.
- `wanix-rust qemu --exec` is an explicit foreground launch mode. It spawns the
  validated argv, inherits stdin/stdout/stderr, reports the child exit status,
  and does not turn Wanix into a background VM supervisor.

## Consequences

Rust Wanix has one local discovery and handoff story for browser filesystem,
workbench, v86, and native QEMU clients. These paths exercise the bigger pieces
without turning `serve` into a VM manager, editor host, network bridge, or core
filesystem crate.

Exact bundle pages, JSON fields, and command flags belong in tests, examples,
and walkthrough docs. Future auth, remote exposure, Ethernet/vnet,
multiplexing, persistent session policy, daemon mode, rootfs build ownership, or
VM lifecycle management should be recorded as new decisions because they change
the serve/client trust boundary.
