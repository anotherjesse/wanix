# ADR 0018: qjs-restore Uses Separate Before and After Process Context

## Status

Accepted.

## Context

The native `qjs-restore` demo runs two Wanix `qjs` tasks: a before task that
creates the QuickJS VM snapshot and an after task that restores that VM image.
The snapshot contains guest VM memory only. Wanix task metadata such as argv,
environment, cwd, stdio fds, namespace, and task identity is host state and is
reattached when the before or after task runtime is created.

Until now the CLI exposed cwd and mounts for restore demos, but before and after
argv/env were always empty. That made it harder to demonstrate the important
restore boundary: QuickJS libc state in the VM image can preserve before-task
process observations, while Wanix service files and task metadata are supplied
by the after task.

## Decision

Expose separate restore CLI options for the two task contexts:

- `--before-env KEY=VALUE`
- `--after-env KEY=VALUE`
- `--before-arg VALUE`
- `--after-arg VALUE`

The options configure the corresponding Wanix task before creating or restoring
the QuickJS runtime. They do not serialize argv/env into snapshot bytes.

`--cwd` and `--mount` remain shared because the current CLI demo restores into a
child task cloned from the before task namespace, then reattaches the same
explicit host mounts.

## Consequences

The CLI can now show both sides of the restore boundary: a restored QuickJS VM
can retain before-task VM/libc observations, while `scriptArgs`, qjs std/os
stdio, and `#task` service files are reattached from the after Wanix task.

This is still a demo workflow, not a persisted task snapshot format. A future
format needs explicit task metadata and policy for which process fields are
persisted, replaced, or rejected on restore.
