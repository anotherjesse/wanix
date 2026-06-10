---
title: Distributed Dev Environments
slug: use-cases/distributed-dev-environments
pageType: use-case
oneLiner: "#cpu runs your task on the node holding the source against its fast local namespace, with the output returning over the wire — Plan 9 cpu(1) over the open internet, behind a default read-only jail."
audience: [developer, visionary]
tags: [mesh, cli, local-trust-only, caveat, shipped]
sourceRefs:
  - docs/recipes/02-mount-remote-peer.md:159-231
  - crates/wanix-cpu/src/scope.rs:69-126
  - crates/wanix-cpu/src/acceptor.rs:10-11
  - crates/wanix-cpu/src/event.rs:20-48
  - crates/wanix-cli/src/mount.rs:26
  - docs/mesh-the-missing-half-of-9p.md:1884-1917
seeAlso:
  - devices/cpu
  - concepts/send-agent-to-the-data
  - concepts/capability-is-a-bind
  - concepts/subtreefs-confine-to-prefix
  - concepts/import-export-and-n
  - use-cases/personal-compute-mesh
  - recipes/02-mount-remote-peer
prerequisites:
  - devices/cpu
  - concepts/send-agent-to-the-data
usedInFlows: []
honestLimits:
  - "v1 #cpu delivers the job's stdout/stderr/exit as a single batch after start returns, not as an incremental stream."
  - "A #cpu Cancel stops the caller draining the control stream; it does not abort the remote computation."
  - "Exec devices (#cpu/#task/#agent) are local-trust only; there are no hard CPU or memory limits on the remote run yet."
  - "The reverse export is jailed to the named subtree and read-only unless --write opts a subtree into read-write."
---

# Distributed Dev Environments

`#cpu` runs your task on the node holding the source against its fast local namespace, with the output returning over the wire — Plan 9 cpu(1) over the open internet, behind a default read-only jail.

**What & why.** The data lives where it lives — a fat checkout on a cloud box, a dataset on the machine that produced it — and your laptop is just the place you type. Pulling that tree across a NAT so you can build it is a tax: a 9P walk+open+read is three to five serial round-trips, and dragging a large tree through the `msize` window over the wire dominates everything (`docs/mesh-the-missing-half-of-9p.md:1908-1912`). The fix is the dual of mounting: instead of pulling the data to your compute, send your compute to the data. You edit here, the build runs *there* against the data node's local files at local-namespace speed, and only the small result comes back. The mechanism is `#cpu` — Plan 9's `cpu(1)`, only the "terminal namespace" reverse-mounted back to the remote machine is now a scoped, content-jailed subtree and the transport is QUIC.

## The outcome: edit here, build there

Two nodes. Node B holds the source under `work/` (a `build.js` and a `dataset.txt`); node A is where you sit, and its working directory can be empty. From A you run one command and the build happens on B, against B's files, with B's CPU:

```sh
cargo build --locked --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

cd "$ROOT_A"                                   # holds build.js — the job's world is A's cwd, reverse-exported
wanix-rust cpu --node "$NODE_B" -- qjs build.js
# -> ran on node B against A's reverse-exported namespace
```

(B serves the exec plane with `mesh-serve --cpu`, which is refused on the public endpoint and `--peer`-scopable on the local one — serving cpu is remote code execution on B.)

That line of stdout was produced by a `qjs` task that ran on B; every file the job opened — including `build.js` itself — resolved through the reverse session back to A's working directory, and the bytes came back to A on the cpu control stream with exit 0 (`docs/recipes/02-mount-remote-peer.md`). This is Plan 9 cpu's split: **the caller's namespace, the server's processor** — the computation traveled to the machine, the namespace stayed yours.

## Reverse-export your cwd — read-only by default

Before B can run anything useful, it needs a view of A's world. So `wanix-rust cpu` does the Plan 9 move in reverse: it *reverse-exports* A's working directory back across the connection, and B runs the job with that reverse export as its world (`docs/mesh-the-missing-half-of-9p.md:1898-1903`). The export is not A's whole host root. It is a scoped sub-namespace built by `ExportScope` in `wanix-cpu`: the job subtree re-rooted with `SubtreeFs`, plus any services you explicitly grant, and **read-only by default** (`crates/wanix-cpu/src/scope.rs:69-82`).

Opt a subtree into read-write only when the remote run must write outputs back to you:

```sh
wanix-rust cpu --node "$NODE_B" --cwd "$ROOT_A" --write -- qjs /work/build.js
```

`--write` flips the scope's rights to read-write (`crates/wanix-cpu/src/scope.rs:84-92`); without it, a write-mode open is refused by the rights gate, and a path outside the declared subtree is simply not present in the exported namespace. Those two invariants are pinned by the `job_subtree_is_read_only_by_default` and `the_scope_cannot_reach_outside_the_job_subtree` tests in `wanix-cpu`. The named idea is the jail: the reverse export is a *room, not a house* — see [Confine to a prefix with SubtreeFs](/concepts/subtreefs-confine-to-prefix). A granted service (e.g. `#kv`) is re-rooted and rights-gated the same way and bound under its `#`-device name (`crates/wanix-cpu/src/scope.rs:94-123`), so the remote job can read exactly the state you handed it and nothing else.

## The acceptor runs the exact local launch pattern

The clean part of this design is that the remote side is not a special remote-execution server. The acceptor accepts two QUIC streams — one control, one export — sorts them by a leading role byte (never "the first one is control"), builds a `RemoteFs` over the export stream, and binds the job's task against that `RemoteFs` exactly as a local launch binds a task against a local `FileSystem` (`docs/mesh-the-missing-half-of-9p.md:1898-1907`, `:1923-1926`). The job's world is *just a `Namespace`*; its root is *just a bound `FileSystem`*; the export is *just a `P9Server`*. The mesh added a direction — the server now runs on the caller — not a new mechanism. That is why `#cpu` composes with everything else: it is the same task model, the same VFS, the same 9P contract, run inside out.

## Compute travels to the data, against an allowlisted acceptor

Two authenticated transports carry this, and both verify B as B before anything happens (`docs/recipes/02-mount-remote-peer.md:219-231`):

- **9P import** (the `mount-*` verbs) dials ALPN `wanix/9p/1`; iroh QUIC verifies B's ed25519 key against the ticket, and the attach grant table is keyed on the *verified* `PeerId`, not a client-claimed `uname`. See [Capability is a bind](/concepts/capability-is-a-bind).
- **cpu exec** dials ALPN `wanix/cpu/1`; the acceptor admits only B's verified peer against an allowlist. The wire-level deny case is the `an_unallowlisted_peer_cannot_run_a_cpu_job` test.

Neither path ever trusts an IP, a hostname, or a username. You mount the key and you run on the key. The grammar is strict, too: options precede `--`, the job command follows it, and a missing `--node` is refused at parse time with exit 2 (`docs/recipes/02-mount-remote-peer.md:188-205`).

## Runnable recipe

Walk the whole thing — two nodes, identity, mount, and a `#cpu` job — in [Recipe 02 — Mount a remote peer](/recipes/02-mount-remote-peer); the `#cpu` job is its section 4. For the broader "your machines stop being islands" framing, see [Your personal compute mesh](/use-cases/personal-compute-mesh).

## See also

- **Devices:** [#cpu](/devices/cpu)
- **Concepts:** [Send the agent to the data](/concepts/send-agent-to-the-data) · [Capability is a bind](/concepts/capability-is-a-bind) · [Confine to a prefix with SubtreeFs](/concepts/subtreefs-confine-to-prefix) · [Import, export, and /n](/concepts/import-export-and-n)
- **Use cases:** [Your personal compute mesh](/use-cases/personal-compute-mesh) · [Send the agent to your files](/use-cases/agents-on-your-files)
- **Recipes:** [Recipe 02 — Mount a remote peer](/recipes/02-mount-remote-peer)

## Status / honest limits

This is real and runnable today, but precise about its edges:

- **Output is one batch, not a stream.** A task is captured-stdout only — the driver returns its buffered stdout after `start` returns, so v1 delivers the job's stdout/stderr/exit as a single batch of `CpuEvent` frames *after* the job finishes, not incrementally during the run (`crates/wanix-cpu/src/acceptor.rs:10-11`, `crates/wanix-cpu/src/event.rs:20-23`). Incremental streaming (a streaming-stdout `File` that pushes frames during eval) is unbuilt.
- **Cancel doesn't stop the remote run.** A `CpuEvent::Cancel` written by the caller stops *draining* the control stream; it does not abort the remote computation, which keeps running on B (`crates/wanix-cpu/src/event.rs:47-48`, `crates/wanix-cpu/src/error.rs:18-20`). There is no task abort hook yet.
- **Exec is local-trust only.** `#cpu`, `#task`, and `#agent` are remote code execution and are gated to trusted peers — cheap, scalable isolation, *not* a sandbox for arbitrary untrusted code. There are no hard CPU or memory limits on the remote run yet; the trust model is your own keys and an explicit acceptor allowlist, not accounts. Public multi-user auth and grant lifecycle remain unimplemented trust-boundary work.
- **The reverse export is jailed.** It is scoped to the named subtree and read-only unless `--write` opts a subtree into read-write (`crates/wanix-cpu/src/scope.rs:69-92`); paths outside the subtree are not present in the exported namespace.
- **`/n/<peer>` is a convention, not a path.** The shipped `mount-*` verbs bind the remote at the single slot `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` namespaces are designed but unshipped. The peer id lives in the `iroh://` ticket you dialed, not in the namespace prefix.
