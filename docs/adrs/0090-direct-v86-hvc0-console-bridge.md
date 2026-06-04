# ADR 0090: Direct v86 hvc0 Console Bridge

## Status

Accepted

## Context

Rust `serve --bundle direct-v86` can already generate a browser page that loads
v86 assets, discovers the direct 9P WebSocket route, and configures the Wanix
9P-root Linux boot handoff. The guest boot contract uses `console=hvc0` and the
Alpine init script attaches the shell to `/dev/hvc0`. QEMU exposes the same path
with `virtconsole,chardev=con`.

The Rust browser page enabled `virtio_console`, but it did not bridge v86
`virtio-console0-output-bytes` to visible page output or send browser input back
through `virtio-console0-input-bytes`. That made a successful browser boot hard
to observe and left the shell unusable from the generated page.

## Decision

Bridge the direct-v86 page's first virtio console to a visible textarea:

- append `virtio-console0-output-bytes` to the page console;
- send printable key presses, enter, tab, backspace/delete, Ctrl-D, and pasted
  text through `virtio-console0-input-bytes`; and
- expose the created emulator as `window.wanixV86` for local debugging.

This remains a page-level browser/v86 composition detail. It does not add a
general terminal emulator, vnet bridge, VM process supervisor, or boot verifier.

## Consequences

`wanix-rust serve --bundle direct-v86` now produces a browser handoff that can
show the same `hvc0` stream QEMU maps to native stdio. A booted Wanix guest can
be observed and lightly interacted with from the generated page while later
cycles work on real boot smokes, richer terminal behavior, and network bridges.
