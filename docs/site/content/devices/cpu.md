---
title: "#cpu — Exec Plane (cpu(1) over the Mesh)"
slug: devices/cpu
pageType: reference
oneLiner: A caller reverse-exports a scoped read-only namespace and a remote acceptor runs a task whose world is that export — Plan 9 cpu(1) over QUIC, grant-allowlisted.
audience: [developer]
tags: [mesh, exec, local-trust-only, caveat, cli]
sourceRefs:
  - crates/wanix-cpu/src/lib.rs:1-72
  - crates/wanix-cpu/src/scope.rs:56-126
  - crates/wanix-cpu/src/acceptor.rs:74-114
  - crates/wanix-cpu/src/role.rs:17-95
  - crates/wanix-mesh/src/cpu/handler.rs:42-138
  - crates/wanix-mesh/src/cpu.rs:39
  - crates/wanix-mesh/src/node.rs:50
  - crates/wanix-cli/src/cpu/run.rs:30-101
  - crates/wanix-cli/src/cpu/parse.rs:30-110
  - crates/wanix-cli/src/mesh/serve_cpu.rs
seeAlso:
  - concepts/send-agent-to-the-data
  - concepts/capability-is-a-bind
  - concepts/subtreefs-confine-to-prefix
  - devices/task
  - concepts/import-export-and-n
  - recipes/02-mount-remote-peer
prerequisites:
  - concepts/import-export-and-n
  - concepts/capability-is-a-bind
usedInFlows: []
honestLimits:
  - "v1 delivers stdout/stderr/exit as a single batch after the remote task's start returns, not incrementally streamed."
  - "CpuEvent::Cancel stops the caller draining the control stream; it does not stop the remote computation (the driver has no abort hook)."
  - "The exec plane is LOCAL-TRUST / grant-allowlisted only; remote code execution is not exposed to untrusted public peers, and there are no hard CPU/memory limits yet (only a concurrent-job cap and per-op deadlines)."
  - "cpu --node takes an iroh:// ticket only — catalog names do not resolve here (executed: `cpu --node nodeb` is refused with \"mesh address must start with iroh://: nodeb\"); names cover the mount surfaces (recipe 10)."
canonicalCaveatFor: []
---

# #cpu — Exec Plane (cpu(1) over the Mesh)

A caller reverse-exports a scoped read-only namespace and a remote acceptor runs a task whose world is that export — Plan 9 cpu(1) over QUIC, grant-allowlisted.

Plan 9's `cpu(1)` let you log in to a remote machine and keep your own files: the CPU server ran your shell, but every file it touched was *your* namespace, imported back over the wire. `#cpu` is that idea generalized to the open internet. You stay on node X, you point at a build script in your working directory, and the heavy compute runs on node Y — but node Y never sees node Y's disk for this job. It sees *your* directory, re-exported to it as a read-only filesystem. Compute travels to the data; equivalently, the data is imported into the runner's world. The crate doc states the goal exactly: "run a task next to the data, namespace from here" (`crates/wanix-cpu/src/lib.rs:1`).

## Show it: run a job on a peer

Both halves ship: `mesh-serve --cpu` binds the `CpuAcceptor` beside the namespace plane, and `wanix cpu` dials it. Executed transcript (same laptop, two terminals):

```sh
cargo build --locked --package wanix-cli
alias wanix='./target/debug/wanix'

# Terminal 1 — node Y, the cpu server. --cpu is refused on any non-loopback
# endpoint; --addr 127.0.0.1:PORT pins a loopback-only socket (add --peer HEX
# to admit only that identity).
wanix mesh-serve --root "$ROOT_Y" --key /tmp/Y.key \
    --addr 127.0.0.1:5680 --cpu
```

```
wanix mesh-serve: node 11fd26a61cf8294ea2ba8d16ce4ebb46cac06152313aa880b5e9072dfe9638a5
wanix mesh-serve: ticket iroh://11fd26a6…dfe9638a5?addr=127.0.0.1:5680
wanix mesh-serve: mount with: wanix mount-ls 'iroh://11fd26a6…dfe9638a5?addr=127.0.0.1:5680'
wanix mesh-serve: serving the #cpu exec plane (remote code execution for admitted peers); run a job with: wanix cpu --node 'iroh://11fd26a6…dfe9638a5?addr=127.0.0.1:5680' -- qjs PROGRAM
```

```sh
# Terminal 2 — node X, the caller. The job's world is X's cwd, reverse-
# exported; build.js lives HERE, not on Y.
cd "$ROOT_X"   # contains build.js and dataset.txt
wanix cpu --node 'iroh://11fd26a6…dfe9638a5?addr=127.0.0.1:5680' -- qjs build.js
```

```
built on the data node; input: caller-side input carried by the reverse export
```

Exit code 0. `build.js` runs on node Y. Its stdout and stderr come back to node X's terminal, and the process exit code is the remote task's exit code. The job read `build.js` — and anything else it opened — out of node X's working directory, proxied file-by-file across QUIC. The command grammar is `cpu --node TICKET [--cwd DIR] [--write] [--env KEY=VALUE ...] -- KIND PROGRAM [ARG ...]` (`crates/wanix-cli/src/cpu/parse.rs:30`). Options precede `--`; everything after `--` is the job command, so a program's own flags are never mistaken for cpu options. `--cwd` chooses the local directory to export (default `.`), `KIND` is the task driver (`qjs`, `wasm`, `noop`), and `PROGRAM` is the argv[0] *inside the exported world*.

## Two streams, role-sorted first

A job dials node Y under ALPN `wanix/cpu/1` (`crates/wanix-mesh/src/cpu.rs:39`) and opens two bidirectional QUIC streams:

- **control** carries `CpuEvent` frames — the job's stdout, stderr, and terminal exit, written by the acceptor after the task runs.
- **export** carries the caller's *own* `wanix_9p::P9Server`. Node Y runs a `wanix_9p_client::RemoteFs` over the same stream and binds it as the task's filesystem root.

Over QUIC the two streams do not arrive at the acceptor in open order: a stream is invisible to the peer's `accept_bi` until its opener writes a first byte, so *first-write order* — not open order — decides which the acceptor sees first (`crates/wanix-cpu/src/role.rs:1`). The fix is a one-byte discriminator written immediately after `open_bi`: `ROLE_CONTROL = 0`, `ROLE_EXPORT = 1` (`crates/wanix-cpu/src/role.rs:17`). The acceptor reads the first byte of each accepted stream and routes it (`crates/wanix-cpu/src/role.rs:82`). v1 never assumes "the first accepted stream is control."

## The acceptor reuses the exact local launch

The whole point of building the exec plane on the 9P contract is that the remote run is byte-for-byte the local run. `run_job` is `allocate_root` → `task.bind(world, ".", ".")` → configure → `start` (`crates/wanix-cpu/src/acceptor.rs:74`), the same sequence a local `#task` launch uses. The only difference is the *world*: instead of a local `MemFs`, the task binds a `RemoteFs` that proxies into the caller's reverse-exported namespace (`crates/wanix-cpu/src/acceptor.rs:106`). Every file the guest opens resolves through the reverse 9P session back to node X. Because `wanix-cpu` never names a concrete runtime, the caller of `run_job` registers the drivers on the `TaskTable` it passes in — keeping the dependency direction honest (`crates/wanix-cpu/src/lib.rs:26`). Each job gets a fresh table, which is the isolation boundary: one job's task state never leaks into another peer's (`crates/wanix-mesh/src/cpu/handler.rs:34`).

A remote job is inspectable through [`#task`](/devices/task) exactly like a local one: the acceptor mirrors the observable `#task` fields — `cmd`, `env`, `dir` — from the spec (`crates/wanix-cpu/src/acceptor.rs:117`).

## ExportScope: a room, not the house

A reverse export must **not** serve node X's whole host root. The export half is a scoped sub-namespace described by `ExportScope` (`crates/wanix-cpu/src/scope.rs:62`): the job's working subtree, plus only the services the caller explicitly grants, **read-only by default**. `ExportScope::new(backing, prefix)` re-roots the job subtree as a `wanix_vfs::SubtreeFs` with read-only rights (`crates/wanix-cpu/src/scope.rs:75`); `.writable()` is the deliberate opt-in for jobs that must write outputs back (`crates/wanix-cpu/src/scope.rs:89`), surfaced as the CLI's `--write` flag. `.grant(GrantedService::new("#kv", backing, prefix, rights))` adds one service, re-rooted and rights-gated the same way and bound under its `#`-device name (`crates/wanix-cpu/src/scope.rs:96`). Nothing the caller does not name is reachable: a file outside the prefix is "simply not present in the scoped namespace" (`crates/wanix-cpu/src/scope.rs:188`).

This is [a capability as a bind](/concepts/capability-is-a-bind), not an ACL. The grant *is* the re-rooted `SubtreeFs` — there is no list to check at open time, because the path that would name an ungranted file does not exist in the world the job was handed. See [SubtreeFs: confine to a prefix](/concepts/subtreefs-confine-to-prefix).

## Default-deny on the acceptor side

The export scope is the caller's half of the confinement; the acceptor enforces the other half. `CpuAcceptor` is grant-allowlisted (`crates/wanix-mesh/src/cpu/handler.rs:42`). Identity is the verified QUIC handshake key — 0-RTT is never used — and a peer the allowlist rejects "gets nothing: close without accepting a stream" (`crates/wanix-mesh/src/cpu/handler.rs:91`). The cpu ALPN is exec, the sharpest capability on the mesh, so default-deny is enforced before any work happens. Admitted jobs run on the blocking pool, capped at `MAX_CONCURRENT_CPU_JOBS = 32` (`crates/wanix-mesh/src/node.rs:50`) with a held semaphore permit and a per-op deadline on every stream read/write, so a stalled or hostile-but-allowlisted peer cannot park acceptor threads forever (`crates/wanix-mesh/src/cpu/handler.rs:65`).

## See also

- [Send the agent to the data](/concepts/send-agent-to-the-data) — the inversion `#cpu` realizes.
- [A capability is a bind](/concepts/capability-is-a-bind) and [SubtreeFs: confine to a prefix](/concepts/subtreefs-confine-to-prefix) — how the export scope confines without an ACL.
- [Import, export, and /n](/concepts/import-export-and-n) — the reverse export reuses the same 9P import half.
- [#task](/devices/task) — the local launch the acceptor mirrors.
- [Recipe 02: mount a remote peer](/recipes/02-mount-remote-peer) — the dial/ticket workflow `#cpu` rides on.

## Status / honest limits

- **Both halves ship.** `wanix cpu` dials and `mesh-serve --cpu` serves (`crates/wanix-cli/src/mesh/serve_cpu.rs`); the end-to-end proof over real loopback QUIC is `mesh_serve_cpu_runs_a_dialed_job_against_the_callers_reverse_export` in `crates/wanix-cli/src/mesh/serve.rs`. Serving cpu follows the exec-device rule: refused on any non-loopback endpoint entirely (even with `--peer`/`--grant` or `--insecure-open` — a LAN `--addr` is mDNS-discoverable, so only loopback is local trust); on the loopback `--addr` endpoint, `--peer HEX` scopes exec to that one verified identity and no `--peer` admits any dialer that reaches the loopback socket.
- **Output is batched, not streamed.** The task model runs the guest to completion inside `start` and only then has its buffered stdout, so v1 delivers stdout, stderr, and exit as a single batch after `start` returns (`crates/wanix-cpu/src/lib.rs:33`, `crates/wanix-cpu/src/acceptor.rs:63`). Incremental streaming is a named follow-up, not an implied capability.
- **Cancel stops draining, not computing.** The driver has no abort hook, so `CpuEvent::Cancel` stops the caller draining the control stream; it does not stop the remote computation (`crates/wanix-cpu/src/lib.rs:36`).
- **Local-trust / grant-allowlisted only.** This crate provides the mechanism, not a public policy. Exporting `#cpu`/`#task` for remote code execution stays local-trust and allowlisted until public auth lands (`crates/wanix-cpu/src/lib.rs:43`). The acceptor caps concurrent jobs and bounds per-op I/O, but there are no hard CPU or memory limits on a running guest yet — read this as cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code.
