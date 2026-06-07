# The Mesh: The Missing Half of 9P

Wanix is Plan 9 reincarnated. It has per-task namespaces, file-like services
(`#task`, `#term`, `#kv`, `#pipe`, `#agent`), programs that are just files you
run, and a 9P server that can export any of it over a socket. For a long time
that last sentence hid a quiet asymmetry. Wanix could **export** a namespace —
serve its files to the outside world — but it could not **import** one. It had
the 9P *server* and not the 9P *client*. It had half of 9P.

This post is about building the other half: `wanix-9p-client`, a synchronous
`FileSystem` that speaks 9P to a remote server and presents the remote tree as
an ordinary local filesystem. With it, you `bind` a `RemoteFs` at `/n/<node>`
and a remote namespace — regular files, `#term`, `#task`, all of it — becomes
part of your local namespace. That is Plan 9 *import*, realized. It is the
keystone of the Wanix mesh, and everything else — iroh transport for the open
internet, `NodeID` identity and capability grants, content-addressed blobs,
`cpu`-style "send the agent to the data" — is an incremental slice that rides on
top of this one.

## What We Built

We built `wanix-9p-client`: one crate whose center of gravity is a single
struct, `RemoteFs`, that implements the fully synchronous
`wanix_fs::FileSystem` trait by talking 9P to a server over any blocking byte
stream. It is the exact mirror of the existing `wanix-9p` server: where the
server decodes T-messages and encodes R-messages, the client encodes T-messages
and decodes R-messages. No new wire format. No async runtime. No transport of
its own. It reuses the codecs that already live in `wanix-protocol` and turns
them inside out.

Then we wired it into the CLI as three tiny `mount-*` verbs so you can watch it
work across two processes, and we pinned the whole thing down with an
end-to-end loopback test that drives real files **and** real service devices
(`#task`, `#term`) across a real socket and asserts the bytes are identical to a
purely local namespace.

Here is the demo, top to bottom, two terminals. Server side dials up a raw 9P
listener over a host directory:

```sh
# Terminal 1 — the server
SROOT=$(mktemp -d)
echo "hello from the server" > "$SROOT/greeting.txt"
mkdir "$SROOT/docs"
wanix-rust p9-listen --root "$SROOT" --addr 127.0.0.1:5640
# wanix-rust p9-listen: listening on 127.0.0.1:5640
```

Client side imports it and reads, writes, and lists through the mount:

```sh
# Terminal 2 — the client
wanix-rust mount-ls    tcp://127.0.0.1:5640
wanix-rust mount-cat   tcp://127.0.0.1:5640 greeting.txt
wanix-rust mount-write tcp://127.0.0.1:5640 docs/note.txt "written across the wire"
wanix-rust mount-cat   tcp://127.0.0.1:5640 docs/note.txt
```

A file written through the client mount appears in the served directory on the
server host. The full real transcript — captured, not fabricated — is in the
[Copy-Paste section](#copy-paste-input--output) below.

## Why: Import, Export, and `/n/`

Plan 9's deepest idea is not "everything is a file." It is that **the namespace
is per-process and you can rearrange it.** Each process carries its own view of
the filesystem tree, and two operations build that view:

- **export**: `exportfs`/`9p` hands a subtree to the network as a 9P service.
- **import**: `import` (and the kernel's `mount`) takes a remote 9P service and
  splices it into *your* namespace at a path you choose.

The convention for that path is `/n/`. In Plan 9 you `import` a machine's
devices and files under `/n/<name>` — `/n/sources`, `/n/kremvax`, `/n/junk` —
and from then on a remote file is just a path. `cat /n/lab/dev/mouse` reads a
mouse on another machine. No new API. No client library per service. The
network became transparent because *one* primitive — the file — already crossed
the wire, and *one* primitive — the namespace bind — already let you place it
anywhere.

This is why "everything is a file" is load-bearing rather than cute. If your
services are files, then a single file-transport protocol (9P) plus a single
placement operation (bind/mount) gives you network transparency *for free,
across every service at once.* You do not write a network client for the
terminal, and another for the task table, and another for the key/value store.
You write a 9P client once, and every file-shaped service any node exports
becomes reachable. The terminal device, the task spawner, the agent — they all
ride the same import.

Wanix already had the *placement* half. `wanix-vfs` has had Plan 9-style
`bind`/namespace resolution from early on, and the server (`wanix-9p`) could
export. What it lacked was the *transport-import* half: a `FileSystem` you could
bind that, when resolved, spoke 9P to somebody else. `RemoteFs` is exactly that
object. Bind it at `n/remote` (the relative Wanix spelling of `/n/remote`) and
every namespace operation that resolves into that subtree turns into a 9P
exchange on the wire. Import is realized. `/n/` is back.

### Why agents are the operators these mechanisms always needed

Per-process namespaces and file-shaped services are, historically, a tool
humans found a little too fiddly to use at full strength. Rearranging your
namespace per command, importing a remote device under `/n/`, composing
services by bind order — it is *powerful*, and it was *underused*, because the
ergonomic cost of constantly reshaping a private world fell on a person who
mostly wanted their shell to stay put.

An agent does not have that problem. An LLM operating a confined world as files
finds per-process namespaces **native**, not fiddly. It reads state by `cat`,
mutates by `write`, lists capabilities by `ls`, spawns work by writing to a
control file, waits on results by reading another. A namespace it can rearrange
is just a context it can shape to the task. The thing humans found too sharp to
hold all the time is the thing an agent reaches for naturally.

And Wanix already has the agent. The `#agent` device is an LLM you can `cat`; it
edits a confined Wanix world live, operates the full services namespace
(`#kv`, `#task`, `#term`), runs Wanix programs, answers as a network service
(`POST /agent`), and even delegates to other agents (`#agent/<id>/reply`). What
the agent did *not* have was **reach**. It could operate one world — the world
it was booted into. The mesh is what gives a confined agent the rest of the
network: bind a remote node at `/n/<node>` and the agent's `cat`/`write`/`ls`
vocabulary now extends, unchanged, to files and services on another machine.
The operator was already here. The mesh hands it the wider world to operate.

## The How: Architecture

### The keystone is one struct that mirrors `serve_stream`

The server's hot loop, `serve_stream`, is strictly serial: it reads one
T-message, dispatches it, writes one R-message, and repeats. The client is the
photographic negative of that loop. `P9Conn::rpc` encodes one T-message, blocks
reading R-message frames until the matching tag returns, and surfaces an
`Rlerror` as a typed error:

```rust
pub fn rpc<F>(&mut self, build: F) -> ClientResult<P9Frame>
where
    F: FnOnce(u16) -> Result<P9Frame, wanix_protocol::P9Error>,
```

Because the server answers one request at a time, the client keeps exactly one
request outstanding. `RemoteFs` holds the connection behind an
`Arc<Mutex<P9Conn>>`, so concurrent `FileSystem` callers *serialize on the
mutex* rather than racing on the wire. No protocol pipelining, no tag-matching
race, no reordering bugs — the simplest correct thing, matched to the server's
own shape.

`RemoteFs` itself is small. Each `FileSystem` method walks from the attach root
(`ROOT_FID`) to a guarded scratch fid, issues the matching 9P exchange, and lets
the guard clunk the fid on the way out:

```rust
impl wanix_fs::FileSystem for RemoteFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> { ... }
    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> { ... }
    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> { ... }
    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> { ... }
    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> { ... }
    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> { ... }
    fn rename(&self, old: &NormalizedPath, new: &NormalizedPath) -> FsResult<()> { ... }
    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> { ... }
}
```

That is the whole keystone: a `FileSystem` that is a 9P client. Once Wanix's
`Namespace::bind` can take *any* `FileSystem`, and `RemoteFs` *is* a
`FileSystem`, the import is automatic. `mount` is barely 150 lines of CLI glue
because the hard part is the trait implementation, and the trait was already the
universal currency of the whole system.

### The constant lift into `wanix-protocol`

The server and the client need the *same* numbers. A negotiated `msize` bounds
the entire 9P message, so the data-carrying body of an `Rread`, `Rreaddir`, or
`Twrite` must leave room for the fixed frame header and fields. The server uses
those overheads to size its *replies*; the client uses the identical overheads
to size its *requests* (`Tread`/`Twrite`/`Treaddir` counts). Rather than have
two copies drift apart, the overheads were lifted into the shared protocol
crate, `crates/wanix-protocol/src/p9/limits.rs`:

```rust
/// Layout: 4-byte size + 1-byte type + 2-byte tag + 4-byte count.
pub const RREAD_HEADER_LEN: u32 = 11;
pub const RREADDIR_HEADER_LEN: u32 = 11;
/// size(4) + type(1) + tag(2) + fid(4) + offset(8) + count(4).
pub const RWRITE_HEADER_LEN: u32 = 23;
```

There is even a unit test asserting `RWRITE_HEADER_LEN == 4 + 1 + 2 + 4 + 8 + 4`
so the wire-field arithmetic cannot rot. This is the small, boring discipline
that makes "mirror the server" actually hold: both halves of 9P now compute
framing from one source of truth.

### A sync state machine over existing codecs

`wanix-9p-client` does its own framing, deliberately. It reads the 4-byte size
prefix first, *checks it against the negotiated ceiling before allocating the
body*, then reads exactly that many bytes and hands them to the existing
`P9Frame::decode`. Every T-message constructor (`p9_tlopen`, `p9_tlcreate`,
`p9_treaddir`, `p9_tgetattr`, `p9_twrite`, …) and every R-message decoder
(`p9_decode_rlopen`, `p9_decode_rreaddir`, `p9_decode_rgetattr`, …) already
existed in `wanix-protocol` for the server. The client is "just" the orchestration:
negotiate `Tversion`/`Tattach`, walk, open, read, write, clunk. A state
machine, not a new protocol.

It even offers `9P2000.L.Google.2` during negotiation so a capable server can
collapse a walk-then-getattr into a single `Twalkgetattr` round trip; a base
`9P2000.L` server negotiates down transparently and the client just does the
two messages.

### The five corrections, and why each one matters

A first draft of a 9P client is easy to write and easy to get subtly,
dangerously wrong. The crate carries five corrections, each one a fix for a
specific way the naive version betrays you. These are not stylistic — each was a
real adversarial verdict against the obvious implementation.

**1. Frame-size ceiling vs. OOM.** The naive client reads the size prefix, then
`vec![0; size]`, then fills it. A hostile or buggy server declares a 1 GiB frame
and your importer allocates a gigabyte before it has read a single body byte.
The corrected `read_one_frame` rejects any declared size below the 9P header or
above the negotiated `msize` *before allocating the body*, poisoning the
connection:

```rust
if size > max_msize {
    return Err(ClientError::Poisoned(format!(
        "server declared a {size}-byte frame exceeding the negotiated msize {max_msize}"
    )));
}
```

There is a test that declares a 1 GiB frame, supplies only the prefix, and
asserts the ceiling fires before the (absent) body is ever read. An importer
must not be DoS-able by the thing it imports.

**2. Honest seekability vs. silent corruption.** Regular files are
offset-addressable; devices and service streams are *not* — the server reads
them sequentially and ignores `Tread.offset`. A naive client tracks a
client-side offset for everything and advances it on every read. Point that at
`#term/1/ctl` and you fabricate offsets the server ignores, then *believe* your
fiction — silent corruption. The corrected client does a `Tgetattr` at open time
and only marks a file seekable if the server reports it as a regular file. A
non-regular file reports `is_seekable() == false` and *refuses* `seek` with
`NotSupported` rather than inventing a position:

```rust
fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
    if !self.seekable {
        return Err(FsError::NotSupported);
    }
    ...
}
```

The e2e test reads `#term/1/ctl` twice across the wire and asserts it returns
the server's honest empty stream both times — no phantom-seek bytes.

**3. RAII fid/tag guards vs. leaks.** Every walk allocates a fid on the server.
A long-lived mount issues an enormous number of walks, opens, and reads. If any
error path between "walk produced a fid" and "operation consumed it" forgets to
clunk — including a panic unwinding through the caller — that fid leaks into the
server's map forever, and over a long session you exhaust the fid space.
`ScratchFid` is the leak guard: it holds a clone of the shared connection and
best-effort clunks its fid on `Drop`, on *every* exit path. When ownership
legitimately transfers to an open `RemoteFile`, `into_fid()` defuses the guard
so the fid is clunked exactly once, by the handle, when the file is dropped.
The pools recycle released fids and tags through free-lists so a long session
never wraps `u32`/`u16` or reuses the `NOFID`/`NOTAG` sentinels.

**4. Honest `read_dir` cost.** A naive `read_dir` either reads one page and
lies about completeness, or loops forever if a broken server never advances its
cookie. The corrected `read_all_entries` is a bounded `Treaddir` cookie loop:
it pages until an empty page, *and* it caps total entries (`MAX_ENTRIES`) and
total round trips (`MAX_ITERATIONS`), *and* it bails if the cookie fails to
advance — "a server that fails to advance the cookie would loop forever; stop
rather than trust a stalled stream." The listing is documented as a best-effort
snapshot, not an atomic view, and the per-entry metadata is honestly
listing-only (type from the dirent byte; size/mode are placeholders) unless a
`Twalkgetattr` batch supplies more. No pretending a directory browse is a
free per-file stat.

**5. Append via the server, never raced client-side.** To append, the naive
client stats the file for its length, then writes at that offset — and races any
other writer between the stat and the write. The corrected client passes
`O_APPEND` to `Tlopen` and lets the *server* seek to end before each write. The
client sends a stable zero offset (which the server ignores for append fids)
rather than computing a position it cannot trust:

```rust
// Append fids let the server seek to end; the offset field is ignored
// server-side, so the client sends a stable zero rather than racing a
// size probe.
let offset = if self.append { 0 } else { self.offset };
```

Each correction is the difference between a client that demos and a client you
would trust to import a node you do not control.

### The control-plane / data-plane split that's coming

Today `RemoteFs` is one connection: one mutex, one request at a time, control
and data interleaved on the same serial stream. That is exactly right for the
first slice and exactly wrong for a busy mesh node serving large blobs. The next
shape separates a **control plane** (cheap, latency-sensitive: walks, stats,
opens, service-file pokes) from a **data plane** (bulk reads/writes that should
not head-of-line-block a `#term` keystroke). The `Arc<Mutex<P9Conn>>` boundary
is deliberately the only thing that would have to change; the `FileSystem`
surface above it stays put.

### The dependency layering: the core stays transport-free

The most important architectural fact about this slice is what it *did not*
touch. `wanix-9p-client` depends on exactly two crates:

```toml
[dependencies]
wanix-fs = { path = "../wanix-fs" }
wanix-protocol = { path = "../wanix-protocol" }
```

No transport. No async runtime. No iroh — there is no `iroh` dependency
anywhere in the workspace yet, by design. The client takes a `Box<dyn Duplex>`
(anything `Read + Write + Send`) and is done. A TCP stream, a Unix socket, a
pipe pair, and — when the next slice lands — an iroh stream all satisfy
`Duplex` identically. The core filesystem (`wanix-fs`), the namespace layer
(`wanix-vfs`), and the protocol codecs (`wanix-protocol`) remain free of any
network engine. When iroh arrives it arrives as *one more `Duplex`*, in a crate
above this one, and not a single line of `wanix-fs`/`wanix-vfs`/`wanix-protocol`
moves. That is the whole point of layering it this way: the mesh is incremental
because the keystone is transport-agnostic.

## Copy-Paste: Input / Output

Everything below is real. Run from the worktree root
(`cd /Users/jesse/lw/wanix-qemu-phase0`).

### (a) The end-to-end mesh test passes

```
$ cargo test -p wanix-cli --test mesh_loopback
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.05s
     Running tests/mesh_loopback.rs (target/debug/deps/mesh_loopback-e712d59983a5fb6a)

running 5 tests
test create_dir_then_read_dir_shows_entry_over_wire ... ok
test regular_file_round_trips_through_mounted_remote ... ok
test task_service_files_cross_the_wire_identically_to_local ... ok
test mounted_remote_bytes_match_a_purely_local_namespace ... ok
test term_service_stream_reads_exact_server_bytes ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

These five tests are the truth check. They stand up a `P9Server` over a real
loopback `TcpStream` on a background thread, dial it, build a `RemoteFs`, bind
it at `n/remote`, and assert, byte-for-byte against an identically-built local
namespace, that:

- a regular file written over the wire reads back identically (and the server's
  `MemFs` observed the exact bytes, and the file reports `is_seekable() == true`);
- `create_dir` then `read_dir` shows the new entry over the wire;
- `#task/self/kind == noop`, `#task/self/id == 1`, and the `#task/new` listing
  is `auto, noop` — proving **`#`-devices**, not just plain files, cross the
  socket;
- `#term/new` read twice returns live server state (`1\n` then `2\n`, identical
  to a local device — no cached/fabricated buffer), the `#term` listing then
  shows `1, 2, new`, and a `#term/1/ctl` stream read returns the server's honest
  empty bytes twice (no phantom-seek corruption).

The supporting unit suites are green too: 27 tests in `wanix-9p-client` (frame
ceiling, fid/tag pools, seek math, dirent mapping, version detection), the full
`wanix-cli` library suite at 289, plus 9 `shared_vfs_differential` tests. The
whole workspace passes under `just check` (fmt + module-lines + clippy
`-D warnings` + `cargo test --workspace --locked`).

### (b) A live two-process import

This is the same keystone, but across two real OS processes over a real TCP
socket. `serve` is the HTTP/WebSocket surface; the *raw-TCP 9P* listener that
`mount-*` dials is `p9-listen`. The client opens a `TcpStream` and immediately
speaks 9P framing, which is exactly what `p9-listen` answers.

Server (terminal 1):

```
$ SROOT=$(mktemp -d) && echo "hello from the server" > "$SROOT/greeting.txt" && mkdir "$SROOT/docs"
$ wanix-rust p9-listen --root "$SROOT" --addr 127.0.0.1:5640
wanix-rust p9-listen: listening on 127.0.0.1:5640
```

Client (terminal 2):

```
$ wanix-rust mount-ls tcp://127.0.0.1:5640
docs
greeting.txt

$ wanix-rust mount-cat tcp://127.0.0.1:5640 greeting.txt
hello from the server

$ wanix-rust mount-write tcp://127.0.0.1:5640 docs/note.txt "written across the wire"
wrote 23 bytes to n/remote/docs/note.txt

$ wanix-rust mount-cat tcp://127.0.0.1:5640 docs/note.txt
written across the wire
```

(`mount-cat` prints bytes verbatim with no trailing newline added — what you see
is exactly what the server delivered.)

Then the new directory entry shows up over the wire:

```
$ wanix-rust mount-ls tcp://127.0.0.1:5640 docs
note.txt
```

And the write **persisted on the server host** — the bytes crossed the wire,
went through the server's `Tlcreate`/`Twrite` path, and landed in the served
directory:

```
$ cat "$SROOT/docs/note.txt"
written across the wire

$ ls -la "$SROOT/docs"
total 8
drwxr-xr-x@ 3 jesse  staff   96 Jun  6 15:54 .
drwx------@ 4 jesse  staff  128 Jun  6 15:54 ..
-rw-r--r--@ 1 jesse  staff   23 Jun  6 15:54 note.txt
```

A file written through the client mount (`n/remote/docs/note.txt`) is the same
file the server reads off disk (`$SROOT/docs/note.txt`). That is import working
across two processes: 23 bytes that never existed on the client's disk, written
through a namespace bind, materialized in the server's directory.

The mechanics under those four client commands: each `mount-*` verb opens one
`TcpStream`, `RemoteFs::connect` negotiates `Tversion`/`Tattach`, the remote is
`bind`-ed into a fresh `Namespace` at `n/remote`, and the verb runs *one*
filesystem operation **through the namespace** — never a local shortcut. The
bytes genuinely traverse `Namespace -> RemoteFs -> 9P -> server`.

> Note on scope: `p9-listen` serves a host directory (`LocalFs`), so the live
> cross-process demo shows host **files** round-tripping. Services over the wire
> (`#term`, `#task`) are proven deterministically by `mesh_loopback.rs`, which
> serves a full Wanix `Namespace` (MemFs + `#term` + `#task`) and asserts the
> devices cross identically. Files in the live demo, services in the test —
> together they cover both halves of what import has to carry.

## The Plan 9 Lineage, and What's Next

It helps to name the three layers, because the mesh roadmap is just "fill them
in, in order."

**9P — the wire, unchanged.** The protocol Wanix already spoke as a server.
Nothing about the framing or the message set changes to add a client; we
mirrored it. 9P was designed to be network-transparent file access, and it does
not care which end you are. This slice simply stood up the end that was missing.

**import / export + `/n/` — this slice.** Export existed. Import is now
`RemoteFs`. The `/n/<node>` convention is now expressible: bind a remote at
`n/remote` and a remote tree — files and `#`-services alike — is part of your
namespace. This is the load-bearing first slice, and it is *done and green.*

**The forward look — riding iroh as the next slice.** Everything past here is
incremental, because it all rides the same `Duplex` and the same import:

- **cpu — send the agent to the data.** Plan 9's `cpu` command rebuilt your
  namespace on a remote machine and ran your shell there, with your local
  devices imported back. The Wanix version is "send the agent to the data": an
  agent operating `/n/<node>` is already operating a remote namespace by file;
  `cpu` is the same move with the *compute* relocated next to the files instead
  of the files streamed to the compute. The agent already runs Wanix programs;
  the mesh lets it run them *there*.
- **factotum via `NodeID`.** Plan 9's `factotum` held keys and spoke auth so no
  program had to. The mesh equivalent is `NodeID` identity plus capability
  grants: a node is named by its key, and importing its namespace is gated by a
  capability you were granted, not by where you happen to be on the network.
- **venti via content-addressed blobs.** Plan 9's `venti` was a
  write-once, hash-addressed block store. The mesh slice is content-addressed
  blobs: large or shared data referenced by hash, deduplicated and verifiable,
  exposed as files like everything else.
- **plumber via gossip.** Plan 9's `plumber` routed messages between programs by
  pattern. Across a mesh that becomes gossip: events and routing that span nodes,
  riding the same identity and transport.
- **iroh as the transport for all of it.** The reason these are *next* and not
  *now* is the transport. Loopback TCP proved the keystone; iroh provides the
  NAT traversal, relays, and direct connections that make `/n/<node>` reach a
  machine on the open internet instead of just `127.0.0.1`. And because
  `RemoteFs` only ever asked for a `Box<dyn Duplex>`, iroh lands as one more
  byte stream above a core that never learned its name.

The missing half of 9P is now built. Wanix could always export; now it can
import, and import is the primitive from which network transparency — and the
agent's reach — falls out. The mesh is incremental from here.

---

# Slice 2 — Identity: A Capability Is a Bind

The first slice gave Wanix the 9P *client* and, with it, import: bind a remote
at `n/remote` and another node's namespace becomes part of yours. But import as
built in Slice 1 was all-or-nothing. The server `p9-listen`-ed a host directory
and handed the *entire* tree to anyone who could open the socket. That is fine
for `127.0.0.1` and a single trusting user. It is exactly wrong for a mesh,
where the point is that *other people's nodes* import *yours*. The missing
piece is the boundary: **who** may import, and **how much** of your namespace
they get.

Slice 2 builds that boundary, and it builds it the Plan 9 way. The headline is
one sentence: **a capability is a bind.** A grant is not an ACL bolted onto the
side of the filesystem; it is a re-rooting of the namespace at a subpath, gated
by rights — the same `bind`-with-a-source-subpath that `wanix-vfs` already does,
plus a permission gate. The peer you granted attaches and lands inside a
`SubtreeFs` whose `.` *is* the subtree you gave them. They cannot name a path
outside it because, in their namespace, there is no outside.

## What We Built

- **`wanix-id`** — a new sync, iroh-free crate that owns identity and
  authorization: `NodeIdentity` (an ed25519 keypair, persisted owner-private and
  stable across restarts), `PeerId` (a node is named by its public key — the key
  *is* the address), `Grant` / `GrantTable` (a **default-deny**,
  live-editable capability table keyed by verified peer identity),
  `Authorization`, and the `AttachPolicy` trait with its `GrantTablePolicy`
  implementation. It depends only on `wanix-fs`, `wanix-vfs`, and
  `ed25519-dalek` — no transport, no async, testable over a pipe.
- **`SubtreeFs`** in `wanix-vfs` — the concrete form of a grant: a `FileSystem`
  that re-roots a backing filesystem at a prefix and gates every method through
  one central `require(right)` check, so a missing right is *always*
  `PermissionDenied`, enforced inside the filesystem itself and not only at
  attach time.
- **`P9Server::with_policy(default_root, peer, policy)`** in `wanix-9p` — the
  one new server entry point. When the policy is `None`, the server is
  byte-for-byte today's. When a policy is present, `handle_attach` consults
  `evaluate(peer, aname)` and installs the returned scoped root for that attach,
  or denies with EACCES. The same `handle_frame`/`serve_stream` hot loop runs
  unchanged underneath.
- **`p9-listen --peer HEX --grant ANAME:PREFIX:RIGHTS`** — the CLI surface that
  wires a default-deny `GrantTablePolicy` (scoped to the server's `--root`) into
  the real TCP transport. Without `--peer`, the listener keeps its prior
  all-or-nothing behavior, so nothing about the unguarded path changed.

## Why: factotum, `/n/`, and "the key is the address"

Plan 9 split *authentication* from *every other program*. A daemon called
`factotum` held your keys and spoke the auth protocols, so a file server never
had to know how to verify you — it asked factotum, and factotum answered.
Authorization, separately, was the file server's job: having learned *who* you
are, it decided *which files* you see, and the mechanism it reached for was the
one Plan 9 already had for everything — the namespace. You were given a view.
The files you were not authorized for were simply not in your tree.

The mesh inherits both halves, and iroh collapses the first one. In Slice 3 the
QUIC handshake authenticates the peer's ed25519 key *in the transport*: by the
time a connection is accepted, the peer's identity is already proven, and the
verified `PeerId` is the node's address simultaneously. There is nothing left
for an in-band `Tauth` to do — `Tauth` stays `ENOSYS` forever, which is the
whole meaning of "made cheap by iroh." Factotum's job became a property of the
connection.

That leaves authorization, and here the Plan 9 instinct is exact: **don't invent
an ACL layer; give the peer a namespace.** A grant says "peer K, attaching name
`projects/foo`, gets the host tree re-rooted at `projects/foo`, read-write." The
object that embodies that is a `SubtreeFs`, and a `SubtreeFs` is *just a
`FileSystem`* — the same universal currency the whole system already trades in.
The peer attaches and is standing inside the granted subtree. A walk toward
`../../docs` cannot escape because the peer's root *is* `projects/foo`; there is
no parent to walk to. This is the `/n/` discipline turned into a security
boundary: you do not filter a shared tree, you hand out *different trees*, and
the namespace does the confinement for free.

And, as with import itself, the operator this most empowers is the agent. A
default-deny capability table keyed by verified identity, where each grant is a
sub-namespace — that is a structure an agent reads by `ls`, extends by adding a
grant, and revokes by removing one. The capability table is itself meant to be a
filesystem an agent edits. Slice 2 builds the table and the enforcement; wiring
it to a `#grant` device an agent `cat`s and `echo`s to is a later edge, but the
shape is deliberately already file-shaped.

## The How: Architecture

### `evaluate(peer, aname) -> Authorization` is the whole trust boundary

Everything funnels through one pure function. `AttachPolicy::evaluate` takes the
verified peer and the requested attach name and returns either an
`Authorization { root, rights }` to install or `None` to deny:

```rust
pub trait AttachPolicy: Send + Sync {
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization>;
}
```

`GrantTablePolicy` implements it by consulting a `GrantTable` — an
`Arc<RwLock<Vec<Grant>>>` that is **default-deny** (a peer with no matching
grant is denied; there is no implicit allow) and matched *exactly* by
`(peer, aname)` with no wildcards. Because the table is cheaply cloneable and
shares one backing store, a grant added or revoked through one handle is visible
through every clone — which is exactly what lets the table be edited live while
the policy reads it on each attach.

When a grant matches, it materializes into the scoped root:

```rust
pub fn authorize(&self) -> Option<Authorization> {
    let root = SubtreeFs::new(Arc::clone(&self.backing), &self.prefix, self.rights).ok()?;
    Some(Authorization::new(Arc::new(root), self.rights))
}
```

That is the sentence "a capability is a bind" expressed in five lines: the grant
*is* the construction of a `SubtreeFs` re-rooted at the prefix with the rights.

### `SubtreeFs`: one `require` per method, rights enforced inside the filesystem

`SubtreeFs` holds the backing filesystem, a `NormalizedPath` prefix, and
`Rights { read, write }`. Every method does the same two steps: check the right,
then rebase the path under the prefix and call the backing. The check is
centralized so it cannot drift:

```rust
fn require(&self, access: Access) -> FsResult<()> {
    let granted = match access {
        Access::Read  => self.rights.read,
        Access::Write => self.rights.write,
    };
    if granted { Ok(()) } else { Err(FsError::PermissionDenied) }
}
```

A read-only grant denies *every* mutator — `create_dir`, `remove_file`,
`rename`, `symlink`, `set_permissions`, … — and also denies a write-mode `open`,
because `access_for_open` classifies `write || create || truncate` as
`Access::Write`. The rights live in the filesystem object, not only at the
attach gate, so even code that already holds a `SubtreeFs` cannot mutate past
its grant.

### The flow at attach time

```
peer K opens a stream ──► P9Server::with_policy(root, K, policy)
        │
        ▼  Tattach{ aname = "projects/foo" }
   handle_attach ──► policy.evaluate(K, "projects/foo")
        │                     │
        │            ┌────────┴─────────┐
        │         Some(auth)          None
        │            │                  │
        ▼            ▼                  ▼
  install auth.root  self.root =     Rlerror EACCES,
  as the fid's root  SubtreeFs       bind nothing
                     (projects/foo)
```

Single-attach-per-connection is the v1 simplification: the connection installs
one scoped root and serves it. Multi-attach scoping (a different `SubtreeFs` per
fid sub-namespace) is a fid-namespace change deferred to a later slice.

### The corrections this slice carries

**1. Default-deny, keyed by *verified* identity — not by client-claimed
`uname`.** A 9P `Tattach` carries a `uname` string the client picks. A naive
server trusts it. The mesh never does: the `GrantTable` is keyed by `PeerId`,
which in Slice 3 comes from the QUIC handshake, not from anything on the wire.
Over plain TCP (no QUIC yet) the peer identity is supplied *explicitly* via
`--peer`, making the trust assumption visible rather than smuggled. An empty
table denies everyone; there is no allow-by-default seam.

**2. The symlink-escape confinement bypass — found and closed.** This is the
sharp one, and the first draft got it wrong. Re-rooting only rewrites the path
*string*. On a symlink-*following* backing like `LocalFs`, a symlink planted
*inside* the granted prefix whose target points up-and-over to a sibling that is
still inside the backing root would resolve to out-of-prefix content — escaping
the grant while never leaving the `LocalFs` root, so `LocalFs`'s own root
confinement never fires. The fix re-confines every symlink-dereferencing method
through a new `FileSystem::confine_to_prefix` hook: `LocalFs` overrides it to
canonicalize the resolved target and reject anything outside the prefix, while
opaque-symlink `MemFs` keeps the no-op default. A planted
`../../docs/secret.txt` link inside a read-only `projects/foo` grant now returns
`PermissionDenied` on `open` *and* on a following `metadata`, while the *opaque*
`read_link` still returns the raw target bytes (it does not dereference) and a
`NoFollow` stat still reports the link itself. The earlier doc claim that "a
walk can never escape the prefix" was corrected: a *string* walk can't, but a
*symlink* walk could until this gate was added.

**3. `with_policy(None)` is byte-for-byte the old server.** The grant boundary
is strictly additive. The unguarded `p9-listen` (no `--peer`) constructs
`P9Server::new(root)` exactly as before; only `--peer` swaps in
`with_policy(root, peer, policy)`. There is a test asserting `build_serve_policy`
returns `None` without a peer, so the no-policy path can't silently acquire a
policy.

### Dependency layering: identity stays iroh-free

`wanix-id` is a sync crate with no transport and no async runtime, exactly as
the blueprint requires. It is testable over a local pipe today and reusable over
iroh tomorrow. Critically, **`SubtreeFs` lives in `wanix-vfs`, not in
`wanix-id`** — re-rooting at a subpath already exists in `Namespace::bind`'s
source-subpath logic, so the genuinely new part (the rights gate) lands next to
the re-rooting it extends, and `wanix-id` keeps only `Grant` / `GrantTable` /
`Authorization` / `AttachPolicy`. When iroh arrives in Slice 3 it injects a
`PeerId` from `Connection::remote_id()` into `with_policy`; not a line of
`wanix-id` or `SubtreeFs` has to move.

## Copy-Paste: Input / Output

Everything below is real, run from the repo root
(`cd /Users/jesse/lw/wanix-qemu`).

### (a) The grant boundary, enforced over real loopback TCP

These five tests stand up `P9Server::with_policy` over a real loopback
`TcpStream` on a background thread, dial it, and drive *raw 9P frames* — a
`Tversion`, then a `Tattach{ aname = "projects/foo" }`, then `Twalk` / `Tlopen`
/ `Tread` / `Twrite` — asserting the wire-level outcomes. They are the
authoritative form of the blueprint's "capability is a bind" demo, because they
send the exact `aname=projects/foo` the importer is meant to send:

```
$ cargo test -p wanix-cli --lib p9_listen::runtime::tests
running 5 tests
test p9_listen::runtime::tests::p9_listen_loop_continue_message_is_only_written_after_connection_errors ... ok
test p9_listen::runtime::tests::build_serve_policy_is_none_without_peer ... ok
test p9_listen::runtime::tests::read_only_grant_denies_writes_with_eacces ... ok
test p9_listen::runtime::tests::revoking_a_grant_denies_the_next_attach ... ok
test p9_listen::runtime::tests::authorized_peer_attaches_scoped_subtree_and_reads_it ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 293 filtered out; finished in 0.00s
```

What each one proves, over the socket:

- `authorized_peer_attaches_scoped_subtree_and_reads_it` — peer K is granted
  `projects/foo` read-write; it attaches `aname=projects/foo`, walks to
  `file.txt` (which exists *only* under the granted prefix), opens and reads it,
  and the bytes come back `granted`. The scoped root *is* `projects/foo`.
- `read_only_grant_denies_writes_with_eacces` — peer K is granted `docs`
  read-only; it attaches, walks to `readme.txt`, and the **write-mode open**
  returns `Rlerror` with errno **13 (EACCES)**. The write can never land because
  the fid was never opened for writing.
- `revoking_a_grant_denies_the_next_attach` — the first attach on the granted
  table succeeds (`Rattach`); the grant is then `revoke`-d on the *shared* table;
  the next connection's attach returns `Rlerror` EACCES. Revoke is observed live.
- `build_serve_policy_is_none_without_peer` — no `--peer` means no policy means
  the server is byte-for-byte the old one.

### (b) The `SubtreeFs` confinement, including the symlink-escape POC

```
$ cargo test -p wanix-vfs subtree::tests
running 9 tests
test subtree::tests::invalid_prefix_is_rejected ... ok
test subtree::tests::dot_prefix_reroots_at_backing_root ... ok
test subtree::tests::every_mutation_on_read_only_subtree_is_permission_denied ... ok
test subtree::tests::rebase_keeps_paths_inside_the_prefix ... ok
test subtree::tests::read_only_subtree_reads_inside_prefix ... ok
test subtree::tests::no_rights_denies_even_reads ... ok
test subtree::tests::read_write_subtree_allows_scoped_mutation ... ok
test subtree::tests::localfs_backed_subtree_allows_in_prefix_symlink ... ok
test subtree::tests::localfs_backed_subtree_denies_symlink_escape_of_prefix ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 28 filtered out; finished in 0.00s
```

`every_mutation_on_read_only_subtree_is_permission_denied` is the "one
`require` per method holds" check. `localfs_backed_subtree_denies_symlink_escape_of_prefix`
is the corrected POC: a `LocalFs` root holds a granted `projects/foo` and an
ungranted sibling `docs/secret.txt`, with a `../../docs/secret.txt` symlink
planted inside the prefix; opening or following that link through a read-only
`SubtreeFs("projects/foo")` returns `PermissionDenied`, while the in-prefix file
reads `inside` and the opaque `read_link` still returns the raw target bytes.

### (c) The identity and capability table unit suite

```
$ cargo test -p wanix-id
running 16 tests
test grant::tests::empty_table_denies_everyone ... ok
test grant::tests::grant_authorizes_only_matching_peer_and_aname ... ok
test grant::tests::clones_share_one_grant_list ... ok
test grant::tests::revoke_removes_the_grant ... ok
test grant::tests::authorized_root_is_scoped_and_rights_gated ... ok
test peer::tests::equality_is_by_key ... ok
test policy::tests::default_deny_when_no_grant_matches ... ok
test identity::tests::peer_id_is_the_public_key ... ok
test peer::tests::hex_round_trips_through_bytes ... ok
test tests::purpose_is_declared ... ok
test policy::tests::grant_table_policy_evaluates_through_the_table ... ok
test identity::tests::secret_bytes_round_trip ... ok
test identity::tests::distinct_generations_have_distinct_keys ... ok
test identity::tests::wrong_length_key_file_is_rejected ... ok
test identity::tests::persisted_key_is_owner_private ... ok
test identity::tests::load_or_create_is_stable_across_restarts ... ok

test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

`empty_table_denies_everyone` and `policy::tests::default_deny_when_no_grant_matches`
pin the default-deny posture; `peer::tests::equality_is_by_key` and
`identity::tests::peer_id_is_the_public_key` pin "the key is the identity is the
address"; `load_or_create_is_stable_across_restarts` and
`persisted_key_is_owner_private` pin the 0600-persisted, stable-across-restarts
node key.

### (d) Default-deny, live across two OS processes over TCP

This is the boundary running across two real processes over a real socket. The
server is started in policy mode — it grants a peer `projects/foo` (read-write)
and `docs` (read-only) — and an importer dials it:

Server (terminal 1):

```
$ SROOT=$(mktemp -d) && mkdir -p "$SROOT/projects/foo" "$SROOT/docs" \
    && echo "foo project file" > "$SROOT/projects/foo/main.rs" \
    && echo "secret docs"      > "$SROOT/docs/readme.txt"
$ wanix-rust p9-listen --root "$SROOT" --addr 127.0.0.1:5652 --once \
    --peer 2222222222222222222222222222222222222222222222222222222222222222 \
    --grant projects/foo:projects/foo:rw \
    --grant docs:docs:ro
wanix-rust p9-listen: listening on 127.0.0.1:5652
```

Importer (terminal 2):

```
$ wanix-rust mount-ls tcp://127.0.0.1:5652
failed to negotiate 9P session: 9P server returned errno 13
$ echo $?
1
```

Errno **13 is EACCES**: the default-deny table rejected the attach before any
walk. The grant table is keyed by `(peer, aname)`, and the `mount-*` client
attaches with the root `aname` (empty) — which matches *neither* the
`projects/foo` nor the `docs` grant — so a peer with no matching grant is
denied. That is the boundary doing exactly its job: even a client that reaches
the socket gets nothing it was not granted, by name.

> **Honest scope note.** The shipped `mount-ls`/`mount-cat`/`mount-write` verbs
> always attach with the root `aname` (Slice 1 had no notion of attaching a
> *named* subtree), so they cannot yet send `aname=projects/foo` from the CLI —
> which is why the *allow* side and the revoke step are proven by the
> `serve_stream`-level loopback-TCP tests in (a), which drive the exact
> `aname=projects/foo` frames, rather than by a `mount-*` command. The *deny*
> side is fully live across two processes here in (d). Teaching `mount-*` to
> carry an `--aname` (so the allow side is also a two-process copy-paste) is a
> natural Slice-3 follow-up, landing alongside the iroh transport that supplies
> the verified `PeerId` for real.

## What's Next

Slice 2 made the boundary; Slice 3 makes it reach. The `PeerId` that `--peer`
supplies by hand becomes the cryptographically verified `Connection::remote_id()`
from an iroh QUIC handshake; the `Box<dyn Duplex>` that `RemoteFs` already wants
becomes an iroh bi-stream; and `with_policy` is fed the real key with not a line
of `wanix-id` or `SubtreeFs` changed. Two laptops on two NATs, one namespace,
grants enforced by identity — that is the next slice, and the trust boundary it
needs is already built and green.

---

# Slice 3 — iroh: The Mesh Goes Real

Slice 1 built the 9P client — import realized over a byte stream. Slice 2 built
the trust boundary — a capability is a bind, keyed by a peer identity supplied by
hand over plain TCP. Both slices were deliberately transport-agnostic: `RemoteFs`
only ever asked for a `Box<dyn Duplex>` (anything `Read + Write + Send`), and the
grant table only ever asked for a `PeerId`. The whole design pointed at one
missing edge — a real transport that **supplies a cryptographically verified
identity** and **reaches a machine that is not `127.0.0.1`**.

Slice 3 lands that edge. It is iroh: QUIC with ed25519 node identities, NAT
traversal, and relays, where the public key *is* the address. A node binds one
iroh endpoint from its persisted node key, serves its Wanix namespace as 9P over
QUIC, and prints a dialable ticket. Another node mounts that ticket and the
remote namespace — files and `#`-services alike — splices into its own at
`/n/<node>`. The peer is authenticated by the QUIC handshake before a single
`Tattach` is served, and the Slice 2 grant table now keys on that *verified*
identity instead of a hand-supplied hex string. The mesh reaches the real
internet, and not one line of `wanix-fs`, `wanix-vfs`, `wanix-protocol`,
`wanix-9p`, or `wanix-9p-client` moved to make it happen.

## What We Built

- **`wanix-mesh`** — the only async crate in the workspace, and the only one that
  contacts iroh or tokio. It binds one `iroh::Endpoint` per node from the
  persisted `wanix_id::NodeIdentity` secret key, exports a Wanix namespace as 9P
  over QUIC under ALPN `wanix/9p/1`, and dials peers to import their namespaces as
  a `wanix_9p_client::RemoteFs`. The synchronous 9P core — `P9Server::serve_stream`
  and `RemoteFs` — runs **unchanged**; the async/sync boundary is bridged here and
  nowhere else.
- **`MeshNode`** — one node: a held multi-thread tokio runtime, an iroh endpoint
  bound from the node identity, and (when serving) an iroh `Router` dispatching
  ALPN `wanix/9p/1` to the protocol handler. It can bind `Public` (n0 preset:
  relays + DNS discovery, for crossing NATs) or `bind_local` (relays/DNS disabled,
  pinned to one IP socket — the testable, LAN, and loopback form). Callers get
  only synchronous values: a `PeerId`, an `EndpointAddr` ticket, and a dialer.
- **`P9ProtocolHandler` + the `BlockingDuplex` bridge** — the inbound side reads
  the verified peer id from the handshake and runs `serve_stream` inside
  `spawn_blocking`; the outbound `MeshDialer` connects, opens a bidi stream, wraps
  it in a `BlockingDuplex`, and hands it to `RemoteFs`. Both directions drive the
  async stream halves on a **held** runtime `Handle`, never `block_on` on a
  worker thread.
- **`wanix-rust mesh-serve` and the `iroh://` mount scheme** — the CLI surface:
  `mesh-serve --root DIR` binds an endpoint and prints `iroh://<peer-id>?addr=...`;
  the existing `mount-ls`/`mount-cat`/`mount-write` verbs now accept that
  `iroh://` ticket and dial the peer over QUIC, running *identically* to the
  `tcp://` path because both yield the same `RemoteFs`.

## Why: cpu, factotum, and "the key is the address"

Plan 9's `import`/`export` and the `/n/` convention assumed a network you could
name and reach: a file server at a known address, an auth server (`factotum`)
holding your keys, a flat trusted campus LAN. That assumption does not survive
contact with the modern internet — NATs, no stable addresses, no campus trust.
The Wanix mesh keeps the Plan 9 *mechanisms* and swaps the *transport* for one
built for exactly this world.

Two Plan 9 ideas collapse into the iroh handshake:

- **factotum becomes a property of the connection.** Plan 9 split authentication
  into a separate daemon so no file server had to speak auth protocols. iroh goes
  further: the QUIC handshake authenticates the peer's ed25519 key *in the
  transport*. By the time a connection is accepted, the peer's identity is already
  proven — `Connection::remote_id()` returns the verified `EndpointId`. There is
  nothing left for an in-band `Tauth` to do; it stays `ENOSYS` forever. The
  server never trusts a client-claimed `uname`; the cryptographic identity *is*
  the transport peer.
- **the node id is simultaneously the identity and the address.** This is the
  property that makes `/n/<node>` workable on the open internet. A node is named
  by its public key. The same 32 bytes that authorize it also let iroh *find* it,
  via relays and DNS discovery, across NATs you could never have addressed
  directly. A ticket is that key plus whatever direct addresses are known for a
  fast first hop. "Mount this node" and "trust this node" are the same act on the
  same bytes.

And this is the slice that makes the **agent's reach** real. An agent operating
`/n/<node>` over loopback was a demo; an agent that mounts a node behind a NAT in
another building, authenticated by its key, confined by a grant, is the mesh
doing its job. The operator the namespace mechanisms always wanted now has the
whole network to operate — by `cat`, `write`, and `ls`, over QUIC.

It is also the foundation for **cpu — send the agent to the data.** Slice 1
framed `cpu` as relocating compute next to files instead of streaming files to
compute. That move needs exactly what Slice 3 supplies: a node you can reach,
an identity that gates what it exposes, and a bidi stream you can reverse-export
a namespace over. Slice 3 is not cpu, but every primitive cpu needs is now on the
wire.

## The How: Architecture

### One endpoint, one identity, the sync core untouched

The center of `wanix-mesh` is `MeshNode`: it owns a dedicated tokio runtime and
binds one `iroh::Endpoint` from the node's persisted secret key. The endpoint
serves *and* dials over the same identity — there is no separate client identity.
Binding speaks the **real, pinned iroh API** (the blueprint warned that earlier
designs cited methods and versions that don't exist):

```rust
let endpoint = Endpoint::builder(presets::N0)   // not node_id(); not 0.102
    .secret_key(secret)
    .alpns(vec![WANIX_9P_ALPN.to_vec()])
    .bind().await?;
let peer = peer_id_for(endpoint.id());           // endpoint.id() -> EndpointId
```

iroh is pinned to `0.98.2` and every signature was validated against that pin:
`endpoint.id()` (not `node_id()`), `Connection::remote_id()` (not
`remote_node_id()`), `Router::builder(ep).accept(ALPN, handler).spawn()`,
`accept_bi`/`open_bi`. The ALPN is the wire contract `b"wanix/9p/1"` that selects
the 9P plane on a shared endpoint.

### The sync↔async bridge, designed not asserted

Every subsystem design flagged the sync/async bridge as *the* risk, and the
single sharpest hazard is concrete: **`Handle::block_on` panics if called from a
thread that is itself a runtime worker.** `wanix-mesh` answers it structurally, in
`duplex.rs`:

- **Inbound.** `P9ProtocolHandler::accept(conn)` reads `conn.remote_id()` once,
  then for each `accept_bi()` bidi stream runs the unchanged synchronous
  `serve_stream` inside `tokio::task::spawn_blocking` — the blocking pool, *not* a
  worker. The bidi stream's two halves are split into a `BlockingReader` and a
  `BlockingWriter`, each holding the runtime `Handle` explicitly (never
  `Handle::current`), so `block_on` always runs on a blocking-pool thread.
- **Outbound.** `MeshDialer::dial` runs connect/open on the runtime, wraps the
  stream halves in a `BlockingDuplex`, and hands it to `RemoteFs::connect`. The
  `FileSystem` methods then run on ordinary OS threads and `block_on` through the
  held `Handle`. The duplex satisfies `wanix_9p_client::Duplex` (`Read + Write +
  Send`) — it is just one more byte stream, exactly as Slice 1 promised iroh would
  be.

### The corrections this slice carries

**1. The first-write gotcha is real, and the handshake satisfies it.** An iroh
bidi stream is invisible to the peer's `accept_bi` until the *opener writes its
first byte*. A dialer that tried to read first would hang forever. The 9P
`Tversion` handshake writes immediately on connect — that first write is exactly
what makes the inbound side see the stream. The dialer's doc comment asserts this
dependency so a future refactor cannot quietly invert read/write order and
deadlock.

**2. Identity is read before any attach, and 0-RTT is off.** The handler never
calls `into_0rtt`, so `remote_id()` is the *proven* peer key, not an
optimistically-claimed one, before a single `Tattach` is served. The grant table
keys on that verified `PeerId` — the Slice 2 boundary, now fed a real
cryptographic identity instead of `--peer HEX`.

**3. Idle mounts must survive; in-flight work must be bounded.** 9P has no
keepalive: a healthy mounted-but-idle session simply blocks on the server's
next-request read, possibly for minutes ("open a mount, walk away"). So the
per-op deadline rides only on the server's *write* (where a stalled peer must not
park a thread), never on its idle read. There is a test —
`idle_mount_survives_past_the_op_deadline` — that sits idle four times past a
300 ms deadline and then succeeds on the same mount. The client keeps the
deadline on both halves, since its read always follows a request write.

**4. Slow-peer DoS is capped, stated, and sized.** Every live inbound session
pins one blocking-pool thread inside `block_on` on its idle read for the session's
lifetime. That cost is made explicit: a `Semaphore` admits at most
`MAX_CONCURRENT_SESSIONS = 512` live sessions, and the runtime's blocking pool is
sized to that plus headroom. A flood of half-open mounts cannot exhaust the
process; the per-op write deadline closes the stalled-response leg.

**5. Default-deny is enforced on the *global* transport, at parse time.** Serving
the public endpoint with no grant gate would export the whole root read-write to
anyone holding the ticket — the exact inversion of default-deny, now on a
transport that reaches the whole internet. `mesh-serve` *refuses* it: a public
serve with no `--peer`/`--grant` and no explicit `--insecure-open` is a usage
error, with guidance to gate it, pin it local, or opt in loudly. A
direct-address-only `--addr` endpoint (peers exchange tickets out of band) is
allowed without grants; the open public export must be deliberate.

### Dependency layering: async stays at the edge

`wanix-mesh` is the *only* crate that depends on iroh and tokio. It sits above
`wanix-9p-client` + `wanix-9p` + `wanix-id` + `wanix-vfs`, and everything it
exposes upward is iroh-free: an `Arc<dyn FileSystem>` (the imported `RemoteFs`), a
`PeerId`, an `EndpointAddr` ticket. The promise Slice 1 made — "when iroh arrives
it arrives as one more `Duplex`, and not a single line of
`wanix-fs`/`wanix-vfs`/`wanix-protocol` moves" — held exactly. iroh churn is
confined to one crate behind one pin.

## Copy-Paste: Input / Output

Everything below is real, captured not fabricated, run from the repo root
(`cd /Users/jesse/lw/wanix-qemu`). The live demo uses a local
direct-address-only endpoint (`--addr 127.0.0.1:PORT`, relays/DNS disabled) so it
needs no external network — loopback QUIC. The same `iroh://` ticket and the same
`mount-*` verbs work against a public NAT-crossing endpoint; only the binding
mode and the discovery path differ.

### (a) The end-to-end mesh-over-QUIC test passes

Five tests stand up two real `MeshNode`s on loopback (relay/DNS disabled), have
node A serve a Wanix namespace, and have node B dial A's direct `EndpointAddr`
ticket, build a `RemoteFs` over the bridged QUIC stream, bind it at `/n/A`, and
drive a full round trip — the unchanged sync 9P server and client, over real
QUIC, with the peer authenticated by its ed25519 key:

```
$ cargo test -p wanix-mesh --test mesh_quic
running 5 tests
test default_deny_denies_an_ungranted_peer ... ok
test service_devices_cross_quic_identically ... ok
test verified_peer_id_keys_a_read_only_grant ... ok
test regular_file_round_trips_through_quic_mount ... ok
test idle_mount_survives_past_the_op_deadline ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.44s
```

What each proves over QUIC: `regular_file_round_trips_through_quic_mount` —
`create_dir` + write + read-back, the server's `MemFs` observing the exact bytes
and the file reporting `is_seekable() == true`; `service_devices_cross_quic_identically`
— `#task/self/kind == noop\n` and a streamed `#term/new` read returns live server
state (`1\n`), so `#`-devices cross the wire, not just plain files;
`verified_peer_id_keys_a_read_only_grant` — node B's verified `PeerId` keys a
read-only grant on `docs`, B attaches `aname=docs`, reads it, and a write-mode
open is denied by the rights gate; `default_deny_denies_an_ungranted_peer` — an
empty grant table denies the attach (the QUIC connection succeeds, the 9P
`Tattach` is default-denied); `idle_mount_survives_past_the_op_deadline` — a
mount idle four times past the per-op deadline still serves a later op.

### (b) A live two-process import over QUIC loopback

Node A mesh-serves a host directory and prints its node id and a dialable ticket.
This is a local direct-address-only endpoint, so the ticket carries
`?addr=127.0.0.1:5680` for first contact and needs no relay.

Server (terminal 1):

```
$ SROOT=$(mktemp -d) && mkdir "$SROOT/docs" \
    && printf 'hello from node A over QUIC\n' > "$SROOT/greeting.txt"
$ wanix-rust mesh-serve --root "$SROOT" --key "$SROOT/../node.key" --addr 127.0.0.1:5680
wanix-rust mesh-serve: node 829fbb4aa611715420d2040ee8e894936ed222780127360c2f08b4238bd0f986
wanix-rust mesh-serve: mount iroh://829fbb4aa611715420d2040ee8e894936ed222780127360c2f08b4238bd0f986?addr=127.0.0.1:5680
```

The client (terminal 2) mounts that exact ticket and round-trips ls / cat /
write / cat through `/n/remote`, every operation a 9P exchange over the QUIC
stream:

```
$ TICKET='iroh://829fbb4aa611715420d2040ee8e894936ed222780127360c2f08b4238bd0f986?addr=127.0.0.1:5680'

$ wanix-rust mount-ls "$TICKET"
docs
greeting.txt

$ wanix-rust mount-cat "$TICKET" greeting.txt
hello from node A over QUIC

$ wanix-rust mount-write "$TICKET" docs/note.txt "written across QUIC"
wrote 19 bytes to n/remote/docs/note.txt

$ wanix-rust mount-cat "$TICKET" docs/note.txt
written across QUIC

$ wanix-rust mount-ls "$TICKET" docs
note.txt
```

And the write **persisted on the server host** — the bytes crossed the QUIC
stream, went through the server's `Tlcreate`/`Twrite` path, and landed in the
served directory:

```
$ cat "$SROOT/docs/note.txt"
written across QUIC
```

Nineteen bytes that never existed on the client's disk, written through an
`iroh://` ticket and a namespace bind, materialized in node A's directory across
a QUIC connection that authenticated A's ed25519 key on the way in.

### (c) Identity-keyed grant enforcement, live over QUIC

Node A serves the same root, but in grant-gated mode: a **default-deny** table
that grants *only* peer `2222…2222` the scoped subtree `docs` read-only. The
`mount-*` verbs attach with the root `aname` (Slice 1 has no notion of a named
attach), which matches no grant — so the default-deny table rejects the attach
before any walk.

Server (terminal 1):

```
$ wanix-rust mesh-serve --root "$SROOT" --key "$SROOT/../node2.key" --addr 127.0.0.1:5681 \
    --peer 2222222222222222222222222222222222222222222222222222222222222222 \
    --grant docs:docs:ro
wanix-rust mesh-serve: node b0afc03cd92636a572460d7579f502c53686a96193b69f690d54eba0a23b3299
wanix-rust mesh-serve: mount iroh://b0afc03cd92636a572460d7579f502c53686a96193b69f690d54eba0a23b3299?addr=127.0.0.1:5681
```

Importer (terminal 2):

```
$ wanix-rust mount-ls 'iroh://b0afc03cd92636a572460d7579f502c53686a96193b69f690d54eba0a23b3299?addr=127.0.0.1:5681'
failed to dial iroh peer: failed to negotiate mesh 9P session: 9P server returned errno 13
$ echo $?
1
```

Errno **13 is EACCES**: the QUIC connection succeeded and the peer's identity was
verified, but the default-deny grant table denied the attach. The grant boundary
from Slice 2 is now enforced on the QUIC transport, keyed on the verified peer.
The *allow* side — peer `2222…` attaching `aname=docs` and reading the scoped
read-only subtree, with a write denied by the rights gate — is proven over QUIC
by `verified_peer_id_keys_a_read_only_grant` in (a), which keys on node B's real
verified `PeerId` and drives the matching `aname=docs` attach (the shipped
`mount-*` verbs always attach the root `aname`, so the CLI shows the live *deny*
side; the test drives the live *allow* side over the same QUIC transport).

### (d) Default-deny is enforced before the network, too

Serving the public endpoint with no grant gate would export the whole root
read-write to anyone with the ticket. `mesh-serve` refuses that at parse time:

```
$ wanix-rust mesh-serve --root /tmp
mesh-serve on the public endpoint with no --peer/--grant exports the entire root read-write to anyone with the ticket; pass --peer HEX with --grant to gate access, --addr IP:PORT to serve a local direct-address-only endpoint, or --insecure-open to deliberately export it to the open internet
```

A local `--addr` endpoint (peers trade tickets out of band) or a grant-gated
public serve is allowed; the open global export must be opted into with
`--insecure-open`, which prints a loud warning.

## The Plan 9 Lineage, and What's Next

Slice 3 finishes the transport story the first two slices were built against.
9P is unchanged — the same frames, the same `msize`, the same fids, now over a
QUIC bidi stream. import/export and `/n/` are unchanged — `bind(RemoteFs, ".",
"n/<node>")` is still the whole move; only the `Duplex` underneath is now an iroh
stream instead of a TCP socket. The capability-is-a-bind boundary is unchanged —
`SubtreeFs` + `GrantTable` + `with_policy`, now fed a `PeerId` that the transport
*proved* instead of one a flag *claimed*. factotum's job moved into the QUIC
handshake; the node id became the address; and the agent's `cat`/`write`/`ls`
vocabulary now reaches any node it holds a ticket for.

What's next rides the same endpoint and the same identity:

- **cpu — send the agent to the data.** A reverse-exported caller namespace over
  a second bidi stream, plus remote `allocate_root` + `bind` + `start`, makes
  Plan 9's cpu(1) workable: run the task *on* the node holding the data, against
  its fast local namespace, outputs returning over the control stream. Every
  primitive it needs — a reachable node, a verified identity, a grant jail, a
  bidi stream — is now on the wire.
- **venti — content-addressed blobs.** A second ALPN (`iroh-blobs`) on the *same*
  endpoint moves bulk bytes — worlds, capsules, modules — peer-to-peer and
  BLAKE3-verified, so large data never crawls through the 9P `msize` window. The
  control plane references content by hash; the data plane ships it.
- **plumber — gossip.** A third ALPN (`iroh-gossip`) carries a `#plumb` topic bus
  for typed, best-effort coordination between agents and tools across nodes.

The missing half of 9P is built, the boundary that confines it is built, and now
the transport that carries both to the open internet is built. Wanix could always
export; it can import; it imports *only what it grants*; and it does all of it
across QUIC, addressed and authenticated by the same key. The mesh is real.

---

# Slice 4 — Service Devices on the Mesh: `#kv` and the Imported Service

The first three slices built the *pipe*: a 9P client (import), a grant boundary
(a capability is a bind), and a real transport (iroh QUIC, the key is the
address). Each one proved itself against files and against the two service
devices that already existed — `#term` and `#task`. But every demo so far carried
the same quiet caveat: the services that crossed the wire were ones the branch
*already had*. Slice 4 removes the caveat. It builds a **brand-new service device
the branch lacked** — `#kv`, a key/value store where each key is a file — and
then does *nothing special at all* to put it on the mesh. It binds into the
served namespace exactly like `#term` and `#task`, and because it is a
`FileSystem`, it imports for free. A node on the other side of a QUIC connection
reads `/n/A/#kv/config` and writes `/n/A/#kv/result` and operates node A's key
store as ordinary files — with no `#kv`-aware code anywhere in the client, the
transport, or the protocol.

That "for free" is the whole point of the slice, and the whole point of "the
namespace is the integration layer." You do not write a network client for the
key store. You wrote a 9P client once, in Slice 1, and a new file-shaped service
becomes reachable across every node the moment it exists.

## What We Built

- **`#kv` as a served service device.** `wanix_kv::KvDevice` already existed as a
  local device (an in-memory `BTreeMap<String, Vec<u8>>` where reading `#kv/<key>`
  returns its value, writing it sets the value, removing it deletes it, and
  listing `#kv` enumerates the keys). Slice 4 *binds it into the served mesh
  namespace*: `services_namespace_for_root` now binds `#kv` alongside
  `#term`/`#pipe`/`#agent`/`#task`, so `mesh-serve --wanix-services` exports a
  node whose `/n/A/#kv` is a live, mountable key store.
- **`mesh-serve --wanix-services`** — the CLI surface that exports the full Wanix
  services namespace (host root plus `#term`/`#pipe`/`#kv`/`#agent`/`#task`) over
  QUIC rather than a bare host directory, so a remote node operates node A's
  service *devices* as files, not just its on-disk files.
- **A streaming-honesty proof for a non-seekable service value.** A `#kv` value
  file is a regular file by type (so the Slice 1 client marks it seekable) but is
  *streamed* server-side — the server reads it from its own cursor and ignores
  `Tread.offset`. `kv_round_trip.rs` (sync, iroh-free, straight against
  `P9Server` over loopback TCP) and the QUIC tests in `mesh_quic.rs` pin that a
  purely sequential read of such a value crosses 9P **byte-exact**, including a
  40 000-byte value with position-dependent contents that would expose any
  off-by-N in the streaming reassembly.
- **The exec-device export gate (the security correction).** `--wanix-services`
  also binds `#task`/`#agent` — the *real* QuickJS/Wasm exec devices. A remote
  peer that could read `#task/new/qjs` and write `cmd`+`ctl` would run arbitrary
  WASI guest code on the serving host. So the flag is **refused on the public
  endpoint** regardless of `--peer`/`--grant`/`--insecure-open`, and allowed only
  on a local direct-address-only `--addr IP:PORT` socket whose ticket is traded
  out of band. Exec-device export stays local-trust only until public auth lands.

## Why: `#kv` Descends From the Plan 9 Service Device, and `/n/` Carries It

Plan 9's deepest move, the one every other idea hangs from, is that a *service*
is a *file tree*. The clock, the network stack, the process table, the mouse, the
authentication agent — `#c`, `#I`, the `/proc` files, `#m`, `factotum` — were not
libraries with APIs. They were synthetic filesystems: directories full of files
whose `read` and `write` *were* the operation. You did not call an API to read
the time; you `cat`'d a file. You did not call an API to kill a process; you
`echo`'d to `/proc/<pid>/ctl`. The "device" in "device file" is literal — a
kernel driver that presents itself as files, registered under a `#x` name.

`#kv` is exactly that pattern, applied to a key/value store. A key is a file. Its
value is the file's contents. `read` gets the value, `write` sets it, `remove`
deletes it, `ls` enumerates. There is no `kv_get`/`kv_set` API — there is a
directory and the ordinary file verbs. This is *modeled on the device shape that
already existed in the branch*: `#task` (where `#task/new/qjs` spawns and
`#task/<id>/ctl` controls) and `#term` (where `#term/new` allocates and
`#term/<id>/data` streams). `#kv` is the smallest possible new member of that
family — "a real database inside Wanix" reduced to its file-shaped essence.

And here the Plan 9 lineage pays off twice. Because `#kv` is a service device in
the Plan 9 sense — a `FileSystem` — it inherits the *other* Plan 9 mechanism for
free: `/n/`. Slice 1 made the point that "everything is a file" is load-bearing
precisely so that **one** file-transport protocol plus **one** placement
operation gives you network transparency *across every service at once*. `#kv` is
the first slice to *spend* that promise on a service the import client never knew
about. The terminal crossed the wire in Slice 1, the task table crossed in Slice
1 — but those were the services the demo author already had in hand. `#kv` is the
control experiment: a device written with no thought of the network, bound into a
namespace, and reachable at `/n/A/#kv/<key>` from another machine over QUIC with
not one line of `#kv`-specific code in the path. *That* is the proof that service
files cross nodes, because the service is the variable and everything else is
held fixed.

This is also where the agent's reach becomes concrete rather than rhetorical. An
agent that operates a key store as `cat`/`write`/`ls` over its *local* `#kv`
operates a *remote* node's `#kv` with the identical vocabulary the moment `/n/A`
is bound. "Read the config, compute, write the result" is the same four file
operations whether the store is in this process or on a node behind a NAT in
another building. Mount-there, run-here: the data stays put as files on node A;
the operator works it from node B.

## The How: Architecture

### `#kv` is a `FileSystem`, so the mesh path is the empty diff

The architectural heart of this slice is what it *did not* have to build. There is
no `KvOverMesh` adapter, no `#kv` case in `RemoteFs`, no `#kv` opcode in
`wanix-protocol`, no `#kv` branch in the QUIC handler. `KvDevice` implements
`wanix_fs::FileSystem` — `open`/`metadata`/`read_dir`/`remove_file` — and the
served namespace binds it:

```rust
namespace.bind(Arc::new(KvDevice::new()), ".", "#kv", BindOptions::default())?;
```

From there, every layer below is the unchanged stack from Slices 1–3. A client
walk to `#kv/config` resolves through the imported `RemoteFs`, which encodes a
`Twalk` + `Tlopen` + `Tread`, which rides the same `Arc<Mutex<P9Conn>>` over the
same `BlockingDuplex` over the same iroh bidi stream that carried `greeting.txt`.
The `#kv` device never learns it is on a network; the network never learns it is
carrying a key store. The bind is the integration.

### Where the Slice 1 seekability correction earns its keep

Slice 1's correction #2 — *honest seekability vs. silent corruption* — was
written for exactly this slice, and Slice 4 is where it gets spent. A `#kv` value
file reports `FileType::File`, so the client's open-time `Tgetattr` marks it
seekable and it tracks a client-side offset. But the server's `KvReadFile` is a
*stream*: it serves bytes from its own internal cursor and does **not** honor
`Tread.offset`. For a purely sequential read this is harmless — the client offset
and the server cursor advance in lockstep, so the bytes are exact. The danger is
the *seek*: the instant a client seeks such a file, its fictional local offset
diverges from the server's ignored-offset stream and the read silently corrupts.

The corrected client never invents that divergence on a sequential read, and the
tests prove it the hard way: a 40 000-byte value whose byte *i* is `i % 251`. Any
dropped, duplicated, or reordered byte across a chunk boundary shifts the pattern
and fails the comparison. It passes — sequential streaming of a non-seekable
service value is byte-exact across both loopback TCP and real QUIC.

### The correction this slice carries: exec-device export is local-trust only

The sharp finding in Slice 4 is a security one, and it is specific to *what*
`--wanix-services` binds. The flag is attractive because it exports the whole
service family — and that family includes `#task` and `#agent`, which are not
inert data. `#task/new/qjs` plus a `cmd`/`ctl` write *runs a QuickJS or Wasm
guest on the serving host*, with the served directory as its read-write
namespace. Exported to the open internet, that is remote code execution for
anyone holding the ticket.

So the gate is deliberately strict and stated at parse time:

- `--wanix-services` is **refused on the public endpoint** (no `--addr`),
  *regardless of* `--peer`/`--grant` (a grant's backing is the same services
  namespace, so even a grant-gated public serve would expose `#task` to the
  granted peer) and *regardless of* `--insecure-open` (which is file-sharing, not
  an exec backdoor).
- It is **allowed only on `--addr IP:PORT`** — a direct-address-only socket with
  relays/DNS disabled, whose ticket is exchanged out of band, i.e. local trust.
- The `--insecure-open` warning and the `mesh-serve` help line both *name the
  hazard*: `--insecure-open` exports file contents read-write, **not** the
  `#task`/`#agent` exec devices, so an operator cannot mistake file sharing for
  remote code execution.

This is the blueprint's "defer agent/exec-export until a grant/jail layer exists"
made concrete: the file-shaped key store crosses the public mesh; the
file-shaped *exec* devices do not, until public auth lands. The live `#kv` demo
below therefore runs on a `--addr` endpoint, which is exactly the local-trust
shape the gate permits.

## Copy-Paste: Input / Output

Everything below is real, captured not fabricated, run from the repo root
(`cd /Users/jesse/lw/wanix-qemu`). The live demo uses a local
direct-address-only endpoint (`--addr 127.0.0.1:PORT`, relays/DNS disabled) — the
local-trust shape the exec-device gate permits — so it needs no external network.

### (a) The `#kv`-over-mesh tests pass

Two suites pin the keystone. `mesh_quic.rs` drives two real `MeshNode`s over
loopback QUIC — node A serves the services namespace, node B dials A's ticket,
binds `RemoteFs` at `/n/A`, and operates `#kv`. `kv_round_trip.rs` drives the
same `#kv` device straight against `P9Server` over loopback TCP, sync and
iroh-free, isolating the seekability contract from the transport.

```
$ cargo test -p wanix-mesh --test mesh_quic
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.12s
     Running tests/mesh_quic.rs (target/debug/deps/mesh_quic-fcb370b80190eff7)

running 7 tests
test regular_file_round_trips_through_quic_mount ... ok
test service_devices_cross_quic_identically ... ok
test kv_service_operated_over_quic_mount ... ok
test default_deny_denies_an_ungranted_peer ... ok
test verified_peer_id_keys_a_read_only_grant ... ok
test large_kv_value_streams_across_quic_without_corruption ... ok
test idle_mount_survives_past_the_op_deadline ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.36s
```

```
$ cargo test -p wanix-9p-client --test kv_round_trip
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.18s
     Running tests/kv_round_trip.rs (target/debug/deps/kv_round_trip-a1262931eca1c227)

running 4 tests
test kv_value_reports_file_type_over_9p ... ok
test kv_value_streams_back_byte_exact ... ok
test kv_write_commits_on_close_over_9p ... ok
test large_kv_value_streams_without_fake_offset_corruption ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

`kv_service_operated_over_quic_mount` is the slice's headline test and the
deterministic form of the blueprint demo, run-here-equivalent to a qjs task:
node A seeds `config`, node B mounts `/n/A` over QUIC, reads `#kv/config` (the
streamed non-seekable value) back byte-exact, writes `#kv/result`, and the test
asserts the bytes that crossed QUIC are the bytes node A's live store now holds,
then reads `result` straight back over the mesh and lists `#kv` to see
`["config", "result"]`. `large_kv_value_streams_across_quic_without_corruption`
is the 40 000-byte position-dependent stream that would catch any seek-offset
slip; `kv_value_streams_back_byte_exact` and
`large_kv_value_streams_without_fake_offset_corruption` are its sync twins.

### (b) A live two-process import: node B operates node A's `#kv` over QUIC

Node A mesh-serves the Wanix **services** namespace (host root plus
`#kv`/`#term`/`#pipe`/`#agent`/`#task`) on a local direct-address-only endpoint
and prints its node id and a dialable ticket. The `#kv` store starts empty and
in-memory; it becomes node A's live key store as soon as a value is written into
it.

Node A (terminal 1):

```
$ SROOT=$(mktemp -d)
$ wanix-rust mesh-serve --root "$SROOT" --key "$SROOT/../A-node.key" \
    --addr 127.0.0.1:5684 --wanix-services
wanix-rust mesh-serve: node 986da0a25beef6984cb7c48c7b598ae7bec32b9e76329ca83ed5a8d8eee3ce95
wanix-rust mesh-serve: mount iroh://986da0a25beef6984cb7c48c7b598ae7bec32b9e76329ca83ed5a8d8eee3ce95?addr=127.0.0.1:5684
```

Node B (terminal 2) mounts that exact ticket and operates node A's key store as
files — every command a 9P exchange over the QUIC stream, resolving into the
imported `#kv` device. (`mount-*` binds the remote at `n/remote`; the path
`#kv/config` is the blueprint's `/n/A/#kv/config`.)

```
$ TICKET='iroh://986da0a25beef6984cb7c48c7b598ae7bec32b9e76329ca83ed5a8d8eee3ce95?addr=127.0.0.1:5684'

# node A authors a config key in its store (the value A owns):
$ wanix-rust mount-write "$TICKET" '#kv/config' 'region=us
replicas=3'
wrote 20 bytes to n/remote/#kv/config

# node B reads node A's config over QUIC — the streamed, non-seekable value,
# byte-exact:
$ wanix-rust mount-cat "$TICKET" '#kv/config'
region=us
replicas=3

# node B writes a result back into node A's key store over QUIC:
$ wanix-rust mount-write "$TICKET" '#kv/result' 'status=ok
built=42'
wrote 18 bytes to n/remote/#kv/result

# node B reads the result it just wrote, straight back over the mesh:
$ wanix-rust mount-cat "$TICKET" '#kv/result'
status=ok
built=42

# node B lists node A's key store:
$ wanix-rust mount-ls "$TICKET" '#kv'
config
result
```

And the value **lives in node A's store**, not node B's session. A *fresh* client
session (a new mount, a new QUIC connection) reads it straight back — proving the
18 bytes B wrote persist in node A's in-memory `#kv` device, not in any client
buffer:

```
$ wanix-rust mount-cat "$TICKET" '#kv/result'
status=ok
built=42
```

`#kv` is in-memory and reachable *only* through the mount, so those 18 bytes
never existed on node B's disk. They were written through a namespace bind, across
a QUIC connection that authenticated node A's ed25519 key on the way in, into a
key store on node A that node B operates entirely as files. That is a service
device — not a plain file, a *device* — crossing nodes for free, with no
`#kv`-aware code anywhere on the path. Mount-there, run-here, realized.

> **Note on scope.** The shipped live verbs are `mount-ls`/`mount-cat`/
> `mount-write`, so the *live two-process* transcript drives `#kv` from the CLI
> rather than from inside a qjs task; the qjs-task-equivalent — node B's task
> resolving `/n/A/#kv` through its own namespace, reading `config` and writing
> `result` — is proven deterministically over real QUIC by
> `kv_service_operated_over_quic_mount` in (a). The CLI shows the files crossing
> live across two processes; the test shows the same operations driven through a
> bound namespace exactly as a task would. Together they cover both halves of the
> blueprint's "an agent reads a remote key store" demo.

## The Plan 9 Lineage, and What's Next

Slice 4 spends the promise the first three slices built. 9P is unchanged. The
import client is unchanged. The grant boundary is unchanged. The QUIC transport is
unchanged. The *only* new thing is a new service device — `#kv`, the smallest new
member of the `#task`/`#term` device family — and the discovery that putting it on
the mesh is an empty diff, because a service that is a `FileSystem` rides `/n/`
for free. The control experiment ran: hold the network fixed, vary the service,
and watch it cross nodes with no special case. It crossed.

What's next rides the same endpoint, the same identity, and now the same
service-device-on-the-mesh pattern:

- **venti — content-addressed blobs.** `#kv` proved a *small* value streams over
  the 9P window honestly. The next device, `#cas`, references *large* data by
  BLAKE3 hash and moves it on a second `iroh-blobs` ALPN on the same endpoint, so
  a 50 MB world never crawls through the `msize` window. The 9P control plane
  names content by hash; the blob data plane ships it.
- **cpu — send the agent to the data.** A node that exports `#kv`/`#task` as files
  is one reverse-exported namespace away from running the task *on* the node
  holding the data. The exec-device gate this slice built — exec export is
  local-trust only — is precisely the boundary cpu must respect when it lands.
- **plumber — gossip.** A `#plumb` topic bus on a third ALPN routes typed events
  between agents and tools across nodes, the same way `#kv` routes values: as
  files, over the mesh, for free.

Wanix could always export; it can import; it imports only what it grants; it does
it across QUIC; and now a *new* file-shaped service crosses every node the moment
it exists, with no network code of its own. The integration layer is the
namespace. The mesh keeps its promise.

---

# Slice 5 — The Data Plane: Content-Addressed Blobs (venti)

Every slice so far moved bytes through the 9P window. That is right for walks,
stats, small reads, directory listings, mutations, and streaming service files —
the *control* plane, where being chatty buys you being simple. It is exactly
wrong for bulk: a 50 MB rootfs crawling through the `msize` window, one bounded
`Tread` at a time, over a serial connection, across a NAT, is a tax you pay on
every byte and never stop paying. Plan 9 already knew the answer. **venti** split
the archival store off from the file protocol: large, immutable data is named by
the hash of its content, stored once, deduplicated globally, and fetched by score
instead of by path. Slice 5 is venti for the mesh.

The slice adds a second plane that runs beside 9P on the *same* endpoint and the
*same* identity: a content-addressed blob store. Large file bytes, frozen worlds,
and module inputs move as BLAKE3-addressed blobs, peer-to-peer, **verified
end-to-end while they stream**. The 9P control plane keeps naming things; it just
stops carrying the heavy ones. And the headline capability the plane unlocks is a
**capsule**: freeze a whole Wanix world — a directory tree the agent built — onto
the blob plane, get back one short token, and ship the entire world to another
node by handing it that token. The other node fetches the world, verifies every
blob, and materializes it, with shared files deduplicated automatically.

## What We Built

- **`ContentHash` — a BLAKE3 content address (venti's score).** A 32-byte newtype
  living in `wanix-fs` (so it sits below everything and creates no dependency
  cycle), with `from_hex`/`to_hex` for the 64-character share form. `hash_bytes`
  is the one hashing entry point; a blob's name *is* the hash of its bytes, so
  identical content always lands on the same name — dedup is not a feature you
  add, it is what content addressing *is*.

- **`ContentStore` + `LocalCasStore` — the venti store (sync core).** A
  dependency-free trait — `put(bytes) -> ContentHash`, `get(&hash) -> bytes`,
  `has(&hash)` — and an on-disk implementation that reuses
  `wanix-module-cache`'s audited boundary verbatim: owner-private directory,
  atomic write-then-rename, and an fd-based verification of the bytes before they
  are trusted. `get` re-hashes on the way out, so a corrupted or substituted blob
  is caught at read time, not assumed away. The store directory is
  `$WANIX_CAS_DIR` or a per-user default; `--store DIR` overrides it.

- **The `#cas` device — venti as files.** `#cas/<hash>` reads a blob,
  `#cas/ingest` is write-then-read-the-hash, `#cas/have/<hash>` reports presence.
  It is a `FileSystem` like every other device, so it binds into the served
  `--wanix-services` namespace next to `#term`/`#task`/`#kv` and — by the now
  familiar Slice 1 rule — imports across the mesh for free at `/n/A/#cas`.

- **Capsules — venti applied to a whole world.** `Capsule::freeze(store, dir)`
  walks a directory tree, ingests **each file as one blob** (so identical files
  across the tree, or across worlds, collapse to a single blob), builds a
  deterministic sorted `WorldManifest` mapping each world-relative path to its
  blob hash, and ingests *that manifest* as a blob too. **The manifest blob's hash
  is the capsule id** — the share token. `Capsule::load(store, id)` fetches the
  manifest blob by id, parses it under the safety caps, and
  `materialize(store, dir)` fetches every referenced file blob and writes the
  world out. The manifest is the HashSeq: a flat, content-addressed listing of the
  world. The capsule id is the BlobTicket's payload; in the local CLI it is the id
  alone, and on the mesh `wanix-mesh` wraps it with A's direct addresses so a peer
  can dial.

- **The CAS-aware control/data split over 9P.** `FileSystem::content_hash(path)`
  is a new default-`None` hook. The server surfaces it as a genuine `cas.hash`
  synthetic xattr: a `Txattrwalk("cas.hash")` walks to a read-only fid over the 64
  hex bytes (a file with no offloadable hash returns `ENODATA`, so the client
  cleanly falls back to a plain `Tread`). The client's `RemoteFs::content_hash`
  learns the hash over the wire through the iroh-free `wanix-9p-client::cas`
  module. So a CAS-aware caller seeing a hash on a large file **skips the `Tread`
  loop and pulls the bytes off the blob plane instead** — BLAKE3-verified — while
  the 9P control plane only ever carried the 64-byte hash. This is the
  control-references-content-by-hash, data-moves-peer-to-peer split, made real
  over the actual wire rather than asserted as a local Rust API.

- **`IrohCasStore` — the blob plane on QUIC.** In `wanix-mesh` (the one async
  crate), the same `ContentStore` trait is backed by `iroh-blobs` on a second ALPN
  registered on the *same* `Router` as `wanix/9p/1` — one endpoint, one identity,
  two planes. A fetch is `endpoint.connect(peer, iroh_blobs::ALPN)` then
  `store.remote().fetch(conn, HashAndFormat::raw(hash))`; a local read is
  `store.get_bytes(hash)`. (The blueprint's earlier `downloader().download(...)`
  shape was a fiction caught in review; the code and its docs now describe the API
  that actually exists.)

## Why: venti, the Content-Addressed Half Plan 9 Split Off

Plan 9's file server (`fossil`) and its archival store (`venti`) were two
different things on purpose. `fossil` spoke the file protocol — names, walks,
mutable directories, the live tree you edit. `venti` spoke content: you handed it
a block, it returned a 20-byte SHA-1 *score*, and that score was both the block's
permanent name and the proof of its integrity. Write the same block twice and you
got the same score and stored it once — dedup fell out of the addressing.
Periodically `fossil` would freeze a snapshot of the whole tree *into* venti,
turning a mutable world into an immutable, hash-named, shareable archive. The file
protocol named the live; venti named the frozen-and-bulk.

Slice 5 is that split, line for line, on the mesh — with BLAKE3 standing in for
SHA-1 (modern, faster, and verifiable *while streaming* rather than only after):

- **A blob is a venti block.** `ContentHash` is the score; `put`/`get` is
  `write`/`read`; `has` is the presence probe. Storing the same bytes twice stores
  one blob.
- **A capsule is a fossil snapshot frozen into venti.** `freeze` turns a mutable
  directory tree into an immutable manifest-of-hashes whose own hash names the
  whole world. The two identical `lib.js` files in the demo collapse to one blob
  exactly as venti would have collapsed two identical blocks.
- **The capsule id is a venti score that also tells you where to fetch it.** A bare
  score said *what* a block was but not *where*; iroh's `BlobTicket` carries the
  hash *and* the provider's direct addresses, so the id is self-locating. Hand
  someone the id and they have both the name and the route.
- **The control/data split is fossil-vs-venti.** 9P stays the file protocol; it
  references bulk by `cas.hash` and never drags it through the `msize` window. The
  blob plane is the archival store; it moves the bytes, verified, on its own ALPN.

And the through-line the whole document keeps pulling on holds again: the
integration is the namespace. `#cas` is a `FileSystem`, so it crosses nodes for
free; a capsule is just blobs in that store; shipping a world is handing over one
hash.

## The How: Architecture, and the Corrections

The flow of `capsule save`:

1. `Capsule::freeze` recursively reads the world tree. Each regular file's bytes
   go through `store.put`, which returns the BLAKE3 hash; **symlinks are skipped**
   (a capsule freezes content, not link topology, and dereferencing one could
   escape the tree). Identical files return the same hash and so reference one
   blob.
2. The `{path -> hash}` map is sorted by path into a `WorldManifest`. Sorting makes
   the serialized form — one `"<hex-hash> <path>\n"` line per file — independent of
   directory-walk order, so the **same world always freezes to the same capsule
   id**. That blob form is then itself `put`, and its hash is the capsule id.

The flow of `capsule load <id>`:

1. `Capsule::load` fetches the manifest blob by the capsule id and parses it.
2. `materialize` fetches every referenced file blob from the store — locally from
   `LocalCasStore`, or **over QUIC from a provider peer via `IrohCasStore`** — and
   writes each one to its path under the target directory.

Three corrections from the blueprint are load-bearing and are baked in:

1. **Materialization is the trust boundary for an *incoming* world, so it is
   defensive by construction.** Every manifest path is re-validated through
   `wanix-fs`'s `NormalizedPath` and a `safe_join` that confines the result inside
   the target directory — a hostile manifest naming `../../etc/...` or an absolute
   path is rejected, not written. The sender's claim about a path is never trusted.

2. **Whole-blob `get` loads a blob fully into memory, so size and fan-out are
   capped.** A manifest is bounded at `MAX_MANIFEST_ENTRIES` (100k) entries and
   `CAPSULE_MANIFEST_MAX_BYTES` (16 MiB) serialized, and every file blob is subject
   to the store's `MAX_BLOB_SIZE` cap — enforced at *write* time in `#cas/ingest`
   so a remote peer can't stream gigabytes into host memory before the cap fires.
   One malicious ticket cannot OOM the loader or fill the disk.

3. **Every blob is BLAKE3-verified on the way out, not trusted because it arrived.**
   `get` re-hashes and compares against the requested hash; a substituted or
   corrupted blob fails with a hash mismatch and the load aborts. This is what
   "verifies every blob" means concretely, and the demo proves it by tampering with
   one blob and watching the load refuse.

The blueprint also forbade two tempting shortcuts that this code honors: the
`cas.hash` value rides as a real synthetic xattr file, **not** as bytes appended
to `Rgetattr` (this codebase's own `p9_decode_rgetattr` calls `cursor.finish()`,
which errors on trailing bytes); and the freshness guard is *not* fid-mode
scanning (`FidEntry` stores no mode) but a `CasFs` open-for-write invalidate plus
hash-on-close.

## Copy-Paste: Input / Output

The CLI demo runs the full venti round-trip with no network: node A freezes a
world and prints the capsule id; node B — a fresh, empty store that has received
A's content-addressed blobs (the exact bytes the blob plane fetches and verifies
over QUIC) — loads the world *by id alone*, verifying every blob and
materializing it, with the shared file deduplicated.

```sh
# Node A builds a world: two byte-identical libs (so dedup is observable),
# an executable init, and a main.js.
$ md5 world/a/lib.js world/b/lib.js
MD5 (world/a/lib.js) = 987f3cfa31d7d224eddecfe9a16bdeab
MD5 (world/b/lib.js) = 987f3cfa31d7d224eddecfe9a16bdeab    # identical content

# --- Node A: capsule save -> the capsule id (the BlobTicket payload) ---
$ wanix-rust capsule save world --store store-A
capsule ac8b46d6799fccb531e523dfb3a0a672162b582427ca66a22c8ca5a31c0be8df saved (4 files) from world
load with: wanix capsule load ac8b46d6799fccb531e523dfb3a0a672162b582427ca66a22c8ca5a31c0be8df <DIR>

# 4 world files, but only 4 blob OBJECTS on disk: 3 unique file blobs
# (the two libs deduped to one) + 1 manifest blob.
$ ls store-A
4846acf69b223aef6b1d99daa51f0bdbfc2991330c7c8c6622a27012b188a78b
66f858247deca4bbf5136d7417533195f91d1d0c79274701bf0347a4d8203e5e
87212e4bec07dc7e83bb05eb722b1f37e53d30abbec2ace92df86ed95aad7390
ac8b46d6799fccb531e523dfb3a0a672162b582427ca66a22c8ca5a31c0be8df   # == capsule id

# The manifest blob IS the capsule id; it is the HashSeq — one
# "<blob-hash> <path>" line per file. a/lib.js and b/lib.js carry the
# SAME hash, so they share one blob.
$ cat store-A/ac8b46d6799fccb531e523dfb3a0a672162b582427ca66a22c8ca5a31c0be8df
87212e4bec07dc7e83bb05eb722b1f37e53d30abbec2ace92df86ed95aad7390 a/lib.js
87212e4bec07dc7e83bb05eb722b1f37e53d30abbec2ace92df86ed95aad7390 b/lib.js
66f858247deca4bbf5136d7417533195f91d1d0c79274701bf0347a4d8203e5e bin/init
4846acf69b223aef6b1d99daa51f0bdbfc2991330c7c8c6622a27012b188a78b main.js

# --- Node B: a different machine, empty store, given only A's blob objects
#     (exactly the bytes iroh-blobs fetches+verifies over QUIC). It loads the
#     world by the capsule id alone. ---
$ wanix-rust capsule load ac8b46d6799fccb531e523dfb3a0a672162b582427ca66a22c8ca5a31c0be8df restored --store store-B
capsule ac8b46d6799fccb531e523dfb3a0a672162b582427ca66a22c8ca5a31c0be8df restored (4 files, 124 bytes) to restored

# The materialized world is byte-for-byte node A's world.
$ diff -r world restored && echo IDENTICAL
IDENTICAL

# "Verifies every blob" is real: tamper with one blob and the load refuses.
$ printf 'console.log("PWNED");\n' > store-evil/4846acf...main.js-blob
$ wanix-rust capsule load ac8b46d6...e8df restored-evil --store store-evil
capsule load: hash mismatch: requested 4846acf69b223aef6b1d99daa51f0bdbfc2991330c7c8c6622a27012b188a78b, got 141b7ff84dcebe1757a2b372449d4dca83893e55d5fefe55aa577490ee4dbdd5
# exit status 1 — the corrupted world never materializes.
```

The over-QUIC half — the part the CLI demo stands in for — is proven end to end
by `wanix-mesh`'s `mesh_blobs` test: two real iroh endpoints on loopback (relay
and DNS disabled, direct-address only), node A serving the blob plane on
`iroh_blobs::ALPN` from the *same* endpoint as its 9P service, node B fetching the
capsule by id over QUIC with each blob BLAKE3-verified, then materializing the
deduped world:

```sh
$ cargo test -p wanix-mesh --test mesh_blobs
running 2 tests
test put_and_get_round_trip_through_one_node_store ... ok
test ship_a_world_by_capsule_id_over_the_blob_plane ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.32s
```

## The Plan 9 Lineage, and What's Next

Slice 5 finishes the half of the mesh the file protocol was never meant to carry.
9P stayed the control plane it has been since Slice 1 — it references bulk by a
64-byte `cas.hash` and never drags the bytes through its window. Beside it now
runs venti: an immutable, BLAKE3-named, globally-deduplicated blob store on a
second ALPN on the same endpoint and the same identity. A capsule freezes a whole
mutable world into that store the way `fossil` froze a snapshot into `venti`, and
the manifest's own hash becomes the world's permanent, self-locating name. Ship
the world by handing over that name; the far node fetches the HashSeq, verifies
every blob, refuses any that fail, confines every path, and materializes the tree
— shared files arriving once.

What's next now has both planes to stand on:

- **cpu — send the agent to the data.** With a world reducible to one hash and
  fetchable peer-to-peer, `wanix cpu --world-ref <hash>` can pull a frozen world
  onto the compute node before running the task — or reverse-export the caller's
  namespace and run *on* the data node. The control/data split is the foundation;
  cpu is the move it enables.
- **plumber — gossip.** A `#plumb` topic bus on a third ALPN routes typed events
  between agents and tools; durable handoff between them rides exactly the blob
  plane built here — a capsule blob *is* the durable artifact a best-effort message
  points at.

Wanix could always export; it can import; it imports only what it grants; it does
it across QUIC; a new file-shaped service crosses every node the moment it exists;
and now a whole world is one verified, deduplicated hash you can hand to anyone.
The control plane names; the data plane carries; the namespace is still the
integration layer. The mesh keeps its promise.

# Slice 6 — cpu: Send the Agent to the Data

Every slice so far moved the *data* to the compute: import a remote namespace at
`/n/A`, then run a task here against bytes that crawl back over the 9P window. Slice
6 is the inverse, and the more powerful move. The caller stays put; the **job**
travels — onto the node that already holds the source tree, runs there against its
local fast namespace, and reverse-exports the caller's own files so the run can read
inputs and write outputs *back through the wire*. Exit status and captured output
return on a separate control stream. This is Plan 9's `cpu(1)`, generalized to the
open internet, with a default jail.

## What We Built

- **`wanix-cpu` — the synchronous, transport-agnostic cpu core.** A new crate
  depending on `wanix-fs + wanix-9p-client + wanix-9p + wanix-task + wanix-vfs` and
  **no network code at all**. It is generic over a `Duplex` (a bidirectional byte
  stream), so the entire cpu mechanism is exercised over an in-memory pipe or a
  loopback `TcpStream` with no async and no iroh — exactly the discipline that let
  the 9P client keep its keystone position. Network is `wanix-mesh`'s job; `wanix-cpu`
  is the protocol and the launch.

- **The acceptor (`run_job`) — the exact local launch, with a remote world.** Node Y
  runs `allocate_root` → `task.bind(world, ".", ".")` → configure → `start`,
  byte-for-byte the local task pattern. The only difference is the *world*: instead of
  a local `MemFs`, the task is bound against a `wanix_9p_client::RemoteFs` that proxies
  every `open`/`read`/`write`/`walk` over the **export stream** back into the caller's
  reverse-exported namespace. The crate never names a concrete runtime — the caller of
  `run_job` registers `qjs`/`wasm`/`noop` drivers on the `TaskTable` it passes in — so
  the dependency direction stays honest and a remote job runs *any* Wanix task kind.

- **The caller (`serve_export` + `drive_control`).** The caller runs its *own*
  `P9Server` over the export stream — it is the 9P **server** for the job, the inverse
  of every other slice — and drains a small typed `CpuEvent` batch off the control
  stream. When it reads the terminal `Exit`, it shuts its export half down, which is
  what lets the reverse server read EOF and stop.

- **`ExportScope` — the default jail.** The reverse export is *not* the caller's whole
  host root. `ExportScope::new(backing, "work")` re-roots to the job subtree and gates
  it **read-only by default** (`SubtreeFs` + `Rights`); `.writable()` opts the subtree
  into read-write so outputs land back; `.grant(GrantedService::new("#kv", …))` mounts
  explicitly-granted services into the job's world at their `#`-paths. A file outside
  the subtree is simply *not present* in the exported namespace.

- **`StreamRole` — a 1-byte discriminator, not a positional race.** A job uses two bidi
  streams (control + export). The caller writes one role byte (`control=0`, `export=1`)
  as the *first byte* on each, and the acceptor classifies each accepted stream by that
  byte — never by arrival order.

- **`CpuEvent` — the tiny control wire.** `kind(1) || len(4, LE) || payload`, with
  `Stdout`/`Stderr`/`Exit`/`Cancel` frames, the payload length bounded by
  `MAX_EVENT_PAYLOAD` (1 MiB) on decode so a hostile peer cannot force an unbounded
  allocation. It is deliberately *not* 9P — the control stream carries only out-of-band
  job lifecycle; the export stream carries the full 9P namespace traffic.

- **The `wanix-mesh` cpu plane and the `wanix cpu` CLI.** `MeshNode::serve_cpu` accepts
  on ALPN `wanix/cpu/1` behind a grant allowlist; `MeshNode::dial_cpu` is the caller.
  `wanix-rust cpu --node iroh://PEER -- KIND PROGRAM [ARG…]` binds an ephemeral dialer
  identity, reverse-exports the local `--cwd` (read-only unless `--write`), runs the job
  on the data node, and writes its captured stdout/stderr and exit code to the process.

## Why: cpu(1), and "Move the Computation, Not the Data"

Plan 9 had two ways to bridge two machines, and they were duals. `import` pulled a
remote namespace into yours — the file server's tree appeared under a local mount, and
your local programs ran against it. `cpu` did the opposite: it logged you into a remote
CPU server, started a shell *there*, and **reverse-mounted your terminal's namespace
back onto the remote machine** so the remote shell saw your files, your `/dev`, your
environment. You typed on your laptop; the compute happened on the fast machine; the
fast machine's view of "your files" was served back over the same connection from your
laptop. Compute went to where the cycles (or the data) were, and the namespace followed
the user, not the host.

Slice 6 is that, line for line:

- **`import` is Slices 1–5.** `RemoteFs` mounted at `/n/A` *is* Plan 9 import; a task
  bound against it runs here, against there.
- **`cpu` is this slice.** `run_job` binds the task against a `RemoteFs` whose far end is
  the **caller's** reverse-exported namespace. The job runs on node Y, but its world is
  node X's files — exactly cpu's reverse-mounted terminal namespace, only the "terminal"
  is now a scoped, content-jailed subtree and the transport is QUIC.
- **The two role-sorted streams are cpu's two channels.** cpu(1) multiplexed the
  interactive channel and the exported-namespace channel over one connection; here the
  control `CpuEvent` stream and the export 9P stream are two QUIC bidi streams, sorted by
  a role byte instead of by cpu's in-band muxing.
- **"Move the computation to the data" is the honest answer to 9P's WAN chattiness.** A
  walk+open+read is 3–5 serial round-trips; pulling a large tree through the `msize`
  window over a NAT is a tax. When chattiness dominates, you stop pulling the data and
  **send the job to it** — the run touches its files at local-namespace speed and only
  the small result comes back over the wire. cpu is the structural fix the whole document
  kept pointing at.

And the through-line holds once more: the job's world is *just a `Namespace`*, its world
root is *just a bound `FileSystem`*, the export is *just a `P9Server`* — the mesh added a
direction (server runs on the caller), not a new mechanism.

## The How: Architecture, and the Corrections

The shape of a job, end to end:

1. The caller opens two bidi streams and writes a role byte first on each:
   `write_role(control, Control)` (`0`), `write_role(export, Export)` (`1`). It builds an
   `ExportScope` over its `--cwd`, runs `serve_export(scope_root, export_stream)` (its own
   `P9Server`) on one thread, and `drive_control(control_stream, &mut output)` on another.
2. The acceptor accepts both streams, reads the leading byte of each, and sorts them with
   `read_role` — never assuming "the first one is control".
3. On the export stream the acceptor builds `RemoteFs::connect(export)` and runs
   `run_job(table, &spec, export, &mut control)`: `allocate_root(kind)`,
   `task.bind(Arc::new(remote_world), ".", ".")`, apply the spec (argv/env/cwd, mirrored
   into `#task`), and `table.start(id)`.
4. After `start` returns, the acceptor reads the task's captured stdout/stderr and exit,
   chunks them into `CpuEvent::Stdout`/`Stderr` frames, and writes a terminal
   `CpuEvent::Exit(code)`.
5. The caller's `drive_control` collects the chunks and returns on `Exit`; it then shuts
   its export half down, the reverse `P9Server` reads EOF, and the job is done.

Four corrections from the blueprint are load-bearing and are baked in, not hand-waved:

1. **Stream order is first-write, not open order — so a role byte, not a position.** Over
   QUIC an `open_bi` stream is invisible to the peer's `accept_bi` until its opener writes
   a first byte. "The first accepted stream is the control stream" is a race; the 1-byte
   `StreamRole` discriminator (`role.rs`) makes the pairing unambiguous regardless of
   which stream's first byte lands first.

2. **The reverse export is a scoped, read-only-by-default jail — never the whole host
   root.** `ExportScope` re-roots to the job subtree via `SubtreeFs` and gates it with
   `Rights`; the naive "export `services_namespace_for_root`'s whole root with
   client-controlled symlink following" is a remote-root hole and is *not* what runs. A
   path outside the subtree is absent from the namespace; a write to a read-only export is
   denied; `--write` is an explicit opt-in for write-back, and granted services are an
   explicit allowlist.

3. **Streaming is batch-after-`start`, stated plainly, not faked.** The current task model
   runs the guest to completion inside `TaskDriver::start` and only *then* has its buffered
   stdout. So v1 delivers stdout/stderr/exit as a **single batch after `start` returns**,
   not incrementally. Incremental streaming (a streaming-stdout `File` that pushes frames
   during eval) is a named, scoped follow-up — the `event.rs` doc says so in so many words.

4. **Cancel stops draining, not the remote computation — documented, not pretended.** The
   task driver has no abort hook, so a caller-emitted `CpuEvent::Cancel` tells the *caller*
   side to stop reading the control stream; the guest on node Y still runs to completion.
   Real remote cancellation = stream teardown, and the limitation is honest in the type's
   own doc comment.

One subtlety the acceptor gets right: `run_job` returning does **not** close the export.
`TaskTable::allocate_task` binds the task's own `#task` filesystem back into its namespace,
making an `Arc` cycle (table → task → namespace → `#task` → table) that no `Drop`/`Weak`
breaks. So dropping the local `table` drops neither the bound `RemoteFs` world nor its
export stream — the **caller** owns the export lifetime and closes it after it reads the
terminal `Exit`. Lifetime is driven by the control protocol, not by Rust drop order.

## Copy-Paste: Input / Output

The full Slice 6 path is proven two ways: the sync core over a real loopback socket (no
async, no iroh — the real `P9Server`/`RemoteFs` over a real `TcpStream`), and the same
core over a real QUIC connection between two iroh endpoints. Both run `allocate_root` →
`bind` → `start` against a reverse-exported world and return the result on the control
stream. The acceptor's task driver in the tests is a tiny `EchoWorldDriver` that reads its
program file *through the task namespace* (i.e. through the reverse export) and echoes it
to stdout — proving the job ran against the caller's files — without pulling a WASI runtime
into the lower crate. A real `qjs`/`wasm` driver honors the identical observable contract;
the CLI dialer registers them on the acceptor's table.

```sh
# --- The sync cpu core over a real loopback TCP socket (no async, no iroh) ---
# Two TCP loopback pairs stand in for the job's two role-sorted QUIC bidi streams.
# The caller serves a scoped, read-only `work` export holding build.js + a granted
# read-only #kv; the acceptor runs a task whose world IS that export, reads the
# caller's file over the reverse 9P session, and returns its output on control.
$ cargo test -p wanix-cpu --test cpu_job
     Running tests/cpu_job.rs (target/debug/deps/cpu_job-ffcef4718a07a8ba)

running 4 tests
test scoped_export_root_denies_paths_outside_the_subtree ... ok
test the_exported_world_is_scoped_and_cannot_reach_outside_the_subtree ... ok
test a_granted_service_imports_into_the_world ... ok
test cpu_job_runs_against_the_reverse_exported_world ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

The four tests are the demo's invariants, made executable:
`cpu_job_runs_against_the_reverse_exported_world` — the job reads the caller's
`work/build.js` *through the reverse export* (the world is the remote namespace) and echoes
`console.log('built on the data node')` back as stdout, exit 0; `the_exported_world_is_scoped…`
— reading a `secret.txt` that lives *outside* the `work` subtree fails the job (the jail
holds); `a_granted_service_imports_into_the_world` — the granted `#kv/config` imports into
the job's world and reads `region=us`; `scoped_export_root_denies_paths_outside_the_subtree`
— a direct check that a read-only export denies both an out-of-subtree path and a write-mode
open.

```sh
# --- The same cpu core over a real QUIC connection between two iroh endpoints ---
# Node A (data node) serves ALPN wanix/cpu/1 with node B explicitly allowlisted.
# Node B dials, reverse-exports its scoped world, and runs the job ON node A.
$ cargo test -p wanix-mesh --test mesh_cpu
     Running tests/mesh_cpu.rs (target/debug/deps/mesh_cpu-5710cf20a4dabb05)

running 2 tests
test an_unallowlisted_peer_cannot_run_a_cpu_job ... ok
test cpu_job_runs_on_the_remote_node_over_quic ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.42s
```

`cpu_job_runs_on_the_remote_node_over_quic` is the headline `wanix cpu --node <DATA> -- qjs
build.js` over real QUIC: the task runs on node A, reads node B's reverse-exported
`work/build.js` over the wire, and the captured stdout (`console.log('cpu ran on the data
node')`) and exit 0 come back on the control stream. `an_unallowlisted_peer_cannot_run_a_cpu_job`
is the trust boundary: with node A's allowlist denying everyone, the connect may open but the
acceptor admits no streams, so the job fails rather than executing — remote code execution stays
grant-allowlisted, never handed to an arbitrary NodeID.

The caller-side CLI is the demo's exact command surface, `wanix cpu --node <DATA> -- qjs
build.js`:

```sh
# The command form: reverse-export DIR read-only by default, run KIND PROGRAM on the
# data node against it; --write opts the export into read-write so outputs land back.
$ wanix-rust help | grep -A1 "cpu --node"
       wanix-rust cpu --node iroh://PEER[?addr=IP:PORT] [--cwd DIR] [--write] [--env KEY=VALUE ...] -- KIND PROGRAM [ARG ...]
         (Plan 9 cpu over the mesh: reverse-exports DIR read-only by default and runs KIND PROGRAM on the data node against it; --write opts the export into read-write)

# The grammar is enforced: options before `--`, the job command after it, --node required.
$ wanix-rust cpu --node iroh://peer
cpu requires `-- KIND PROGRAM [ARG ...]` after its options          # exit 2

$ wanix-rust cpu -- qjs build.js
cpu requires --node iroh://PEER[?addr=IP:PORT] naming the data node # exit 2

$ wanix-rust cpu --node "not a ticket" -- qjs build.js
mesh address must start with iroh://: not a ticket                  # exit 2
```

The over-QUIC `mesh_cpu` test is the live two-node stand-in for a `wanix cpu --node iroh://A
-- qjs build.js` against a `serve_cpu` data node: the same `run_job`/`serve_export`/
`drive_control` core runs unchanged over the real iroh transport. Wiring `serve_cpu` into a
long-running CLI data node (so two shells, not one test, drive it) is the small remaining
step; the mechanism, the jail, and the wire are all proven here.

## The Plan 9 Lineage, and What's Next

Slice 6 closes the loop the document opened with: Wanix could always *export* a namespace,
Slices 1–3 let it *import* one and run against it, Slice 4 carried service devices across,
Slice 5 carried whole worlds by hash — and now the **computation itself travels to the data**,
the other half of the import/cpu dual that Plan 9 always had and the real internet never made
workable. A job runs on the node that holds the data, against that node's fast local namespace,
with the caller's own files reverse-exported back through a scoped, read-only-by-default jail
and the result returned on a typed control stream. The streams are role-sorted by a byte, not a
position; the export is a sub-namespace, not the host root; the output is an honest batch, not a
faked stream; cancel is documented for what it is. `cpu(1)`, over QUIC, with a default jail.

What's next now has both directions of the dual *and* a way for jobs to find each other:

- **Incremental streaming.** A streaming-stdout `File` that pushes `CpuEvent` frames during
  eval, so a long build's output arrives live rather than as a post-`start` batch — the one
  named follow-up the batch-honesty correction set up.
- **The plumber and agents (Slice 7).** A `#plumb` topic bus routes typed events between
  agents and tools, and the agent layer dispatches a session to a local engine or a
  `RemoteEngine` proxying to `/n/<node>/#agent`. With import, cpu, and a message bus, an agent
  on one machine can edit a confined world on another, hand off a task over a topic, and `cpu` a
  sub-agent onto a third — agents operating a Plan 9 mesh as files, which is where this whole
  document has been heading.

Import pulled the world to you; cpu sends you to the world; the namespace is still the
integration layer, and now it travels in both directions.
