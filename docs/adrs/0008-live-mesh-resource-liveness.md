# ADR 0008: Live Mesh Resource Liveness

## Status

**Accepted.** This records the user-visible liveness contract for live
`iroh://` resources imported through the native mesh wire: volumes, tools,
services, and peer namespaces that are mounted into an agent or shell namespace.
ADR 0004 owns the wire shape; ADR 0007 owns the catalog/resource story. This ADR
owns what a live resource should *feel like* when the provider disappears,
reappears, or flakes.

## Context

An `iroh://PEER[?addr=...]` resource is a live reference, not a snapshot. The
peer id is the resource identity; any `addr=` values are route hints. A mounted
resource may go away because a `volume serve` process was killed, a laptop slept,
Wi-Fi changed, or a route hint went stale. Agents must be able to distinguish
that from ordinary filesystem state such as `ENOENT`.

The native mesh wire already uses one QUIC stream per filesystem operation and
one stream per open file. That avoids 9P-style head-of-line blocking, but it also
means liveness has two levels:

- the **mount** (`NativeFs`) is a durable namespace binding and can reconnect on
  a later operation;
- an **open file handle** owns one stream and cannot be transparently moved to a
  new provider process.

## Decision

### Hard mounts fail closed

An explicit CLI mount such as `qjs-shell --mount-mesh IROH=/vol/notes` is a
required dependency. If the initial dial cannot establish the requested peer
identity within the mount deadline, the command fails instead of starting with a
placeholder directory. Future catalog/recipe launches may add "soft mounts" with
offline placeholder files, but the low-level CLI flag is hard by default.

Current numbers:

- mesh core default operation deadline: **30 seconds** (`wanix-mesh`);
- foreground CLI mesh mount deadline: **5 seconds** (`wanix-cli` mount/dial
  helper);
- tests may override nodes to shorter deadlines, e.g. **250 ms**, to exercise
  the contract without making the suite wait.

### The mount survives; operations fail

A mounted `NativeFs` stays bound across outages. A fresh filesystem operation
opens a new QUIC bidi stream. If the cached connection is dead before a request
is sent, the stream factory may reconnect once by the stable peer id and retry
opening the stream. If the provider is still down, the operation returns the
typed mesh transport error (`FsError::Unreachable`), which WASI/QJS maps to
`EIO` instead of `ENOENT`.

The stringly `Other("mesh: ...")` encoding was debt, now paid: transport
unreachability is the typed `FsError::Unreachable(detail)` (mirrored
losslessly by `WireFsError::Unreachable`), constructed at the native-wire and
9P-client transport edges, mapped to `EIO` by WASI and to errno 5 by the 9P
server edge. Callers key liveness behavior off the variant; the detail string
is diagnostics only. Richer detail (attempt counts, retry-after) belongs in
the liveness status files below, speaking the shared error taxonomy of
ADR 0009.

When the same peer identity returns, later fresh operations should work without
the caller rebuilding the namespace binding. A stale `addr=` hint must not pin
the mount to the old port; it remains only a route hint, and the authenticated
peer id is the authority.

### Do not replay in-flight work

Once a request may have been sent, Wanix does not automatically replay it after a
timeout or connection loss. This is especially important for mutating operations:
a write may have reached the provider even if the reply did not. The correct
surface is a transport error and caller policy.

Open file handles are not resurrected. If a provider dies while a file handle is
open, reads and writes on that old handle return a mesh transport error within
the operation deadline. They must not return EOF to hide the failure, and they
must not silently reconnect to a new provider process. The recovery move is to
close the stale handle and open the path again.

### Flaky providers need backoff state

The v0 client may probe on each foreground operation. The intended agent-facing
next step is a small mount liveness state machine:

- first reconnect attempt immediately, then **250 ms** initial backoff;
- exponential backoff with jitter, capped at **30 seconds**;
- mark the resource offline after **3 consecutive failures** or **30 seconds**
  since last success, whichever is clearer for the caller surface;
- reset to online on any successful operation;
- expose the state as files before inventing callbacks.

This state machine should not change the no-replay rule. It schedules probes and
surfaces status; it does not decide that a timed-out mutation is safe to repeat.

## Consequences

Agents get a simple rule: a live mesh path that returns `EIO` is temporarily
unusable, not missing. They can retry by opening the path again, inspect liveness
state once it exists, or choose another resource. Shells stay responsive because
foreground CLI mounts use a 5s deadline. Shell helpers should preserve the
same distinction in their text surface: `ENOENT` may be rendered as "not found",
but a provider outage should remain an I/O errno so agents do not mistake a
missing resource for a missing file.

The native wire remains transport-authenticated. A wrong or stale route hint may
fail or rediscover the real peer, but it never authorizes a different process as
the requested resource because the QUIC endpoint id must match.

This identity model carries more weight than outage recovery: because the peer
id is the authority and route hints are disposable, a mounted namespace is
*location-independent by construction* — which is exactly what lets a tier-1
task snapshot (ADR 0010) be restored on a different node and re-establish its
mounts by peer identity. This ADR's liveness contract is, quietly, the task
migration contract.

The first regression suites are
`crates/wanix-cli/tests/mesh_iroh_resilience.rs` (missing-provider dial failure,
provider outage with later recovery, stale direct hint recovery, and stale
open-file handle failure) and the qjs mount tests in
`crates/wanix-cli/src/qjs_support/mod.rs` (QJS `os.open` sees mesh outage as
`EIO` and recovers on a later fresh open). The iroh suites currently inherit
iroh's slow endpoint teardown behavior (~30s for these integration cases); that
cost is a test and lifecycle hardening target, not part of the desired
user-facing contract.

## Open questions

- How should a future soft catalog mount represent offline state as files
  without confusing existing path walkers?
- Should the mesh core split deadlines by plane (`FileSystem` foreground ops vs.
  long-running `#cpu`/agent control streams) instead of one node deadline?
- Can `MeshNode` shut down failed or stale iroh endpoints without waiting for
  iroh's graceful QUIC drain, while still closing healthy long-lived nodes
  politely?
