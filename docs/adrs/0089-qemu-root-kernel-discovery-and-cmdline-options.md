# ADR 0089: QEMU Root Kernel Discovery and Cmdline Options

## Status

Accepted

## Context

ADR 0088 added `wanix-rust qemu --root DIR --kernel PATH` as a print-only
handoff for booting the Wanix tiny Linux root with QEMU/KVM and virtio-9p. That
proved the native handoff shape, but it still made callers pass a kernel path
even when they were pointing at an extracted guest root that already contained
the expected `/boot/bzImage`.

Rust `serve --bundle direct-v86` already discovers `/boot/bzImage`, falling back
to legacy `/bzImage`, so the native QEMU handoff should share the same default
guest-root convention. Boot experiments also need a safe way to tune the kernel
command line without editing Rust constants.

## Decision

Make `wanix-rust qemu --root DIR` discover its kernel from the guest root:

- prefer `boot/bzImage`;
- fall back to `bzImage`; and
- keep `--kernel PATH` as an explicit override.

Add command-line controls for the guest kernel arguments:

- `--cmdline TEXT` replaces the default 9P-root command line; and
- repeatable `--append TEXT` appends extra arguments to the selected command
  line.

The command still prints a shell-quoted QEMU invocation instead of executing
QEMU. It continues to validate the root directory, discovered or explicit kernel
file, memory size, and the QEMU `-fsdev` comma boundary.

## Consequences

An extracted Wanix guest root is now enough to produce a native boot handoff:

```text
wanix-rust qemu --root rootdir
```

This aligns native QEMU with the direct-v86 boot discovery convention while
keeping process supervision, QEMU binary discovery, rootfs extraction, btrfs
image creation, and microvm variants as future cycles.
