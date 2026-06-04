# ADR 0083: Serve Direct v86 Boot Contract

## Status

Accepted.

## Context

ADR 0071 established Rust `serve` discovery, and ADR 0079 added
`serve --bundle direct-v86` as a generated browser page that imports v86 and
discovers the Rust direct 9P WebSocket route. That proved route wiring, but the
generated v86 configuration did not include the kernel command line and console
settings that tell the Wanix Linux guest to mount the Rust 9P export as its root
filesystem.

The existing Go v86 path boots the tiny Alpine guest with `console=hvc0`,
`init=/bin/init`, `root=host9p`, `rootfstype=9p`, and
`rootflags=trans=virtio,version=9p2000.L,aname=,cache=none,msize=131072`.
Without those defaults, the Rust page can connect to v86 but does not describe
the actual Wanix 9P-root boot contract.

## Decision

Expose the direct-v86 boot defaults in Rust `serve`:

- `/.well-known/wanix.json` now includes a `v86` object with the default kernel
  command line, memory size, VGA memory size, and `virtioConsole` hint.
- The generated direct-v86 page reads those hints from discovery, renders the
  command line as an editable control, and passes it to `new V86(...)`.
- The generated v86 config enables `virtio_console`, disables the speaker and
  mouse, sets memory defaults, and keeps `filesystem.proxy_url` pointed at the
  direct Rust 9P WebSocket route.
- Query parameters can override the kernel URL with `kernel` or `bzimage`,
  set `initrd`, replace `cmdline`, or append additional kernel arguments with
  `append`.

The page still assumes the caller serves v86 assets and a usable kernel from the
same static root or passes URLs through query parameters.

## Consequences

The Rust direct-v86 bundle is now a boot handoff proof, not only a WebSocket
route wiring proof: clients can discover both the 9P route and the guest kernel
parameters needed to mount it.

This does not yet assemble or fetch kernel/rootfs assets, implement Ethernet or
vnet, add public authentication policy, or prove a full browser VM boot in CI.
