# ADR 0088: Native QEMU Virtio-9P Handoff Command

## Status

Accepted.

## Context

Wanix now has a Rust `serve` path for browser/v86 experiments, but the same tiny
Linux guest should also be runnable outside Chrome on Linux hosts with QEMU/KVM.
The current guest boot model already matches a virtio-9p root: kernel
`/boot/bzImage`, `console=hvc0`, `root=host9p`, and a host directory exported as
the root filesystem.

Actually supervising QEMU is a larger workflow decision: process lifecycle,
host TTY handling, KVM availability, rootfs extraction, and future image modes
all need more design.

## Decision

Add `wanix-rust qemu --root DIR --kernel PATH`, a native CLI handoff command
that prints a shell-quoted `qemu-system-i386` virtio-9p command instead of
executing it. The printed command uses:

- `-enable-kvm -cpu host` by default, with `--no-kvm` for non-KVM hosts.
- A configurable memory size with `--memory-mb`, defaulting to `512`.
- `-fsdev local,id=host9p,path=DIR,security_model=mapped-xattr`.
- `virtio-9p-pci` with `mount_tag=host9p`.
- `virtio-serial-pci` plus `virtconsole,chardev=con`.
- `-chardev stdio,id=con -nographic`.

The command validates that `DIR` exists and is a directory and that `PATH`
exists and is a file. It canonicalizes both paths before printing. It rejects a
root path containing `,` because QEMU `-fsdev` options are comma-separated and a
comma would change the option boundary.

The guest root flags stay on base `version=9p2000.L`. Google.2 9P negotiation is
implemented in the Rust server, but the QEMU guest path should not switch
dialects until a boot proof shows the kernel and userspace accept it.

## Consequences

Rust Wanix now has a tested native QEMU handoff artifact that points the same
guest/rootfs story outside the browser without committing to a VM supervisor.

Later cycles added `--exec` and `wanix-rust rootfs` archive extraction. Future
cycles can add QEMU binary discovery, rootfs build automation, btrfs image
generation, microvm/qboot variants, or process/terminal supervision on top of
this stable generated argv contract.
