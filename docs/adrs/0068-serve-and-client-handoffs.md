# ADR 0068: Serve And Client Handoffs

## Status

Accepted

## Context

`wanix-rust serve` is the local composition surface for browser, v86,
workbench, VS Code, and local tool clients. It needs to serve static assets and
protocol routes from one listener without making the 9P server or core runtime
own HTTP, WebSocket, browser isolation, or demo-page policy.

Native QEMU is the corresponding non-browser VM handoff. It shares the same
guest-root and 9P-root assumptions as direct-v86 but remains an inspectable
command handoff rather than a VM manager.

## Decision

Rust `serve` combines static HTTP, discovery, and direct protocol routes on one
local listener:

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
  terminal-backed QuickJS task. The route accepts a Wanix `cwd` query value and
  resize control frames so editor terminals can start in the requested Wanix
  cwd and deliver dimensions through `#term/<id>/winch`.

Generated bundle pages and browser smokes should consume the discovery document
rather than hard-coding route assumptions.

Browser filesystem and workbench clients are frontend integrations over that
serve contract:

- `serve --bundle fs9p` exposes a browser filesystem smoke page that proves
  direct 9P browse/read/write behavior against the served root.
- The workbench extension can back its `wanix:` filesystem provider with direct
  9P operations, plus bounded client-side search over `wanix:/`.
- `serve --bundle workbench-fs9p` is a local generated VS Code web workbench
  launch path that points the extension at Rust serve discovery.
- When discovery advertises services, the workbench path can open qjs shell
  sessions and can start `qjs` Wanix tasks by driving `#task` and `#term` over
  direct 9P.
- Running the active `wanix:` JavaScript file as a `qjs` task should use the
  same task command/env/dir/fd service files as native clients.

Direct v86 is a browser/emulator handoff over Rust serve:

- `serve --bundle direct-v86` consumes discovery and wires the advertised
  direct 9P WebSocket into v86 `filesystem.proxy_url`;
- serve owns or advertises the v86 module, wasm, BIOS, and helper asset routes
  needed by the generated page;
- discovery reports boot asset readiness for kernel and optional initrd routes
  found in the served root;
- discovery exposes boot hints such as the default 9P-root Linux cmdline,
  memory size, VGA memory size, and virtio-console expectation;
- query overrides can supply kernel, initrd, cmdline, and autostart behavior
  for deterministic smokes; and
- the generated page exposes the guest `hvc0` virtio-console stream for visible
  browser boot and shell interaction.

Native QEMU is a validated command handoff:

- `wanix-rust rootfs --archive FILE.tgz --out DIR` extracts a guest root into a
  missing or empty directory, rejects unsafe archive paths, validates shared VM
  boot markers, and prints ready-to-run QEMU and direct-v86 commands.
- `wanix-rust qemu --root DIR` canonicalizes and validates the guest root,
  discovers `/boot/bzImage` or legacy `/bzImage` unless `--kernel PATH` is
  supplied, supports cmdline override, append options, mount-tag override, and
  a validated QEMU local 9P `security_model`, and prints a shell-quoted
  QEMU/KVM virtio-9p command by default.
- By default, the native QEMU command uses the same 9P-root guest shape where
  practical, with `host9p`, base `9p2000.L` root flags, `mapped-xattr` local 9P
  security, and `hvc0` virtconsole defaults.
- `wanix-rust qemu --exec` is an explicit foreground launch mode. It spawns the
  validated argv, inherits stdin/stdout/stderr, reports the child exit status,
  and does not turn Wanix into a background VM supervisor.

## Consequences

Rust Wanix has one local discovery and handoff story for browser filesystem,
workbench, v86, and native QEMU clients. These paths exercise the bigger pieces
without turning `serve` into a VM manager, editor host, network bridge, or core
filesystem crate.

Future auth, remote exposure, Ethernet/vnet, multiplexing, or persistent
session policy should be recorded as new decisions because they change the
serve/client trust boundary. Future QEMU work that adds daemon mode, persistent
VM management, network/vnet, terminal multiplexing, or rootfs build ownership
should also get a new decision.
