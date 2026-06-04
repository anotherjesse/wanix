# ADR 0058: Rust 9P Protocol And Server Contract

## Status

Accepted

## Context

9P is the main protocol path for Linux/v86/editor clients to browse and mutate a
Wanix namespace from outside the runtime. The Rust port needs a protocol stack
that keeps wire codecs, server fid state, filesystem semantics, and transports
separate enough to evolve safely.

The old ADR set recorded many individual operation milestones. Those tests are
valuable, but the durable decision is the Rust-owned 9P contract and the
boundaries around it.

## Decision

`wanix-protocol` owns dependency-free 9P frame splitting, tag extraction,
version negotiation, and typed operation codecs. It covers the server-facing
9P2000.L surface plus selected Google.2 compatibility codecs when clients need
them.

`wanix-9p` owns the server state that maps fids to Wanix filesystem objects and
open-file state. It translates filesystem results into 9P replies and Linux-ish
errno errors while leaving listener, socket, stdio, and browser policy to
adapters.

The supported contract includes:

- version negotiation for 9P2000.L, capped compatibility for
  `9P2000.L.Google.2`, and explicit rejection of unsupported versions;
- attach, walk, clunk, open, create, read, write, and error replies;
- directory reads using opaque one-based cookies;
- metadata through `Tgetattr`, POSIX file-type mode bits, virtual uid/gid, and
  permission updates where Wanix filesystems support them;
- `Tstatfs` synthetic mount probes;
- file and directory mutations including create, mkdir, unlink, remove,
  rename, legacy fid-oriented rename/remove, symlink, and readlink;
- setattr for size, access/modification times, and permissions;
- append mode as opened-fid state, so writes append at EOF regardless of client
  offsets;
- compatibility probe handling for `Tflush`, `Tfsync`, lock/getlock, auth,
  mknod, hard-link, and xattr requests; and
- Google.2 `Twalkgetattr`/`Rwalkgetattr` and `Tflushf`/`Rflushf` compatibility
  where useful for v86/Linux clients.

Stdio, TCP, WebSocket, and `serve` transports are adapters over the same server
contract. They must preserve binary frame boundaries and keep diagnostic/status
output out of binary response streams.

Unsupported features should return deliberate protocol errors until Wanix has a
backing contract. In particular, Rust Wanix does not currently require a
separate 9P auth phase, special-file creation, hard links, or extended
attributes.

## Consequences

External 9P clients can mount or browse a Wanix namespace through native,
browser, v86, and editor paths without each transport inventing filesystem
semantics. The compatibility surface is testable at the codec, server, CLI, and
serve layers.

Future 9P work should add ADRs only when it changes the protocol contract,
authentication/trust boundary, transport multiplexing model, or backing Wanix
filesystem semantics.

## Replaces

This ADR consolidates ADRs 0059 through 0067, ADRs 0072 through 0082, and
ADR 0087 into the Rust 9P protocol and server contract.
