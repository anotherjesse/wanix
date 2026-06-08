# Option C transport consolidation — review

The implementing workflow (`wf_e1597173-82a`) stalled (an agent made no progress)
*after* committing the substantive work but *before* its own Review stage ran.
This is the manual review that replaces it, plus the one fix it surfaced.

## What landed (commits since `a4f94a2`)

- `ee0e280` Core+WS — one session core; `WebSocketDuplex` adapter; ws session
  duplication removed.
- `d5df2b4` Listener+Fold — `serve --p9` raw door; `p9-listen` + `p9-ws`
  retired (logic moved to `serve/raw9p`); trust guard; discovery `tcp` route;
  ADR 0006; cockpit `p9.ts`; doc updates (cli-command-index, serve-and-discovery,
  rust-walkthrough, AGENTS.md).
- `<this review's commits>` — trust-boundary tests (the stalled, uncommitted
  Trust-stage tests) and the fail-closed ordering fix below.

## Verdict by invariant

- **One 9P session implementation — HOLDS.** `P9Server::serve_duplex<D: Read +
  Write>` is the canonical loop (`wanix-9p/src/transport.rs:136`); `serve_stream`
  delegates to it via `DuplexPair` (`:131`). All transports are adapters over it:
  `p9-stdio` (`p9_stdio.rs:70` → `serve_stream`), the websocket door
  (`serve/ws_duplex.rs` `WebSocketDuplex` → `serve_duplex`; proven by
  `websocket_duplex_serves_a_session_over_the_core` and the split-frame
  reassembly test), and the raw `--p9` door (`serve/raw9p.rs` → the shared
  listener). No residual duplicated frame loop or parallel error enum remains.
- **Retirement is clean.** No live references to `p9-listen`/`p9-ws` in
  `collected.rs`/`help.rs`; `lib.rs:616-619` asserts the help text no longer
  advertises them. Their reusable logic (grant/policy plumbing, accept loop) was
  *moved*, not duplicated, into `serve/raw9p`.
- **Discovery + cockpit consistent.** `routes.p9.tcp` is additive, emitted only
  when `--p9` bound a door (`discovery.rs:160`); the cockpit reads the optional
  `tcp?` field (`workbench/src/wanix/p9.ts:30`). (Workbench TS is outside
  `just check`.)
- **ADR 0006** records the durable contract (one session core; ws/raw/stdio/iroh
  as transports; `--wanix-services` loopback-only; subcommands retired).

## Bug found and fixed: trust-guard TOCTOU (fail-closed ordering)

`start_raw9p_door` bound the listener **and spawned the accept thread**, and the
caller enforced `enforce_services_trust_boundary` *afterwards*. A non-loopback
`serve --p9 ADDR --wanix-services` therefore opened an accepting RCE door for the
window between spawn and the refusal. Fixed by splitting bind from accept
(`raw9p::bind_raw9p_door` → enforce boundary → `raw9p::spawn_raw9p_accept`), so
the door **fails closed before it can accept a single connection**. The HTTP/ws
door was already checked before its accept loop; both doors are now
check-before-serve.

## Trust boundary — final state

`--wanix-services` binds the `#task`/`#agent` exec devices (RCE). It is refused
when *either* 9P door (HTTP/ws or raw `--p9`) is bound non-loopback
(`enforce_services_trust_boundary`, mirroring `mesh/serve.rs`). Tests cover the
HTTP-door and `--p9`-door refusals and the loopback-allowed companions. The raw
door may be capability-gated with `--peer`/`--grant` (`P9Server::with_policy`).
Note: over plain TCP, `--peer` is asserted not cryptographically proven (only
iroh QUIC proves identity) — loopback remains the real local-trust control.

## Outstanding follow-up (not a blocker)

Demos/recipes/corpus that referenced `p9-listen`/`p9-ws` must be reworked to the
`serve --p9` surface and re-rendered through the SSG (tracked separately):
`docs/recipes/01,02` (+ corpus mirrors), `rust-walkthrough.md`,
`reference/cli-command-index.md`, the 9P-contract concept pages, the mesh docs,
and upgrading `docs/site/serve-from-wanix.sh` to demonstrate a live `#sites`
write via `mount-write tcp://`.
