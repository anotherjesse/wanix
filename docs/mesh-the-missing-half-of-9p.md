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
