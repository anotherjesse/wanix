---
title: "#plumb — Plumber Bus"
slug: devices/plumb
pageType: reference
oneLiner: "#plumb/<topic>/send publishes a newline-JSON {kind,from,to,body} envelope; <topic>/recv reads envelopes received since it opened."
audience: [developer]
tags: [shipped, device, mesh, caveat]
sourceRefs:
  - crates/wanix-plumb/src/lib.rs:1-148
  - crates/wanix-plumb/src/files.rs:31-94
  - crates/wanix-plumb/src/envelope.rs:1-42
  - crates/wanix-plumb/src/buffer.rs:16-79
  - crates/wanix-plumb/src/path.rs:19-56
  - crates/wanix-plumb/src/local.rs:50-101
  - crates/wanix-mesh/src/plumb.rs:1-62
seeAlso:
  - concepts/best-effort-epidemic-delivery
  - concepts/blocking-stream-eof-contract
  - concepts/single-frame-serve-caveat
  - concepts/send-agent-to-the-data
  - concepts/service-devices
  - devices/index
prerequisites:
  - concepts/service-devices
  - concepts/blocking-stream-eof-contract
usedInFlows: []
honestLimits:
  - "Delivery is best-effort epidemic, not a durable queue: a subscriber that was not listening when a message was published never sees it, and there is no acknowledgement. Durable handoff belongs in #kv or a capsule blob."
  - "A live recv over a single serve connection deadlocks: serve handles one 9P frame at a time per connection, so a blocking recv read cannot interleave with later frames. The cockpit self-check probes the publish path only."
  - "A flooded subscriber loses data: the recv buffer is capped at 16x MAX_ENVELOPE_LEN and drops the oldest bytes under flood — the same lossy semantics gossip already has for a lagged reader."
---

# #plumb — Plumber Bus

`#plumb/<topic>/send` publishes a newline-JSON `{kind,from,to,body}` envelope; `<topic>/recv` reads envelopes received since it opened.

## What & why

Coordination between tasks, agents, and nodes does not get its own RPC layer in Wanix — it is two files in a directory. Write one JSON line to `#plumb/build/send` and every reader currently holding `#plumb/build/recv` open sees that line. There is no broker process, no subscribe API, no topic registry to provision against: a topic directory exists the moment you name it. This is Plan 9's plumber idiom — a typed, addressed message bus — rebuilt as a plain `FileSystem` (`crates/wanix-plumb/src/lib.rs:120`). Because it is just a filesystem, it imports across the mesh for free: `/n/A/#plumb/build/recv` reads node A's bus as ordinary files, with no device-specific networking code on either side.

## Topics exist on demand

There is no allocation step. Any single path segment is a valid topic name, and walking to it always succeeds: `metadata` on a topic returns a directory without ever touching state (`crates/wanix-plumb/src/lib.rs:143-145`). Each topic directory exposes exactly two files — `recv` and `send` (`crates/wanix-plumb/src/lib.rs:159-162`). The device root lists topics it has *locally touched*, but that listing is cosmetic: the bus itself is nameless gossip, so the root can only show names this node has opened, capped at `MAX_KNOWN_TOPICS = 4096` so a remote importer can't grow it without bound (`crates/wanix-plumb/src/lib.rs:58`). A topic works whether or not it appears in the listing.

Topic names are caller-controlled — over an imported `#plumb`, a remote 9P client supplies the name through a walk path — so they are length-bounded at parse time to 255 bytes (`MAX_TOPIC_LEN`, matching the conventional Plan 9 directory-entry ceiling) before they can allocate any membership or retained string (`crates/wanix-plumb/src/path.rs:19,45`).

## `#plumb/<topic>/send` — publish

`send` is strictly write-only; the open path rejects any read open (`crates/wanix-plumb/src/files.rs:16-21`). One write is one envelope. The write parses its bytes as a single JSON object, re-serializes the envelope to a newline-terminated line, and hands that line to the backing port (`crates/wanix-plumb/src/files.rs:50-60`):

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'
# in another terminal, against a services-enabled serve:
echo '{"kind":"task.done","from":"nodeA","body":{"path":"/world/out"}}' \
  > '#plumb/build/send'
```

The envelope is the unit of coordination (`crates/wanix-plumb/src/envelope.rs:28-42`):

- **`kind`** (required) — the typed routing tag, e.g. `task.done`, the primary dispatch key.
- **`from`** / **`to`** (optional, default empty) — addresses: a node id, an agent id, or empty for an anonymous/topic-wide broadcast.
- **`body`** (optional, default null) — a free-form arbitrary JSON value.

Two guards keep `send` honest. The envelope is `#[serde(deny_unknown_fields)]`, so a typo like `{"kind":"x","oops":1}` surfaces as an error rather than being silently dropped (`crates/wanix-plumb/src/envelope.rs:29,118-120`). And an envelope larger than `MAX_ENVELOPE_LEN = 64 * 1024` (64 KiB) is rejected before it reaches the port, so a hostile peer cannot force an unbounded allocation through the bus (`crates/wanix-plumb/src/envelope.rs:20,63-69`). A write reports its whole length consumed: a partial publish has no meaning on a best-effort bus (`crates/wanix-plumb/src/files.rs:57-59`).

## `#plumb/<topic>/recv` — subscribe

`recv` is strictly read-only — the open rejects write, create, and truncate (`crates/wanix-plumb/src/files.rs:24-29`). Opening it subscribes to the topic and returns a blocking stream of newline-JSON envelopes. The stream follows the same blocking-read-until-data-or-EOF discipline the pipe and agent event streams use (see [blocking stream / EOF contract](/concepts/blocking-stream-eof-contract)): a `read` blocks on a condvar with a 50 ms re-check, a push always wakes it, and `Ok(0)` returns only once the subscription is permanently closed (`crates/wanix-plumb/src/buffer.rs:90-113`).

The key word is *since it opened*. A subscription only sees messages broadcast after it joined — publish-then-subscribe means the subscriber misses the earlier message entirely (`crates/wanix-plumb/src/local.rs:131-141`). This is intentional: it is the [best-effort epidemic delivery](/concepts/best-effort-epidemic-delivery) contract, not a durable queue. There is no replay and no acknowledgement.

A flooded subscriber is protected, not slowed: the per-subscription `LineBuffer` is capped at `MAX_BUFFERED_BYTES = 16 * MAX_ENVELOPE_LEN` (a useful backlog of full-size envelopes). A remote node that knows a topic name could push an arbitrary number of individually-bounded messages into an idle subscriber's buffer; once the ceiling is reached, the oldest buffered bytes are dropped to make room (`crates/wanix-plumb/src/buffer.rs:30,62-77`). That matches the lossy semantics gossip already has for a lagged reader.

## Two ports: local and gossip

The transport is injected as a `PlumbPort`, which keeps the device crate synchronous and iroh-free (`crates/wanix-plumb/src/port.rs`). Two implementations exist:

- **`LocalPlumbPort`** — in-process, single-node. A `publish` fans out to every live subscriber on the same topic in the same process; it has no network and no durability (`crates/wanix-plumb/src/local.rs:55-91`). It backs the device's tests and any single-node deployment. A publish to a topic with no live subscriber is a no-op on the wire, never an error, and the topic is evicted once its last subscriber drops, so caller-controlled names cannot pin the map forever.
- **`GossipPlumbPort`** — the `wanix-mesh` backend. Each topic name maps to an iroh-gossip `TopicId`, so a message broadcast on one node is received on another; the gossip ALPN rides the same identity-bound endpoint as the 9P control plane and the blob data plane (`crates/wanix-mesh/src/plumb.rs:1-62`). The first `send`/`recv` on a topic joins its gossip swarm; an idle topic is swept and its membership reclaimed.

Swapping the port is the only difference between single-node and cross-machine coordination. The device, the envelope shape, and the file contract are identical — which is what lets two agents on two machines hold one conversation over a topic ([send the agent to the data](/concepts/send-agent-to-the-data)).

## See also

- [Best-effort epidemic delivery](/concepts/best-effort-epidemic-delivery) — why a late subscriber misses earlier messages, and why that is the design.
- [Blocking stream / EOF contract](/concepts/blocking-stream-eof-contract) — the shared `recv` read discipline across `#plumb`, `#pipe`, and `#agent`.
- [The single-frame serve caveat](/concepts/single-frame-serve-caveat) — why a live `recv` cannot interleave with a write on one serve connection.
- [Send the agent to the data](/concepts/send-agent-to-the-data) — `#plumb` as the coordination plane between agents and nodes.
- [Service devices](/concepts/service-devices) — the plain-`FileSystem` device pattern `#plumb` follows.
- [The device index](/devices/index) — the full service-device set.

## Status / honest limits

`#plumb` ships on this branch as a service device under `serve --wanix-services`, both single-node (`LocalPlumbPort`) and across the mesh (`GossipPlumbPort`). Three boundaries are load-bearing:

- **Best-effort, not durable.** A subscriber that was not listening when a message was published never sees it, and there is no acknowledgement (`crates/wanix-plumb/src/envelope.rs:6-10`). Durable handoff belongs in [`#kv`](/devices/kv) or a content-addressed capsule blob, not on the bus.
- **Live `recv` deadlocks on a single serve connection.** `serve` handles one 9P frame at a time per connection, so a blocking `recv` read holds the connection and no later frame is processed. The cockpit self-check therefore probes the *publish* path only — it writes one envelope to `#plumb/<topic>/send` and confirms acceptance, leaving end-to-end delivery to the mesh path that uses a separate connection (`workbench/src/web/cockpit-self-check.ts:368-398`). Live pub/sub through `serve` needs a second 9P connection or concurrent frame handling.
- **A flooded subscriber loses the oldest data.** The `recv` buffer is capped at `16 * MAX_ENVELOPE_LEN` and drops oldest bytes under flood (`crates/wanix-plumb/src/buffer.rs:30`). This bounds memory against a hostile or idle reader; it is not lossless transport.
