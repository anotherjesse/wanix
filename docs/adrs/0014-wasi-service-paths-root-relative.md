# ADR 0014: WASI Service Paths Stay Namespace-Root Relative

## Status

Accepted

## Context

Wanix task namespaces expose service resources such as `#task/self/id`.
QuickJS tasks also run with a WASI root preopen whose source can be the task
working directory, so ordinary guest paths like `main.js` resolve relative to
the task cwd.

If WASI treated every relative path as relative to the preopen source, a guest
path like `#task/self/id` would become `app/#task/self/id` for a task running in
cwd `app`. That would diverge from existing Wanix task-service semantics and
force JavaScript to use a different spelling for task resources when it goes
through `qjs:std`.

## Decision

Wanix-backed WASI paths beginning with `#task` are normalized as Wanix task
service paths rooted at the task namespace root. This applies after normal
WASI path validation and before joining the path to the preopen source path.

Ordinary paths remain relative to the WASI directory fd/preopen source, so
`main.js` still resolves in the task cwd when fd 3 maps guest `/` to that cwd.
Other hash-prefixed names, such as ordinary hidden/service-like filesystem
entries outside `#task`, keep normal dirfd-relative behavior.

## Consequences

QuickJS stdlib calls such as `std.loadFile("#task/self/id")` can reach Wanix
task identity through the live WASI provider instead of the interim `Wanix`
host object.

Dynamic service files such as `#task/new/qjs` should be read through
`qjs:os.open`/`os.read` rather than `std.loadFile(...)`, because their metadata
size is not the same thing as the bytes produced by the first read. Service
field writes should likewise use `qjs:os.open` without create/truncate flags.

This preserves the existing Wanix meaning of `#task` across host APIs and WASI
imports. It also means Wanix task-service paths are intentionally outside the
cwd remapping applied to ordinary files.
