# ADR 0068: Serve Discovery And Route Policy

## Status

Accepted

## Context

`wanix-rust serve` is the local composition surface for browser, v86,
workbench, VS Code, and local tool clients. It needs to serve static assets and
protocol routes from one listener without making the 9P server or core runtime
own HTTP, WebSocket, browser isolation, or demo-page policy.

The durable decision is the discovery and route contract, not every generated
page or smoke workflow that proved a route.

## Decision

Rust `serve` combines static HTTP and direct protocol routes on one local
listener:

- `wanix-rust serve [DIR] [--listen HOST:PORT] [--once] [--bundle NAME]
  [--wanix-services]` serves a selected root, defaulting to the current
  directory and a demo-friendly local address.
- Static HTTP responses use the browser headers needed by local v86 and
  workbench demos.
- `/.well-known/export9p` is the reserved direct binary 9P WebSocket route.
- `/.well-known/wanix.json` advertises discovery data: direct 9P routes,
  selected bundles, service availability, direct-v86 boot hints, and explicitly
  unimplemented routes such as Ethernet/vnet until those contracts exist.
- `--once` remains a deterministic single-connection mode for tests and
  scripted smokes; normal serve accepts concurrent HTTP and 9P WebSocket
  clients.
- `--wanix-services` exports a Wanix namespace containing `#task` and `#term`
  through direct 9P from a service-root task context.
- In service mode, the task table registers at least `noop` and `qjs`, so
  direct 9P clients can allocate a QuickJS task, set `cmd`/`env`/`dir`, bind
  fds, and start it through `#task`.
- In service mode, `/.well-known/qjs-shell` exposes a terminal/session
  WebSocket route for browser/workbench pseudoterminals to drive a
  terminal-backed QuickJS task.

Generated bundle pages and browser smokes should consume the discovery document
rather than hard-coding route assumptions.

## Consequences

Serve is the visible local entrypoint for the bigger pieces without turning into
a VM manager, editor host, network bridge, or core filesystem crate. Browser,
v86, and workbench clients can share route discovery and direct 9P semantics.

Future auth, remote exposure, Ethernet/vnet, multiplexing, or persistent
session policy should be recorded as new decisions because they change the
serve trust boundary.

## Replaces

This ADR consolidates ADRs 0069 through 0071, ADR 0086, and ADRs 0095 through
0097 into the Rust serve route and discovery contract.
