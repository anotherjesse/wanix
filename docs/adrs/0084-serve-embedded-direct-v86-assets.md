# ADR 0084: Serve Embedded Direct v86 Assets

## Status

Accepted.

## Context

ADR 0083 made Rust `serve --bundle direct-v86` advertise and apply the Linux
9P-root boot contract, but the generated page still assumed that the selected
static root contained v86 browser assets at `/v86/lib/*` and `/v86/bundle/*`.
That made the demo brittle: serving an extracted guest root or another project
directory could produce the direct-v86 page without the v86 module, wasm, or
BIOS files it needed to start.

The Rust repository already carries the matching v86 browser assets. The direct
bundle should not require callers to copy those files into every served root.

## Decision

When `wanix-rust serve --bundle direct-v86` is active, Rust serve owns these
routes from embedded bytes:

- `/v86/lib/libv86.mjs` as the primary browser module advertised in discovery
- `/v86/lib/mod.js` as the compatibility re-export module
- `/v86/lib/offscreen.js`
- `/v86/bundle/v86.wasm`
- `/v86/bundle/seabios.bin`
- `/v86/bundle/vgabios.bin`

The generated direct-v86 page reads the v86 asset URLs from
`/.well-known/wanix.json`, dynamically imports the advertised primary module
URL, and uses the advertised wasm and BIOS URLs in the `new V86(...)` config.

For `--bundle direct-v86`, these embedded v86 routes take precedence over files
with the same paths in the static root. Other bundle names and ordinary static
serving continue to use the static root.

## Consequences

The direct-v86 demo no longer depends on the served filesystem root containing
the v86 browser runtime. A caller still needs to provide a usable Linux kernel
and root filesystem content, but the browser emulator assets are now part of the
Rust serve contract.

Embedding these assets slightly increases the CLI binary size. That is an
intentional tradeoff for a self-contained local v86 demo path.
