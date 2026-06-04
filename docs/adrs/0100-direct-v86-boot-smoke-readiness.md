# ADR 0100: Direct v86 Boot Smoke Readiness

## Context

Rust `serve --bundle direct-v86` already generated a browser v86 launcher with
embedded emulator assets, 9P WebSocket discovery, boot asset hints, and an hvc0
console bridge. That made the page useful for manual handoff, but it was still
awkward as a repeated runtime smoke: a caller had to click Start, boot readiness
was implicit, and v86 lifecycle progress was not surfaced in a stable page
contract.

The next big Wanix runtime proof is a browser VM boot against the same
guest-root shape that native QEMU targets. That proof should exercise Rust
`serve`, direct 9P, Linux/v86 compatibility, and terminal output together before
we add speculative 9P features or broad VM supervision.

## Decision

Make the generated direct-v86 page smoke-friendly:

- `?autostart=1` starts v86 after discovery and configuration are loaded.
- The page keeps a visible boot log and mirrors it on `window.wanixV86BootLog`
  for browser smoke automation.
- v86 lifecycle and download events update visible status.
- The hvc0 console bridge also sends `virtio-console0-resize` events derived
  from the browser console size.
- Discovery reports boot readiness from the served root: detected kernel,
  optional initrd, `/bin/init`, a boolean `ready`, and missing required markers.

This does not make Rust `serve` a full VM supervisor. It provides a stable
browser smoke entry point that can reveal the next real 9P or guest-boot
blocker.

## Consequences

The direct-v86 path now has a concrete local smoke target:

```text
/?bundle=direct-v86&autostart=1
```

Future 9P compatibility work should be driven by boot traces from this path or
from the native QEMU parity path, not by speculative protocol completeness.
Native QEMU remains a command handoff until a later cycle adds process
supervision.
