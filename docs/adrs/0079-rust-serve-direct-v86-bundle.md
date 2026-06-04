# ADR 0079: Direct v86 Browser Handoff

## Status

Accepted

## Context

v86 is a browser emulator client of Rust Wanix. It needs a served page, browser
assets, boot hints, root filesystem access, and a visible console path, but it
does not make Wanix a browser runtime again and it is not a Wasmtime guest.

The direct-v86 path proves that Rust `serve` can hand a Linux guest a Wanix
filesystem through direct 9P while keeping VM supervision and richer networking
out of scope.

## Decision

`wanix-rust serve --bundle direct-v86` provides a browser handoff route for v86:

- the generated page consumes `/.well-known/wanix.json` and wires the advertised
  direct 9P WebSocket into v86 `filesystem.proxy_url`;
- serve owns or advertises the v86 module, wasm, BIOS, and helper asset routes
  needed by the generated page;
- discovery reports boot asset readiness for kernel and optional initrd routes
  found in the served root;
- discovery exposes boot hints such as the default 9P-root Linux cmdline,
  memory size, VGA memory size, and virtio-console expectation;
- query overrides can supply kernel, initrd, cmdline, and autostart behavior for
  deterministic smokes;
- the generated page exposes the guest `hvc0` virtio-console stream for visible
  browser boot and shell interaction; and
- lifecycle status, boot logs, resize delivery, and readiness markers exist for
  repeatable browser smoke tests.

This path is a browser/emulator handoff. Rust Wanix does not yet become a VM
supervisor, disk-image builder, network bridge, or browser authentication layer.

## Consequences

Rust Wanix has a direct browser VM route that exercises serve discovery, 9P,
boot assets, and terminal-like console I/O without tying the runtime foundation
back to Chrome.

Future v86 work should keep consuming serve discovery and should add ADRs only
when changing boot contracts, browser trust/auth policy, network/vnet behavior,
or VM lifecycle ownership.

## Replaces

This ADR consolidates ADRs 0083 through 0085, ADR 0090, and ADR 0100 into the
direct-v86 handoff contract.
