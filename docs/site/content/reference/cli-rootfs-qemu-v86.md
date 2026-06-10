---
title: "CLI: rootfs, qemu, direct-v86 (VM Handoffs)"
slug: reference/cli-rootfs-qemu-v86
pageType: reference
oneLiner: "rootfs prepares and validates a guest root, qemu emits a wanix-qemu-virtio9p.v1 argv handoff, and direct-v86 generates a browser emulator page — launch contracts, not a VM supervisor."
audience: [developer]
tags: [cli, shipped, loopback-only, caveat, vm]
sourceRefs:
  - crates/wanix-cli/src/rootfs.rs:38-101
  - crates/wanix-cli/src/rootfs/prepare.rs:9-90
  - crates/wanix-cli/src/rootfs/archive.rs:65-101
  - crates/wanix-cli/src/rootfs/handoff.rs:32-58
  - crates/wanix-cli/src/qemu.rs:18-122
  - crates/wanix-cli/src/qemu/handoff.rs:84-92
  - crates/wanix-cli/src/qemu/parse.rs:10-56
  - crates/wanix-cli/src/serve/direct_v86.rs:10-128
  - docs/adrs/0005-serve-and-client-handoffs.md:50-68
  - rust-walkthrough.md:386-479
seeAlso: [reference/cli-command-index, concepts/loopback-only-handoffs, concepts/three-serve-bundles, concepts/discovery-document, reference/serve-and-discovery]
prerequisites: [concepts/loopback-only-handoffs]
usedInFlows: []
honestLimits:
  - "These commands emit and validate launch handoffs; they do not supervise a VM lifecycle, daemonize, or manage a running guest."
  - "rootfs build is out of scope: rootfs --archive only extracts and validates a pre-built guest root, it does not produce a kernel or filesystem image."
  - "qemu --exec is a foreground launch only and refuses to run unless the live wanix binary supplies process IO; --json cannot combine with --exec."
  - "The /.well-known/rootfs.json route is loopback-only because the manifest carries host paths and launch argv."
  - "Ethernet/vnet bridging is explicitly unimplemented; the v86 and QEMU 9P mounts are the only guest connectivity story today."
---

# CLI: rootfs, qemu, direct-v86 (VM Handoffs)

rootfs prepares and validates a guest root, qemu emits a `wanix-qemu-virtio9p.v1` argv handoff, and direct-v86 generates a browser emulator page — launch contracts, not a VM supervisor.

Wanix runs WASI tasks itself, but a full Linux guest is not a WASI task — it is a kernel that boots against a 9P root. Wanix does not own that boot. Instead, three CLI surfaces produce *validated handoffs*: `rootfs` checks that a guest root is shaped correctly, `qemu` turns that root into a launch command, and the `direct-v86` serve bundle hands a browser emulator the same boot facts. Each one stops at the edge of the VM. You — or your editor, browser, or launcher — own the process. This page is the map of those three surfaces and the stable manifest kinds they speak. ([ADR 0005](/reference/adr-index) records the boundary: "Explicit foreground launch is allowed, but background supervision, daemon lifecycle, and VM management are separate future decisions," `docs/adrs/0005-serve-and-client-handoffs.md:56-61`.)

## rootfs: prepare and validate a guest root

Start with a pre-built Linux guest archive and ask `rootfs` to lay it out and check it:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'
wanix rootfs --archive /path/to/linux-rootfs.tgz --out /tmp/wanix-rootfs
```

The visible result is an extracted directory plus a copyable launch summary: the resolved kernel route, the init route, and ready-to-run `qemu` and `serve` commands (`crates/wanix-cli/src/rootfs/handoff.rs:9-30`). Behind that, extraction rejects any archive entry that escapes the output directory (`../escape.txt`, absolute paths) with `unsafe rootfs archive path` before writing the escape (`crates/wanix-cli/src/rootfs/archive.rs:65-101`). Preparation then validates two boot markers: a kernel at `boot/bzImage` or `bzImage`, and an init at `bin/init` that must be a regular file and, on Unix, executable — a non-executable `/bin/init` is a hard error (`crates/wanix-cli/src/rootfs/prepare.rs:9-90`). This is the whole point: a guest that will not boot fails here, at prepare time, not later inside an opaque emulator.

For scripts and editors, `--json` emits the `wanix-rootfs.v1` manifest instead of prose (`crates/wanix-cli/src/rootfs.rs:38-86`):

```sh
wanix rootfs --archive /path/to/linux-rootfs.tgz --out /tmp/wanix-rootfs --json
```

It carries `rootPath`, `kernelRoute`/`kernelPath`, `initRoute`/`initPath`, an embedded `qemu` block, and a `serveDirectV86` argv with `bundle`, `wanixServices`, and `p9Msize` (`crates/wanix-cli/src/rootfs/handoff.rs:32-58`). The flag set is exactly `--archive FILE`, `--out DIR`, and `--json`; anything else is a usage error (`crates/wanix-cli/src/rootfs.rs:38-76`).

## qemu: emit a validated virtio-9p launch

`qemu --root` validates the same guest-root shape and prints a QEMU command that mounts the root over virtio-9p:

```sh
wanix qemu --root /tmp/wanix-rootfs
```

By default it produces a quoted shell argv. Add `--json` to get the `wanix-qemu-virtio9p.v1` manifest — `rootPath`, kernel/initrd paths, memory, `mountTag`, `securityModel`, `p9Msize`, the resolved `cmdline`, and the full `argv` (`crates/wanix-cli/src/qemu/handoff.rs:84-92`, manifest kind asserted at `crates/wanix-cli/src/rootfs/handoff.rs:127`). Shell and JSON share one validated argv, so a script and a future UI consume the same launch without reinterpreting a shell string ([ADR 0005](/reference/adr-index); `rust-walkthrough.md:470-474`).

The knobs all affect the guest boot contract, so they stay explicit (`crates/wanix-cli/src/qemu.rs:18-39`, `crates/wanix-cli/src/qemu/parse.rs:10-56`):

- `--kernel PATH` / `--initrd PATH` override discovery; a `/boot/initrd` in the prepared root is picked up automatically.
- `--mount-tag TAG` (default `host9p`) and `--p9-msize N` (default `131072`) both update the generated `root=TAG ... msize=N` cmdline.
- `--security-model` accepts `mapped-xattr`, `mapped-file`, `passthrough`, or `none`.
- `--memory-mb N`, `--no-kvm`, `--append TEXT` (repeatable), `--qemu-bin NAME`.
- `--cmdline TEXT` replaces the whole kernel cmdline; when you use it, you own the matching `root=TAG` and `msize=N` yourself.

`--exec` is a foreground launch, not a supervisor. The default cmdline expects an executable `/bin/init`; pass `--cmdline` when the guest owns a different init policy. Two guardrails are enforced: `--json` cannot combine with `--exec` (`crates/wanix-cli/src/qemu/parse.rs:34-39`), and `run_qemu_command` refuses `--exec` outright because exec needs live process IO supplied by the real binary path (`crates/wanix-cli/src/qemu.rs:47-92`).

## direct-v86: a browser emulator handoff

The third surface is a serve bundle, not a separate command:

```sh
wanix serve --root /tmp/wanix-rootfs --listen 127.0.0.1:7654 --bundle direct-v86
```

This serves the built-in v86 assets — `libv86.mjs`, the offscreen and re-export shims, `v86.wasm`, and the SeaBIOS/VGA BIOS blobs, all compiled into the binary via `include_bytes!` and served only when the bundle matches (`crates/wanix-cli/src/serve/direct_v86.rs:10-83`). The generated page reads the discovery document, attaches v86 to the advertised direct 9P route, and surfaces a boot-readiness JSON: it probes the static root for a kernel (`/boot/bzImage` or `/bzImage`), an initrd, and an executable `/bin/init`, then reports `ready` plus any `missing` markers (`crates/wanix-cli/src/serve/direct_v86.rs:85-128`). The default v86 cmdline carries the matching 9P rootflags (`crates/wanix-cli/src/serve/direct_v86.rs:11`); append `?bundle=direct-v86&p9-msize=65536` to a URL to change the mount size without rewriting the whole cmdline (`rust-walkthrough.md:429-430`).

## /.well-known/rootfs.json for loopback launchers

When `serve` points at a *prepared* root, loopback clients can fetch the same trusted-local handoff over HTTP:

```sh
curl http://127.0.0.1:7654/.well-known/wanix.json
curl http://127.0.0.1:7654/.well-known/rootfs.json
```

`/.well-known/rootfs.json` republishes the `wanix-rootfs.v1` manifest — host paths, boot markers, the QEMU manifest, and the direct-v86 serve argv — for local browser, editor, and VM launchers (`crates/wanix-cli/src/rootfs.rs:88-97`). Because the manifest includes host paths and launch argv, the route is loopback-only; this is the same trust posture as every other Wanix handoff (see [loopback-only handoffs](/concepts/loopback-only-handoffs)). The direct-v86 page itself consumes this route, renders a rootfs summary, and exposes the full manifest as `window.wanixRootfsHandoff` plus copyable QEMU and serve commands for trusted local tooling (`rust-walkthrough.md:425-428`).

## See also

- [CLI command index](/reference/cli-command-index) — every `wanix` subcommand in one place.
- [Loopback-only handoffs](/concepts/loopback-only-handoffs) — why these manifests never cross the network.
- [The three serve bundles](/concepts/three-serve-bundles) — `fs9p`, `workbench-fs9p`, and `direct-v86`.
- [The discovery document](/concepts/discovery-document) and [serve and discovery](/reference/serve-and-discovery) — what the v86 page reads on boot.
- [Wanix as host](/concepts/wanix-as-host) — where the runtime ends and the guest VM begins.

## Status / honest limits

These three surfaces emit and validate launch handoffs; none of them supervises a running VM, daemonizes, or manages guest lifecycle ([ADR 0005](/reference/adr-index)). `rootfs` does not *build* a guest root — it only extracts and validates a pre-built archive, rejecting unsafe paths and missing or non-executable boot markers (`crates/wanix-cli/src/rootfs/prepare.rs:9-90`). `qemu --exec` is a foreground launch that refuses to run without the live binary's process IO and cannot combine with `--json` (`crates/wanix-cli/src/qemu.rs:47-92`). `/.well-known/rootfs.json` is loopback-only by design, since the manifest carries host paths and argv (`crates/wanix-cli/src/rootfs.rs:88-97`). Ethernet/vnet bridging is explicitly unimplemented; the virtio-9p and direct-9p mounts are the only guest connectivity today.
