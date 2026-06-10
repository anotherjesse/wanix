---
title: Recipe 02 — Mount a Remote Wanix Peer and Run a Job on It
slug: recipes/02-mount-remote-peer
pageType: use-case
oneLiner: Stand up two nodes, have A dial B over iroh QUIC, mount B's directory as a Plan 9 namespace, and run a #cpu job on B against A's reverse-exported files.
audience: [developer]
tags: [mesh, cli, local-trust-only, caveat, shipped]
sourceRefs:
  - docs/recipes/02-mount-remote-peer.md
  - crates/wanix-cli/src/mount.rs:26
  - crates/wanix-cli/src/cpu/run.rs:30-91
  - crates/wanix-cli/src/cpu/parse.rs:40-110
  - crates/wanix-cli/src/mesh/serve.rs:54-149
seeAlso:
  - concepts/import-export-and-n
  - concepts/key-is-the-address
  - devices/cpu
  - concepts/capability-is-a-bind
  - concepts/send-agent-to-the-data
  - use-cases/personal-compute-mesh
prerequisites:
  - concepts/import-export-and-n
  - concepts/key-is-the-address
  - devices/cpu
usedInFlows:
  - {flow: wire-a-mesh, step: 3}
honestLimits:
  - "mount-* always binds the remote at the single slot /n/remote; per-peer /n/<peer-id> is designed but unshipped."
  - "#cpu serving (mesh-serve --cpu) is refused on the public endpoint entirely; on a local --addr endpoint, --peer scopes exec to one verified identity."
  - "--wanix-services exports the #task/#agent exec devices and is refused on the public endpoint; it is local-trust only."
  - "#cpu (and #task/#agent) are local-trust only: cheap isolation, not a sandbox for arbitrary untrusted code, with no hard CPU/memory limits yet."
  - "A node's #kv is in-memory; state survives only while mesh-serve runs."
canonicalCaveatFor: []
---

# Recipe 02 — Mount a Remote Wanix Peer and Run a Job on It

Stand up two nodes, have A dial B over iroh QUIC, mount B's directory as a Plan 9 namespace, and run a `#cpu` job on B against A's reverse-exported files.

**What & why.** Two machines, two terminals. One holds the data; the other holds nothing and reaches across. You will watch node A read a file that only exists on node B — no SSH, no rsync, no shared filesystem mount — just a filesystem walk over a QUIC connection that authenticated B by its ed25519 key before a single byte of namespace crossed. Section 4 runs a job on B's CPU against A's reverse-exported working directory — both halves of `#cpu` ship (`mesh-serve --cpu` serves, `wanix-rust cpu` dials). This is the `/n/remote` import recipe end to end, and the spine of [Wire a mesh](/learn/wire-a-mesh). Everything is grounded in the shipped CLI verbs; the caveats are stated flatly at the bottom.

## 0. Build the binary

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix-rust='./target/debug/wanix-rust'
```

## 1. Each node gets a persistent ed25519 identity

A Wanix node's identity is a 32-byte ed25519 seed loaded from (or generated into) a key file; the public key *is* its `PeerId` and its iroh `EndpointId`. The default is `~/.wanix/node.key`, created `0600` and stable across restarts. The address is the key — there is no DNS, no username, no IP-as-identity ([The key is the address](/concepts/key-is-the-address)).

Pick two clean key paths so two nodes on one laptop don't collide, and give B some files:

```sh
export NODE_B_KEY=/tmp/wanix-B.key
export ROOT_A=$(mktemp -d)
export ROOT_B=$(mktemp -d)

mkdir -p "$ROOT_B/work"
printf 'data that only lives on node B\n' > "$ROOT_B/work/dataset.txt"

cat > "$ROOT_A/build.js" <<'JS'
import * as std from "qjs:std";
std.out.puts("ran on node B against A's reverse-exported namespace\n");
JS
```

Node B holds `dataset.txt`. Node A holds `build.js` — the program A will run *on B's processor*, against A's own reverse-exported namespace (Plan 9 cpu's split: caller's namespace, server's CPU).

## 2. Start node B with `wanix-rust mesh-serve` (the data node)

Node B binds an iroh endpoint from its persistent key, exports `$ROOT_B` over QUIC (the native wire, with 9P alongside for foreign clients), and prints its node id plus a dialable ticket on stderr (`announce_message`, `crates/wanix-cli/src/mesh/serve.rs:319-335`).

For a same-laptop demo, serve a **local direct-address-only endpoint** with `--addr 127.0.0.1:5680`. The parser refuses to serve the *public* endpoint with no grant gate — that would hand the whole root read-write to anyone holding the ticket, and `parse_rejects_ungranted_public_serve` (`serve.rs:322`) pins that. A `--addr` endpoint is fine: its ticket is exchanged out of band, not discovered via relays.

We also pass `--wanix-services`, which exports the full services namespace (`#term`, `#pipe`, `#kv`, `#agent`, `#task`, plus the host root), and `--cpu`, which serves the `#cpu` exec plane (ALPN `wanix/cpu/1`) beside the namespace plane. `--cpu` is remote code execution on B and follows the exec rule: refused on the public endpoint entirely; add `--peer HEX` on the local endpoint to admit only that one verified identity.

Terminal 1 (node B):

```sh
wanix-rust mesh-serve \
    --root "$ROOT_B" \
    --key  "$NODE_B_KEY" \
    --addr 127.0.0.1:5680 \
    --wanix-services \
    --cpu
```

Executed stderr — the node id, the dialable ticket, a copy-pasteable mount command, and the loud exec-plane notice (the long hex is B's verifying key; yours will differ):

```
wanix-rust mesh-serve: node 11fd26…638a5
wanix-rust mesh-serve: ticket iroh://11fd26…638a5?addr=127.0.0.1:5680
wanix-rust mesh-serve: mount with: wanix-rust mount-ls 'iroh://11fd26…638a5?addr=127.0.0.1:5680'
wanix-rust mesh-serve: serving the #cpu exec plane (remote code execution for admitted peers); run a job with: wanix-rust cpu --node 'iroh://11fd26…638a5?addr=127.0.0.1:5680' -- qjs PROGRAM
```

The `ticket` line is B's **verified address**. Copy yours (the full ticket your terminal printed, not this shortened one):

```sh
export NODE_B='iroh://11fd26…638a5?addr=127.0.0.1:5680'
```

The 64 hex digits after `iroh://` are B's ed25519 public key. The `?addr=` query is a first-contact direct-address hint; on the public internet you would drop it and let iroh resolve from the key alone.

## 3. From node A, mount B's root

Node A does not even need to run `mesh-serve` to *import* — the `mount-*` verbs bind an ephemeral dialer node and attach over QUIC (`crates/wanix-cli/src/mount.rs`). An `iroh://` mount rides the **native FileSystem-over-iroh wire** (`NativeFs`, one stream per op); a `tcp://` mount is the foreign-edge raw-9P `RemoteFs`. Both are plain `FileSystem`s, so every `mount-*` verb works identically over either.

Terminal 2 (node A):

```sh
wanix-rust mount-ls "$NODE_B"          # -> work
wanix-rust mount-ls "$NODE_B" work     # -> dataset.txt
wanix-rust mount-cat "$NODE_B" work/dataset.txt
# -> data that only lives on node B
```

Those bytes never existed on A's disk. They crossed a dedicated QUIC stream as a `postcard`-framed native-wire read reply and printed on A. This is Plan 9's import — a server *exports* a namespace, a client *imports* it and binds it into its own tree ([Import / export and /n](/concepts/import-export-and-n)). Because devices are filesystems too, `--wanix-services` means `#kv`, `#agent`, and friends ride along the same mount ([devices import for free](/concepts/devices-import-for-free)).

You can write back, too — `mount-write` truncates-or-creates a file at the path, and the bytes land on B's host disk:

```sh
wanix-rust mount-write "$NODE_B" work/note.txt 'written from A'
```

## 4. Run the `#cpu` job on B

Mounting moves bytes toward you. `#cpu` does the opposite: it moves the *computation* ([Send the agent to the data](/concepts/send-agent-to-the-data)). `wanix-rust cpu` dials B's exec plane (ALPN `wanix/cpu/1`, served because step 2 passed `--cpu`), **reverse-exports** A's working directory as a scoped namespace, and asks B to run a task whose world *is* that reverse export (`crates/wanix-cli/src/cpu/run.rs:30-91`). Executed transcript, from A's `$ROOT_A`:

```sh
cd "$ROOT_A"
wanix-rust cpu --node "$NODE_B" -- qjs build.js
# -> ran on node B against A's reverse-exported namespace
```

Exit code 0. The script ran on B; every file it opened — including `build.js` itself — resolved through the reverse session back to A. The contract: strict grammar (`crates/wanix-cli/src/cpu/parse.rs:40-110` — options before `--`, `KIND PROGRAM [ARG ...]` after, missing `--node` or `--` refused at parse time), a **read-only-by-default** reverse export with `--write` opting the subtree into read-write, out-of-scope paths denied by `ExportScope` in `wanix-cpu`, and output delivered as one batch after the task finishes. The end-to-end proof is `mesh_serve_cpu_runs_a_dialed_job_against_the_callers_reverse_export` (`crates/wanix-cli/src/mesh/serve.rs`).

## 5. Trust boundary, in one line

Both planes authenticate B as B before anything else happens. The import plane (`mount-*` over `iroh://`, ALPN `wanix/fs/1` — the native wire) has iroh QUIC verify B's ed25519 key against the ticket, and the attach grant table keys on the *verified* `PeerId`, never a client-claimed name ([Capability is a bind](/concepts/capability-is-a-bind)). The exec plane (`cpu`, ALPN `wanix/cpu/1`) admits the verified peer against an allowlist. Neither path ever trusts an IP, hostname, or username. Mount the key, run on the key.

## See also

- Concepts: [Import / export and /n](/concepts/import-export-and-n) · [The key is the address](/concepts/key-is-the-address) · [Capability is a bind](/concepts/capability-is-a-bind) · [Devices import for free](/concepts/devices-import-for-free) · [Send the agent to the data](/concepts/send-agent-to-the-data)
- Devices: [#cpu](/devices/cpu) · [#kv](/devices/kv) · [#agent](/devices/agent)
- Flows: [Wire a mesh](/learn/wire-a-mesh)
- Use case: [Your personal compute mesh](/use-cases/personal-compute-mesh)

## Status / honest limits

This is real and runnable today, but be precise about the edges.

- **One mount slot, not per-peer paths (yet).** The aspirational shape is `/n/<peer-id>/…`. The shipped `mount-*` verbs always bind the remote at the single slot **`/n/remote`** — `MOUNT_POINT` in `crates/wanix-cli/src/mount.rs:26`. The peer id lives in the `iroh://` ticket you dialed, not in the namespace prefix. Use `/n/<peer>` only as a labelled convention; per-peer mounts wait on the namespace seam, a queued follow-up.
- **`#cpu` ships both halves, batched.** `mesh-serve --cpu` binds the `CpuAcceptor` beside the namespace plane (`crates/wanix-cli/src/mesh/serve_cpu.rs`); output is one batch after the task finishes, and there is no remote cancel.
- **Exec is local-trust only.** `--wanix-services` binds the `#task`/`#agent` exec devices (remote code execution) and the parser refuses it on the public endpoint regardless of `--peer`/`--grant`/`--insecure-open` (`serve.rs:130-139`). `#cpu`, `#task`, and `#agent` are cheap, scalable isolation — *not* a sandbox for arbitrary untrusted code, and there are no hard CPU or memory limits yet. Run them only against peers you trust.
- **`#kv` is in-memory.** A node's `#kv` lives only as long as its `mesh-serve` process; freeze the world to a [capsule](/concepts/wanix-capsule) to persist it.
- **Live pub/sub needs a second connection.** The serve 9P transport handles one frame at a time per connection, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection ([single-frame serve caveat](/concepts/single-frame-serve-caveat)).
