# ADR 0038: QuickJS WASI Symlink Stdlib Fixture

## Status

Accepted

## Context

ADR 0036 and ADR 0037 moved symlink metadata, readlink, and creation into
Wanix-owned live WASI providers, but the bundled QuickJS fixture still hid the
matching `qjs:os` APIs under QuickJS-NG's non-WASI libc guards. That left the
substrate proven by WAT and adapter tests, but not by ordinary JavaScript
running as a Wanix `qjs` task.

## Decision

Rebuild the checked-in `wanix-qjs-engine` QuickJS fixture so `qjs:os.lstat`,
`qjs:os.readlink`, and `qjs:os.symlink` are exported under WASI.

The fixture source change is deliberately narrow: split the QuickJS-NG libc
preprocessor block so those three link operations compile for WASI, while
process APIs such as `exec`, `waitpid`, `pipe`, `kill`, `dup`, and `dup2`
remain excluded. The resulting fixture imports `path_readlink` and
`path_symlink` in addition to the existing libc Preview 1 imports.

Wanix task, fd, namespace, rights, and host-mount policy remain outside the
engine fixture. The engine only decodes guest memory and forwards Preview 1
calls to an attached live WASI provider.

## Consequences

JavaScript running through `qjs:os` can now call `lstat`, `readlink`, and
`symlink` and reach live Wanix-backed WASI semantics. The proof spans the
engine fixture, `wanix-qjs` task driver, and native CLI host-mount demo.

Changing the fixture changes the module SHA-256 used by snapshot identity
validation. Older snapshots made with the prior fixture are expected to be
rejected by the exact-build snapshot contract.

This does not add engine-owned mutable virtual symlinks, Windows symlink
creation, process APIs, broader `path_open` symlink policy, or snapshot
serialization for open symlink-related fd state.
