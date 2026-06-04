# ADR 0087: 9P Google.2 WalkGetAttr Compatibility

## Status

Accepted.

## Context

The Rust 9P server supports enough 9P2000.L for local mounts, v86 experiments,
and editor-facing serve work, but gVisor/progrium-style clients can negotiate
`9P2000.L.Google.N` extensions. Google.2 adds `Twalkgetattr`, a combined walk
and metadata request that avoids an immediate follow-up `Tgetattr`.

The extension is cumulative: Google.1 adds `Tflushf`/`Rflushf`, and Google.2
adds `Twalkgetattr`/`Rwalkgetattr`.

## Decision

`wanix-9p` accepts base `9P2000.L`, `9P2000.L.Google.1`, and
`9P2000.L.Google.2`. Requests for higher Google extension versions are capped
at `9P2000.L.Google.2`, because Rust Wanix only implements extensions through
Google.2 today.

`Tflushf` is implemented as a fid-validating no-op that returns `Rflushf` for an
existing fid and `EBADF` for an unknown fid. Wanix does not yet expose a deeper
per-file flush contract, so this preserves mounted-client compatibility without
inventing filesystem semantics too early.

`Twalkgetattr` reuses ordinary walk resolution and installs `newfid` only after
a successful full walk. Its returned metadata uses the same no-follow lookup and
session-virtual uid/gid ownership as `Tgetattr`.

`Rwalkgetattr` intentionally does not reuse the full `Rgetattr` payload. The
wire format is:

```text
valid[8]
attr body without qid
nwqid[2]
nwqid * qid[13]
```

Rust models that with a separate `P9AttrBody` so the missing qid is visible in
the type system.

Rust `serve` discovery keeps the existing base `protocol` field and adds a
supported protocol list containing `9P2000.L` and `9P2000.L.Google.2`. The
direct-v86 default Linux root flags remain on base `9p2000.L` until a guest boot
proof shows the kernel path accepts the Google extension string.

## Consequences

Mounted clients that understand the Google.2 dialect can reduce walk-plus-stat
round trips while still falling back to base 9P2000.L behavior when needed.
The Rust server can advertise a richer editor/browser protocol surface without
changing the conservative v86 boot handoff contract.

Future Google extensions must be added explicitly before increasing the
advertised cap beyond Google.2.
