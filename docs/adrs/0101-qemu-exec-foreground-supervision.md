# ADR 0101: QEMU Exec Foreground Supervision

## Context

Rust Wanix already had a native QEMU/KVM virtio-9p handoff command that printed a
validated shell-quoted argv for the same guest-root shape used by direct-v86:
`/boot/bzImage`, `console=hvc0`, `root=host9p`, and a host directory exported as
the root filesystem.

After adding direct-v86 boot smoke readiness, the native QEMU side was the next
handoff-grade gap. Users still had to copy a generated command into a shell
before they could try the same root outside Chrome.

## Decision

Add `wanix-rust qemu --exec` as an explicit opt-in foreground launch mode:

- Without `--exec`, `wanix-rust qemu` keeps printing the stable shell-quoted
  argv.
- With `--exec`, the CLI builds the same validated argv, writes the quoted
  command to stderr for observability, spawns the configured QEMU binary, and
  returns QEMU's exit status.
- The child process inherits stdin, stdout, and stderr so QEMU's `-nographic`
  and `hvc0` console path can own the foreground terminal.
- Captured CLI mode rejects `--exec`; live process IO is required.

This is foreground process supervision, not a daemon, VM manager, rootfs
assembler, network bridge, or terminal multiplexer.

## Consequences

The native VM path now has a runnable command:

```text
wanix-rust qemu --root rootdir --exec
```

Future cycles can add QEMU binary discovery, rootfs extraction from
`extras/dist`, signal/TTY policy, vnet/ethernet integration, or richer VM
lifecycle controls without changing the stable print-only argv contract.
