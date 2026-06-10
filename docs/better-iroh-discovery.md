# Better Iroh Discovery Plan

Status: planning note, not an accepted ADR.

## Problem

Wanix currently prints mesh tickets like:

```text
iroh://f671...?addr=127.0.0.1:57343
```

This is useful as an explicit, deterministic direct-route hint: the peer id names
who must answer, and `addr=` says one place to try dialing it. But it is a poor
default user model for a multi-node Wanix mesh:

- it makes local demos look like they depend on hardcoded host/port strings;
- it makes multi-volume output noisy because every resource prints a full socket
  address;
- it hides iroh's actual discovery story behind direct-address handoff plumbing;
- it encourages users to think the address is the resource identity, when the
  peer id is the identity and the address is only one way to find it;
- it creates an apparent second way to name resources. As volumes are added,
  removed, restarted, or rebound on new ports, any durable `addr=` string becomes
  stale routing data. The canonical resource name should not change just because
  a process got a new socket.

The low-level direct address should stay available for tests and explicit
diagnostics. It should not be the primary happy-path UX, and it should not be
stored as the durable identity of a resource.

## Desired model

Wanix should treat the iroh endpoint id as the low-level resource name, and iroh
discovery as the address resolution layer:

```text
resource name -> iroh://PEER -> discovered direct/relay route -> native FileSystem mount
```

The canonical low-level address should be:

```text
iroh://PEER
```

or, once the catalog exists:

```text
notes
photos
macbook/notes
```

The direct-address form remains a valid fallback:

```text
iroh://PEER?addr=127.0.0.1:57343
```

but it should be understood as "try this route for `PEER`", not as a separate
resource name. If that socket is stale, or belongs to a different Wanix instance,
the dial must fail because the remote endpoint cannot authenticate as `PEER`.
The bad outcome should be "route failed", never "mounted the wrong resource".

Canonical rule:

```text
iroh://PEER          = resource address
addr=HOST:PORT       = ephemeral route hint for that peer
iroh://PEER?addr=... = resource address plus one non-canonical route hint
```

## Iroh discovery options

Iroh separates identity from reachability:

- Endpoint id / peer id: the authenticated identity of the remote endpoint.
- Discovery: resolves an endpoint id into direct addresses and/or relay URLs.
- Ticket direct addresses: bundled route hints that can shortcut discovery but
  do not define identity.

Relevant discovery modes:

- DNS/Pkarr discovery: endpoint address information is published under the
  endpoint key and resolved through iroh's discovery services.
- mDNS/local discovery: endpoints announce on the local network and discover each
  other via LAN broadcast/multicast style discovery.
- Direct address hints: the ticket itself carries `addr=HOST:PORT` as a route
  hint for the peer id.

The current Wanix demo path is direct address hints. The better mesh UX should
prefer discovery where available, with direct address hints only as a fallback or
debug mode.

Reference docs:

- <https://docs.iroh.computer/concepts/discovery>
- <https://docs.rs/iroh/latest/iroh/discovery/mdns/index.html>

## Proposed Wanix modes

### 1. Direct-route mode

Purpose: deterministic tests and explicit diagnostics.

Example:

```sh
wanix volume serve --all --addr 127.0.0.1:0
```

Output:

```text
notes   iroh://PEER_A?addr=127.0.0.1:57343
photos  iroh://PEER_B?addr=127.0.0.1:53447
```

Keep this mode because it is easy to reason about and avoids discovery
dependencies in tests. Do not treat the printed `addr=` as a durable resource
address; it is a current route hint for `PEER_A` or `PEER_B`.

### 2. Discovery mode

Purpose: normal multi-node UX.

Example shape:

```sh
wanix volume serve --all --discover
```

or, if discovery is the default:

```sh
wanix volume serve --all
```

Output:

```text
notes   iroh://PEER_A
photos  iroh://PEER_B
```

The server enables iroh discovery and publishes/announces each per-volume
endpoint. The client dials by endpoint id and lets iroh resolve the current
address/relay path.

This is the preferred low-level mode. If a volume server restarts and `notes`
gets a new UDP port, its resource address remains `iroh://PEER_A`; discovery
updates the route.

### 3. Catalog mode

Purpose: humane names and composition.

Example shape:

```sh
wanix mount notes=/vol/notes photos=/vol/photos
```

The catalog entry can store:

```text
name = notes
kind = iroh
endpoint = PEER_A
discovery = local | dns | pkarr | auto
route_hints = optional transient direct hints
```

The catalog should not require users to paste direct socket addresses unless they
are intentionally using direct-route diagnostics. Durable catalog identity is the
endpoint id, not `addr=`.

## Implementation plan

### Phase 0: Make `iroh://PEER` the canonical low-level address

Do not remove `?addr=...`.

Instead, make docs and output call it a "direct-route hint" or "direct handoff",
not "the iroh address". The key distinction:

```text
iroh://PEER          = canonical resource address
iroh://PEER?addr=... = canonical resource address plus an explicit route hint
```

Acceptance:

- CLI help and ADR language stop implying `addr=` is intrinsic to mesh identity.
- Tests continue to use direct tickets for deterministic loopback.
- Catalog-related docs never store `addr=` as the durable resource id.

### Phase 1: Add an iroh local discovery option — SHIPPED (always-on mDNS)

**Shipped:** every mesh endpoint enables mDNS local-network address lookup
(`iroh-mdns-address-lookup`, advertise + resolve) in
`wanix-mesh::MeshNode::build_endpoint`. It is **always on, not a flag** — a bare
`iroh://PEER` dials on the LAN/same machine with no `addr=`, and a peer that
restarts on a new port is rediscovered by its stable id. We deliberately did not
add a `--discovery local` flag; an opt-out can be added later only if an
environment without multicast (some CI) needs it. Proof:
`crates/wanix-cli/tests/mesh_iroh.rs::mdns_discovers_a_bare_peer_id_without_a_direct_addr`.
Direct `addr=` still works as a shortcut/fallback, and identity is unchanged: a
wrong hint fails the dial, never mounts another peer.

> Historical (pre-Phase-1 snapshot, superseded by the **Shipped** note above):
> at investigation time, mDNS/LAN discovery was not in the workspace and a bare
> `iroh://PEER` was unresolvable on the `--addr`/loopback path. Neither is true
> anymore — `iroh-mdns-address-lookup` is a `wanix-mesh` dependency, enabled
> unconditionally in `MeshNode::build_endpoint`, and the candidate
> `--discovery local` flag sketched here was deliberately not added.

Questions to settle in code:

- Is local discovery a feature already enabled in the workspace's `iroh`
  dependency?
- Does `wanix-mesh::MeshNode` need a builder/config object instead of growing
  boolean arguments?
- Should discovery be configured on both server and dialer nodes?
- How do we expose discovery failures cleanly in CLI errors?

Acceptance:

- A local discovery integration test can dial `iroh://PEER` without `addr=`.
- A separate direct-route test keeps `?addr=` so failures are diagnosable, but
  the discovery test treats `iroh://PEER` as the address under test.
- A stale/wrong direct-route test proves `iroh://PEER?addr=wrong` fails rather
  than mounting a different endpoint.
- No change to the native FileSystem-over-iroh wire.

### Phase 2: Add DNS/Pkarr discovery mode

Add a non-local discovery option for real multi-network use.

Candidate CLI:

```sh
wanix volume serve --all --discovery public
wanix qjs-shell --mesh-discovery public --mount-mesh iroh://PEER=/vol/notes
```

This should use iroh's existing discovery services instead of inventing a Wanix
address server.

Acceptance:

- A served resource can be dialed as `iroh://PEER` without a direct `addr=`.
- The operator can tell whether the endpoint is discoverable before the ticket is
  printed, or gets a clear warning that discovery may still be propagating.
- Public exposure still requires the same explicit safety posture as today:
  ungranted read/write export must require `--insecure-open` or a future auth
  model.

### Phase 3: Make catalog entries discovery-aware, not route-bound

Once `#catalog` exists, store resource identity separately from route hints.

Example entry:

```json
{
  "name": "notes",
  "kind": "iroh",
  "endpoint": "PEER_A",
  "discovery": ["local", "pkarr"],
  "route_hints": ["127.0.0.1:57343"]
}
```

The catalog resolver tries:

1. local discovery, if enabled;
2. DNS/Pkarr discovery, if enabled;
3. relay path, if discovered;
4. direct route hints, if present and allowed by policy.

The exact order can change, but the identity cannot: every route must
authenticate as `endpoint`.

Acceptance:

- Users compose by names, not socket strings.
- Recipes and catalog entries remain portable across network changes.
- Direct `addr=` strings are optional route hints, not the durable resource name.
- Restarting a volume server can change ports without invalidating catalog
  identity.

## Design constraints

- Keep "one ticket = one resource root" for now.
- Do not use 9P `aname` or native subresource selectors to solve discovery.
- Do not introduce a Wanix-specific discovery server before trying iroh's built
  in discovery mechanisms.
- Keep direct-route tickets for deterministic tests and explicit diagnostics.
- Treat `iroh://PEER` as the canonical low-level resource address. `addr=` is a
  route hint only.
- Discovery must not weaken identity: the dial must still authenticate the peer
  id even if the address came from mDNS, DNS/Pkarr, a catalog, or a direct ticket.
- Public discovery must not imply public authorization. Finding a resource and
  being allowed to mutate it are separate concerns.

## Open questions

1. Should local discovery be enabled by default for `volume serve`, or only with
   an explicit flag until the behavior is well understood?
2. Should `volume serve --all` print both forms during the transition?

   ```text
   notes   iroh://PEER_A   direct=iroh://PEER_A?addr=127.0.0.1:57343
   ```

   If it does, the first column should still be the canonical address; direct
   routes should be labeled as hints.

3. How should we name discovery policy in catalog entries: `auto`, `local`,
   `public`, `direct-hints`, or a list?
4. How do we surface "endpoint id known, but discovery found no current route" in
   shell errors?
5. Does one endpoint per volume create too much discovery traffic if a host has
   many volumes? If yes, revisit scoped subresources per ADR 0007's criteria.

## Recommendation

Keep direct-route tickets as a low-level escape hatch, but make the next mesh UX
slice explicitly discovery-backed:

1. wire iroh local discovery into `wanix-mesh::MeshNode`;
2. prove `iroh://PEER` works on a LAN/loopback test without `addr=`;
3. prove stale/wrong `addr=` hints fail identity verification rather than
   mounting the wrong resource;
4. then let `#catalog` store endpoint ids and discovery policy instead of direct
   socket strings.

This keeps the current implementation reliable while moving the human model away
from hardcoded addresses and toward `iroh://PEER` as the one low-level resource
name.
