# ADR 0085: Serve Direct v86 Boot Asset Discovery

## Status

Accepted.

## Context

The Rust direct-v86 bundle can now serve its own emulator runtime assets, but
booting a Linux guest still requires a kernel from the served root. The original
page defaulted to `/bzImage`, while the Wanix tiny-Linux and QEMU notes use the
kernel inside an extracted root filesystem at `/boot/bzImage`.

That mismatch made the direct-v86 page easy to launch but easy to misconfigure:
serving an extracted guest root could still produce a page whose default kernel
URL did not exist.

## Decision

`/.well-known/wanix.json` includes a `v86.boot` object derived from files in the
served static root:

- `kernel` prefers `/boot/bzImage`, falling back to `/bzImage`.
- `initrd` uses the first present initrd route from `/boot/initrd`,
  `/boot/initrd.img`, `/initrd`, or `/initrd.img`.

The generated direct-v86 page applies those discovered URLs before query-string
overrides. Query parameters remain authoritative for ad hoc experiments.

## Consequences

Serving an extracted Wanix Linux root with `boot/bzImage` is now a stronger
out-of-the-box v86 handoff. The browser page no longer assumes a legacy top-level
kernel path when the served root already exposes the kernel in the layout used
by the guest filesystem and QEMU/KVM notes.

This is discovery only. Rust serve still does not assemble a root archive,
download kernels, or implement the Ethernet/vnet bridge.
