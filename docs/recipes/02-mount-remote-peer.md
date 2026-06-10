# Recipe 02 — Mount a remote Wanix peer and run a job on it

Goal: stand up two Wanix nodes (A and B), have A learn B's verified ed25519
identity, dial B over the iroh QUIC mesh, mount B's directory as a Plan 9
namespace, and then send a `#cpu` job from A so A's code *runs on B's
processor against A's reverse-exported namespace* — Plan 9 cpu's split:
caller's namespace, server's CPU.

This is import (Plan 9's `srv`/`mount`) and cpu (Plan 9's `cpu(1)`) realized
over a verified QUIC transport. The wire contracts are ALPN
`wanix/9p/1` for the import plane and ALPN `wanix/cpu/1` for the exec plane,
declared in [`crates/wanix-mesh/src/lib.rs`][mesh-lib] and
[`crates/wanix-mesh/src/cpu.rs`][mesh-cpu]. The peer's address is its
ed25519 public key — there is no DNS, no usernames, no IP-as-identity. The
key *is* the address.

[mesh-lib]: ../../crates/wanix-mesh/src/lib.rs
[mesh-cpu]: ../../crates/wanix-mesh/src/cpu.rs
[mesh-serve]: ../../crates/wanix-cli/src/mesh/serve.rs
[mesh-ticket]: ../../crates/wanix-cli/src/mesh/ticket.rs
[mount]: ../../crates/wanix-cli/src/mount.rs
[cpu-cli]: ../../crates/wanix-cli/src/cpu/run.rs
[id-identity]: ../../crates/wanix-id/src/identity.rs
[client-remote]: ../../crates/wanix-9p-client/src/remote.rs

## 1. Each node gets a persistent ed25519 identity

A Wanix node's identity is a 32-byte ed25519 seed, loaded from (or generated
into) a key file, with the public key as its `PeerId` and the iroh
`EndpointId`. See [`NodeIdentity::load_or_create`][id-identity] —
`~/.wanix/node.key` is the default, created with `0600` permissions, stable
across restarts.

Pick two clean key paths so the two nodes on this laptop don't collide:

```sh
export NODE_A_KEY=/tmp/wanix-A.key
export NODE_B_KEY=/tmp/wanix-B.key

# Make two host directories the nodes will export.
export ROOT_A=$(mktemp -d)
export ROOT_B=$(mktemp -d)

mkdir -p "$ROOT_B/work"
printf 'data that only lives on node B\n' > "$ROOT_B/work/dataset.txt"

cat > "$ROOT_A/build.js" <<'JS'
import * as std from "qjs:std";
std.out.puts("ran on node B against A's reverse-exported namespace\n");
JS
```

Node B holds `dataset.txt`. Node A holds `build.js` — the program A will run
*on B's processor*. This is Plan 9 cpu's split exactly: the job's namespace is
the **caller's** (reverse-exported to the server), and the server contributes
its CPU.

## 2. Start node B with `wanix mesh-serve` (the data node)

Node B binds an iroh endpoint from its persistent key, exports `$ROOT_B` as 9P
over QUIC, and prints its node id plus a dialable `iroh://` ticket on stderr.
The CLI surface is parsed in [`crates/wanix-cli/src/mesh/serve.rs`][mesh-serve].

For a same-laptop demo we serve a **loopback-only endpoint** with
`--addr 127.0.0.1:5680`. The blueprint refuses to serve any non-loopback
endpoint without a grant gate (default-deny: the public endpoint is reachable
by any NodeID with the ticket, and a LAN `--addr` is mDNS-discoverable) — see
the parse test `parse_rejects_ungranted_public_serve` in
[`mesh/serve.rs`][mesh-serve]. A loopback endpoint is fine: no other host can
reach the socket at all.

We also pass `--wanix-services`, which exports the full services namespace
(`#term`, `#pipe`, `#kv`, `#agent`, `#task`, plus the host root) so node A
can later launch tasks through `#task` and `#cpu`. **`--wanix-services` is
local-trust only**: the parser refuses it on any non-loopback endpoint because
`#task`/`#agent` are remote code execution. The error message in
[`mesh/serve.rs`][mesh-serve] says so explicitly.

We also pass `--cpu`, which serves the `#cpu` exec plane (ALPN `wanix/cpu/1`)
beside the namespace plane so step 4 can run a job on B. **`--cpu` is the
sharpest capability on the mesh — remote code execution on B** — so it
follows the same exec rule: refused on any non-loopback endpoint entirely; on
the loopback `--addr` endpoint, add `--peer HEX` to admit only that one
verified identity (without it, anything on this host that dials the loopback
socket is admitted, like the rest of a local open serve).

Terminal 1 (node B):

```sh
wanix mesh-serve \
    --root  "$ROOT_B" \
    --key   "$NODE_B_KEY" \
    --addr  127.0.0.1:5680 \
    --wanix-services \
    --cpu
```

Executed stderr (the long hex is B's verifying key, your value will differ):

```
wanix mesh-serve: node 11fd26a61cf8294ea2ba8d16ce4ebb46cac06152313aa880b5e9072dfe9638a5
wanix mesh-serve: ticket iroh://11fd26a61cf8294ea2ba8d16ce4ebb46cac06152313aa880b5e9072dfe9638a5?addr=127.0.0.1:5680
wanix mesh-serve: mount with: wanix mount-ls 'iroh://11fd26a61cf8294ea2ba8d16ce4ebb46cac06152313aa880b5e9072dfe9638a5?addr=127.0.0.1:5680'
wanix mesh-serve: serving the #cpu exec plane (remote code execution for admitted peers); run a job with: wanix cpu --node 'iroh://11fd26a61cf8294ea2ba8d16ce4ebb46cac06152313aa880b5e9072dfe9638a5?addr=127.0.0.1:5680' -- qjs PROGRAM
```

The `ticket` line is B's **verified address**. Copy it; node A needs it.

```sh
export NODE_B='iroh://11fd26a61cf8294ea2ba8d16ce4ebb46cac06152313aa880b5e9072dfe9638a5?addr=127.0.0.1:5680'
```

The 64 hex digits after `iroh://` are B's ed25519 public key — its `PeerId`
from [`wanix-id`][id-identity]. The `?addr=` query carries first-contact
direct addresses; on the public internet you'd drop it and let iroh's relay
discovery resolve from the key alone.

## 3. From node A, mount B's root

Node A doesn't need to run `mesh-serve` at all to *import* — the `mount-*`
verbs in [`crates/wanix-cli/src/mount.rs`][mount] bind an ephemeral dialer
node and attach over QUIC. Under the hood
[`crates/wanix-cli/src/mesh/ticket.rs`][mesh-ticket] parses the ticket and
[`MeshDialer`][mesh-lib] opens the connection; the same
[`RemoteFs`][client-remote] used over loopback TCP backs the mount, just over
a QUIC stream now.

Terminal 2 (node A):

```sh
wanix mount-ls "$NODE_B"
```

Expected stdout:

```
work
```

The mount root is the directory B exported. List inside it:

```sh
wanix mount-ls "$NODE_B" work
```

```
dataset.txt
```

Read the file across the wire:

```sh
wanix mount-cat "$NODE_B" work/dataset.txt
```

```
data that only lives on node B
```

Those bytes never existed on A's disk. They crossed the QUIC stream from B's
`Tread` reply, deserialized through the 9P client in
[`wanix-9p-client`][client-remote], and printed on A.

**One honest caveat.** The recipe's promised path is `/n/<peer-id>/`. The
shipped `mount-*` verbs always bind the remote at `/n/remote` in a single
mount namespace — see `MOUNT_POINT` in [`mount.rs`][mount]. The peer-id
appears in the `iroh://` ticket, not in the namespace prefix. Once the
namespace seam grows per-peer mounts (a queued follow-up in `CLAUDE.md`),
the namespace shape becomes `/n/<peer-id>/...`; today, think of `/n/remote`
as the mount slot for "whichever ticket I dialed".

## 4. Run a `#cpu` job on B from A's CLI

The point of mesh isn't just importing files — it's also moving the
computation. `wanix cpu` dials B's exec plane (ALPN `wanix/cpu/1`,
served because step 2 passed `--cpu`), **reverse-exports** A's working
directory as a scoped read-only namespace, and asks B to run a task whose
world *is* that reverse export. The task runs on B's CPU; every file it
opens — including the program itself — resolves through the reverse session
back to A. (B's own files are *not* in the job's world: the cpu split is
caller's namespace, server's processor.)

Parsing lives in [`crates/wanix-cli/src/cpu/parse.rs`][cpu-cli], the dial
and reverse-export in [`crates/wanix-cli/src/cpu/run.rs`][cpu-cli], the
serve gate in `crates/wanix-cli/src/mesh/serve_cpu.rs`, and the session
core in [`crates/wanix-cpu/src/`][mesh-cpu].

From A's `$ROOT_A` (which holds `build.js` from step 1):

```sh
cd "$ROOT_A"
wanix cpu --node "$NODE_B" -- qjs build.js
```

Executed stdout (captured stdout from the task running *on B*):

```
ran on node B against A's reverse-exported namespace
```

Exit code 0. The script ran on B, read from A's reverse-exported directory,
and the bytes came back on the cpu control stream. The grammar is enforced
in [`cpu/parse.rs`][cpu-cli]:

```sh
# Options must precede the `--`, the job command follows it.
wanix cpu --node "$NODE_B" -- qjs build.js

# Missing --node is refused at parse time:
$ wanix cpu -- qjs build.js
cpu requires --node iroh://PEER[?addr=IP:PORT] naming the data node
$ echo $?
2

# Missing `--` separator is refused at parse time:
$ wanix cpu --node "$NODE_B" qjs build.js
cpu requires `-- KIND PROGRAM [ARG ...]` after its options
$ echo $?
2
```

By default A's reverse export is **read-only**; pass `--write` to opt the
job's subtree into read-write so the remote run can write outputs back to A:

```sh
wanix cpu --node "$NODE_B" --cwd "$ROOT_A" --write -- qjs build.js
```

The export is scoped by [`wanix_cpu::ExportScope`][mesh-cpu] — paths outside
the subtree are denied, and the jail invariant is the
`the_exported_world_is_scoped_and_cannot_reach_outside_the_subtree` test in
`wanix-cpu`.

## 5. Trust boundary, in one line

The two transports both authenticate B as B before anything else happens:

- **9P import** (`mount-*`): A dials ALPN `wanix/9p/1`; iroh QUIC verifies
  B's ed25519 key against the ticket; the 9P attach grant table is keyed on
  the verified `PeerId`, not a client-claimed `uname`.
- **cpu exec** (`cpu`): A dials ALPN `wanix/cpu/1`; the acceptor admits A's
  verified peer against an allowlist (the test
  `an_unallowlisted_peer_cannot_run_a_cpu_job` is the wire-level deny case,
  and `mesh_serve_cpu_with_peer_refuses_other_identities` proves the CLI
  `--peer` scoping end to end). Serving `--cpu` at all is refused on the
  public endpoint, even with `--peer`/`--grant` or `--insecure-open`.

Neither path ever trusts an IP, hostname, or username. Mount the key, run on
the key. Anything else is the old web.
