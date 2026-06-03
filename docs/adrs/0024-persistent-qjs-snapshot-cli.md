# ADR 0024: Persistent QuickJS Snapshot CLI

## Status

Accepted

## Context

`qjs-restore` demonstrates QuickJS VM snapshot and restore inside one CLI
invocation. That proves the VM-versus-host-resource boundary, but it does not
give Wanix a native demo where snapshot bytes persist outside the process and
are resumed later with freshly attached task resources.

Wanix snapshot bytes are QuickJS WebAssembly VM images. They do not include
task identity, namespace bindings, cwd, argv, env, fd table entries, stdio
attachments, or host mounts. Those resources are Wanix host state and must be
reattached explicitly each time the VM image is restored.

## Decision

Add two native CLI commands:

- `wanix-rust qjs-snapshot --snapshot FILE <script.js>` runs JavaScript as a
  qjs Wanix task, writes the captured QuickJS VM image to `FILE`, and reports
  task output through ordinary Wanix stdio fds.
- `wanix-rust qjs-resume --snapshot FILE <script.js>` reads the VM image from
  `FILE`, restores it into a fresh qjs Wanix task, reattaches cwd, argv, env,
  stdio, namespace, and explicit host mounts from the new CLI invocation, then
  evaluates the supplied resume script.

Both commands accept the same `--env`, `--cwd`, `--mount`, script path, and
script argument shape used by the ordinary qjs demo. Open dynamic Wanix task
fds and live WASI fds continue to block snapshot creation.

## Consequences

The CLI now has an externally visible persistence demo for QuickJS task VM
state outside Chrome and outside a single native process invocation.

Snapshot files remain VM images, not Wanix process checkpoints. Resume creates
or selects new Wanix host resources and attaches them to the restored runtime,
so demos can intentionally show preserved JavaScript globals alongside fresh
Wanix env, namespace, stdio, and host mount state.

Future richer snapshot policies can serialize selected virtual fd state or task
metadata, but the default boundary stays explicit: QuickJS VM memory is stored
in the snapshot file, while Wanix host resources are rebuilt by Wanix policy.
