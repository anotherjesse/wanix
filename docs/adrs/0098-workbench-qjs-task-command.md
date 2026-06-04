# ADR 0098: Workbench qjs Task Command

## Status

Accepted.

## Context

`serve --wanix-services` now exports `#task` and `#term` over direct 9P and
registers a real `qjs` task driver. The Rust-served workbench could browse and
edit that namespace, and it could open the dedicated qjs-shell WebSocket route,
but it did not yet expose a VS Code workflow that starts the active JavaScript
file as a Wanix task through `#task`.

## Decision

Add a `Wanix: Run Current JavaScript as qjs Task` workbench command. The command
requires an active `wanix:` editor, saves the file, allocates `#term/new` and
`#task/new/qjs` through the direct 9P filesystem handle, writes the task `cmd`
and `dir` service files, binds `#term/<id>/program` to fd 0/1/2 with `ctl bind`,
starts the task with `ctl start`, and opens the resulting VS Code
pseudoterminal.

The generated workbench launcher passes a `qjsTask` capability flag when
discovery advertises `qjs` in `services.drivers`. The extension treats QuickJS as
a Wanix task driver capability, not as a separate browser or VS Code process
model.

## Consequences

The served workbench can now visibly run JavaScript outside Chrome's runtime
foundation while preserving Wanix ownership of task identity, namespace, cwd,
fds, stdio, and exit state. The dedicated qjs-shell WebSocket route remains the
interactive shell path, while this command proves the direct `#task` API from
the VS Code surface.
