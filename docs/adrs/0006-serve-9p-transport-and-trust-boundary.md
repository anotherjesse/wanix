# ADR 0006: Serve 9P Transport and Trust Boundary

## Status

Accepted

## Context

The Rust `serve` 9P edge had grown three near-identical per-connection session
loops: the websocket door inside `serve` duplicated the frame buffer / encode /
write loop that already lived in `wanix-9p`, and the standalone `p9-listen`
(raw TCP) and `p9-ws` (websocket) subcommands each reimplemented the same
accept/serve loop on top of it. The websocket door was also the only way to
write a service file (e.g. `#sites/<host>`) against a live `serve`, because
`p9-listen` exports raw 9P but never exposed `--wanix-services`, and the CLI's
`mount-*` verbs speak raw TCP 9P only — so a running `serve` could not be
mutated from the CLI.

That same serve 9P edge was unauthenticated: `P9Server::new(root)` with no
`AttachPolicy`, and `--wanix-services` binds the `#task`/`#agent` exec devices,
which is remote code execution. `serve --listen :PORT --wanix-services` silently
exposed RCE on every interface.

## Decision

There is **one per-connection 9P session core**, and the transports are thin
adapters over it:

- `P9Server::serve_duplex<D: Read + Write>` owns the single session loop;
  `P9Server::serve_stream<R: Read, W: Write>` delegates to it through an internal
  `DuplexPair`. The loop body is written once and never reads and writes
  concurrently.
- The websocket door is a `WebSocketDuplex` framing adapter (HTTP upgrade, ws
  de/masking, reassembly, Ping→Pong inside `read`, Close→`Ok(0)`) handed to that
  same `serve_duplex`. It is not a second server. 9P frames split across ws
  messages are reassembled by the core's frame buffer.
- Raw 9P over TCP is a **serve mode** (`serve --p9 ADDR`), not a separate
  subcommand. It exports the served namespace (`Arc::clone` of the serve 9P
  root) on a dedicated blocking accept thread, default-bound to loopback, and
  reports its bound address in startup status and discovery (`routes.p9.tcp`),
  so `mount-write tcp://HOST:PORT '#sites/<host>' 'dir /abs'` works against a
  live `serve`.
- The standalone `p9-listen` and `p9-ws` subcommands are **retired**. Their
  reusable logic (grant/policy plumbing, the accept/serve loop) is preserved and
  reused by serve under `serve/raw9p`.

The serve 9P edge carries the **trust boundary**:

- `--wanix-services` is refused whenever either 9P door (the websocket door,
  which rides the HTTP listener, or the raw `--p9` door) is bound to a
  non-loopback address, mirroring the mesh exec-device rule. RCE devices never
  reach a non-loopback peer.
- The raw `--p9` door may be capability-gated with `--peer HEX` and
  `--grant ANAME:PREFIX:RIGHTS`, building the server through
  `P9Server::with_policy` (default-deny `AttachPolicy`); `--grant` requires
  `--peer`, and a policy without `--p9` is a usage error.
- `--addr` is deprecated in favor of `--listen` (kept as a hidden parsing
  synonym so existing scripts/tests pass).

## Consequences

- `p9-ws` and `p9-listen` are gone; raw-9P export and websocket 9P are both
  serve surfaces over one session core. The duplicated session loops are
  removed.
- `mount-write tcp://...` mutates a live `serve` namespace; the in-Wanix sites
  transport gap is closed.
- `p9-stdio` (the pipe / QEMU-v86 console bridge) and `mesh-serve` (iroh
  cryptographic trust across machines) are unaffected — they are separate
  transports with their own trust models, not redundant with this edge.
- The previously silent `--listen :PORT --wanix-services` RCE hole is now a hard
  usage error.

Exact flag spellings, JSON fields, and per-test proofs belong in tests,
examples, and commit messages. Future browser/VS Code auth policy, multi-user or
per-principal serve namespaces, and remote (non-loopback) exposure of the 9P
edge change this trust boundary and should be recorded as new decisions.
