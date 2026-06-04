# ADR 0068: Serve and Client Handoffs

## Status

Accepted

## Context

`wanix-rust serve` is the local composition surface for browser filesystem,
workbench, VS Code, v86, QEMU-launcher, and local tool clients. It needs to
serve static assets, discovery documents, and protocol routes from one listener
without making the 9P server or core runtime own HTTP, WebSocket, browser
isolation, or demo-page policy.

Native QEMU and direct-v86 are VM handoffs over the same prepared-root and
9P-root assumptions. They are launch/attachment contracts, not evidence that
Wanix has become a VM supervisor or that the browser has become the runtime
foundation again.

## Decision

Rust `serve` combines static HTTP, discovery, and direct protocol routes on one
local listener:

- static responses and bundle assets use the local browser headers needed by
  v86, workbench, and filesystem demos;
- `/.well-known/export9p` is the reserved direct binary 9P WebSocket route;
- `/.well-known/wanix.json` is the reserved discovery document for routes,
  selected bundle metadata, service availability, boot hints, and explicit
  placeholders for unimplemented routes such as Ethernet/vnet;
- `/.well-known/rootfs.json` is the reserved prepared-root handoff document for
  trusted local clients. Because the manifest includes absolute local paths and
  launch argv, it returns `wanix-rootfs.v1` only to loopback clients when the
  served root satisfies the VM boot-marker contract;
- deterministic single-connection serve mode may exist for tests and scripted
  clients, but normal serve accepts concurrent HTTP and 9P WebSocket clients;
  and
- service mode exports a Wanix namespace containing the served root, `#task`,
  and `#term` through direct 9P from a service-root task context.

Generated bundle pages and browser clients should consume the discovery document
rather than hard-coding route assumptions.

Browser filesystem and workbench clients are frontend integrations over the
serve contract. Rust-hosted workbench integrations should discover serve, browse
or mutate `wanix:/` over direct 9P, and use `#task` command/env/dir/fd files and
`#term` service files for tasks and terminals. The legacy MessagePort/CBOR
workbench bridge remains a compatibility path for browser-embedded Wanix
systems, not the Rust-hosted direction.

Direct v86 is a browser/emulator handoff over Rust serve. The generated page
should consume discovery, attach v86 to the advertised direct 9P route, use
served or advertised v86 assets, and surface boot readiness without inferring
filesystem layout.

Native QEMU is a validated command handoff over the same prepared-root shape.
Rootfs preparation validates archive safety and VM boot markers, machine-readable
manifests use stable handoff kinds such as `wanix-rootfs.v1`, and QEMU launch
manifests use stable argv/policy data such as `wanix-qemu-virtio9p.v1`. Explicit
foreground launch is allowed, but background supervision, daemon lifecycle, and
VM management are separate future decisions.

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
