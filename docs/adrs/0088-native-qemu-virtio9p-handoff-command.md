# ADR 0088: Native QEMU Handoff

## Status

Accepted

## Context

Native QEMU is another client of the Rust Wanix filesystem and boot workflow.
It should be easy to launch a guest root with virtio-9p, but the Rust port is
not yet a full VM manager.

The important boundary is that Wanix can construct a validated handoff command
and optionally supervise a foreground process, while leaving daemonization,
terminal multiplexing, rootfs assembly, network bridging, and lifecycle policy
for later decisions.

## Decision

`wanix-rust qemu --root DIR` builds a validated native QEMU/KVM virtio-9p
handoff command:

- it canonicalizes and validates the guest root;
- it discovers kernel and optional initrd paths from the root when present;
- it supports explicit kernel/initrd paths, cmdline override, and cmdline
  append options;
- it prints a shell-quoted command by default, preserving reviewable
  print-only behavior; and
- it advertises the same guest-root shape used by the direct-v86 route where
  practical.

`wanix-rust qemu --exec` is an explicit foreground launch mode. It spawns the
validated argv, inherits stdin/stdout/stderr, reports the child exit status, and
does not turn Wanix into a background VM supervisor.

## Consequences

Developers can move from Rust-generated guest-root artifacts to native QEMU
without hand-writing fragile commands, while the default mode remains inspectable
and safe to paste or modify.

Future QEMU work that adds daemon mode, persistent VM management, network/vnet,
terminal multiplexing, or rootfs assembly should get its own ADR because it
changes the ownership boundary.

## Replaces

This ADR consolidates ADR 0089 and ADR 0101 into the native QEMU handoff
contract.
