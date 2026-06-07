---
title: Best-Effort Epidemic Delivery (not a queue)
slug: concepts/best-effort-epidemic-delivery
pageType: concept
oneLiner: "#plumb has no durability or ack: a late subscriber misses earlier messages; durable handoff belongs in #kv or a capsule."
audience: [developer]
tags: [mesh, plumb, shipped, caveat]
sourceRefs:
  - crates/wanix-plumb/src/envelope.rs:1-10
  - crates/wanix-plumb/src/local.rs:55-93
  - crates/wanix-plumb/src/local.rs:139-147
  - crates/wanix-mesh/src/plumb.rs:1-20
  - docs/mesh-the-missing-half-of-9p.md:2229-2234
seeAlso:
  - devices/plumb
  - devices/kv
  - concepts/kv-smallest-database
  - concepts/wanix-capsule
  - concepts/send-agent-to-the-data
  - concepts/single-frame-serve-caveat
prerequisites:
  - devices/plumb
usedInFlows:
  - {flow: learn/wire-a-mesh, step: 5}
honestLimits:
  - "No durability: a subscriber that was not listening when a message was broadcast never sees it."
  - "No acknowledgement: a publisher cannot tell whether anyone received the message."
  - "Lossy under flood and on a freshly formed swarm: the first broadcast can drop before membership settles."
  - "A blocking recv cannot interleave with a write on the same serve connection (one 9P frame at a time)."
canonicalCaveatFor: [plumb-no-durability]
---

# Best-Effort Epidemic Delivery (not a queue)

`#plumb` has no durability or ack: a late subscriber misses earlier messages, and durable handoff belongs in `#kv` or a capsule.

It is tempting to reach for `#plumb` as a message queue — post work on one side, drain it on the other, trust nothing is lost. That is the wrong model. `#plumb` is Plan 9's plumber made real on the mesh: a typed, broker-less, best-effort bus. Messages fan out to whoever is *currently* listening on a topic and are gone the moment they are delivered. Treating it as a queue will silently lose your handoff. This page names the contract precisely so you put coordination on `#plumb` and durable state somewhere it actually lives.

## Show it: a late subscriber misses the message

Two writes and a read tell the whole story. Post to a topic, then subscribe, then post again:

```sh
# topic "build": publish before anyone is listening...
echo '{"kind":"early"}' > '#plumb/build/send'

# ...now a subscriber opens recv...
# (cat blocks, draining envelopes received since this open)
cat '#plumb/build/recv' &

# ...and a second publish arrives.
echo '{"kind":"late"}' > '#plumb/build/send'
```

The `recv` reader sees `{"kind":"late"}` and never `{"kind":"early"}`. The reference backend proves exactly this: `a_late_subscriber_misses_earlier_messages` publishes `early`, subscribes, publishes `late`, and asserts the subscription only sees what was broadcast after it joined (`crates/wanix-plumb/src/local.rs:139-147`). A publish to a topic with no live subscribers is not an error — it is dropped on the floor (`crates/wanix-plumb/src/local.rs:55-61`). That is the **epidemic** part: a message spreads to the membership that exists at broadcast time, and to no one else.

## The shape: typed, addressed, broker-less

Each `#plumb` message is one JSON object on its own line — the **envelope** (`crates/wanix-plumb/src/envelope.rs:1-10`):

```json
{"kind":"task.done","from":"nodeA","to":"nodeB","body":{"out":"/world/out/rootfs.img"}}
```

The fields mirror Plan 9's plumber rules. `kind` is the typed routing tag and the primary dispatch key (`task.done`, `build.start`, a turn event). `from` and `to` are optional addresses — a node id, an agent id, or empty for a topic-wide broadcast. `body` is a free-form JSON payload. Unknown fields are rejected, so a typo in a `send` surfaces as an error instead of being silently dropped (`crates/wanix-plumb/src/envelope.rs:28-42`).

This is the right shape for agent coordination. Agents and tools do not want a central broker to register with; they want to announce "I finished the build, the output is here" and have any interested peer pick it up by `kind`. The bus is **broker-less**: no server owns the topic, no one mediates routing, the topic name *is* the rendezvous. On the mesh, that name maps straight onto an iroh-gossip topic — `GossipPlumbPort` derives a `TopicId` from `blake3(topic)`, `send` broadcasts on it, and `recv` drains what the topic receives, so a `task.done` posted to `#plumb/build` on one node is read from `#plumb/build/recv` on another (`crates/wanix-mesh/src/plumb.rs:1-11`).

## What it is NOT

State the boundaries flatly, because each one is a way to lose data if you assume otherwise:

- **No durability.** There is no log, no backlog, no replay. A subscription only ever sees messages broadcast after it joined. The local port holds only weak references to live subscriber buffers and evicts a topic the moment its last subscriber drops (`crates/wanix-plumb/src/local.rs:62-93`) — nothing is retained.
- **No acknowledgement.** `publish` returns `Ok(())` whether one subscriber, a thousand, or zero received the line. A publisher cannot tell delivery from a drop.
- **Lossy under flood and during swarm formation.** On a freshly formed gossip swarm, the first broadcast can drop before membership settles. The two-machine handoff demo re-posts the nudge with `recv_with_retry` for exactly this reason — that is the honest behavior of epidemic delivery, not a bug to paper over (`docs/mesh-the-missing-half-of-9p.md:2229-2234`).

The plumber carries the *nudge*, not the *payload*.

## Where durable handoff belongs instead

Put the bytes somewhere with a defined lifetime, and use `#plumb` to point at them:

- For mutable shared state that two agents both touch, write the value to [`#kv`](/devices/kv) and broadcast a `task.done` whose `body` names the key. `#kv` is the smallest database in the system — see [#kv: the smallest database](/concepts/kv-smallest-database). (`#kv` is in-memory and lives only as long as the serve process.)
- For an immutable artifact you want to hand off whole — a built rootfs, a frozen world — write it through `#cas` or freeze a [capsule](/concepts/wanix-capsule), then plumb the content hash. The blob is verified end-to-end on the far side.

This is the demo pattern: B writes its build output into A's confined world over the imported 9P window, *then* posts the `task.done` envelope naming the `out` path. A reads `recv`, follows the path into its own world, and finds exactly the bytes B produced (`docs/mesh-the-missing-half-of-9p.md:2229-2234`). The durable thing moved over the file plane; `#plumb` only said "look here." See [send the agent to the data](/concepts/send-agent-to-the-data).

## See also

- [#plumb device](/devices/plumb) — the topic, `send`, and `recv` file contract.
- [#kv device](/devices/kv) and [#kv: the smallest database](/concepts/kv-smallest-database) — where durable mutable state lives.
- [Wanix capsule](/concepts/wanix-capsule) — freeze a world to persist an immutable artifact.
- [Send the agent to the data](/concepts/send-agent-to-the-data) — the coordination-vs-payload split in practice.
- [Single-frame serve caveat](/concepts/single-frame-serve-caveat) — why a blocking `recv` cannot interleave with a write on one connection.

## Status / honest limits

- **No durability, no ack.** A late subscriber misses earlier messages and a publisher never learns whether anyone received a message (`crates/wanix-plumb/src/local.rs:55-93`).
- **Lossy.** A freshly formed swarm can drop the first broadcast before membership settles; expect to retry the nudge (`docs/mesh-the-missing-half-of-9p.md:2229-2234`).
- **One frame at a time per serve connection.** A blocking `recv` cannot interleave with a write on the *same* 9P connection; live pub/sub over `serve` needs a second connection — see [the single-frame serve caveat](/concepts/single-frame-serve-caveat).
- **The topic name is the only boundary.** Gossip carries message bytes and grants no filesystem or exec capability, so it is safe on the public endpoint — but anyone who knows a topic name can read and write that bus (`crates/wanix-mesh/src/plumb.rs:13-20`).
