# ADR 0058: Rust 9P Protocol And Server Contract

## Status

Accepted

## Context

9P is the main protocol path for Linux/v86/editor clients to browse and mutate a
Wanix namespace from outside the runtime. The Rust port needs a protocol stack
that keeps wire codecs, server fid state, filesystem semantics, and transports
separate enough to evolve safely.

The durable decision is the Rust-owned 9P contract and the boundaries between
wire codecs, server state, filesystem semantics, and transports.

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

- version negotiation for 9P2000.L, bounded `9P2000.L.Google.2`
  compatibility, and explicit rejection of unsupported versions;
- attach/walk/open/read/write/create/clunk/error basics;
- directory iteration with opaque cookies;
- metadata, statfs, permission, size, timestamp, symlink/readlink, hard-link,
  rename, remove, mkdir, and append behavior where the backing Wanix filesystem
  supports it;
- compatibility probes for flush, fsync, lock, auth, mknod, and xattr requests;
  and
- selected Google.2 operations needed by v86/Linux clients.

Stdio, TCP, WebSocket, and `serve` transports are adapters over the same server
contract. They must preserve binary frame boundaries and keep diagnostic/status
output out of binary response streams.

Unsupported features should return deliberate protocol errors until Wanix has a
backing contract. In particular, Rust Wanix does not currently require a
separate 9P auth phase, special-file creation, or extended attributes. Hard
links are exposed through the Wanix filesystem contract and may still return
unsupported errors for virtual filesystems that do not have shared-inode
semantics.

## Consequences

External 9P clients can mount or browse a Wanix namespace through native,
browser, v86, and editor paths without each transport inventing filesystem
semantics. Operation-by-operation coverage belongs in codec/server tests and
current-state docs, not in one ADR per operation.

Future 9P work should add ADRs only when it changes the protocol contract,
authentication/trust boundary, transport multiplexing model, or backing Wanix
filesystem semantics.
