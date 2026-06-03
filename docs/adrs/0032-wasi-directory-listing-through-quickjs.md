# ADR 0032: WASI Directory Listing Through QuickJS

## Status

Accepted

## Context

QuickJS-NG exposes `qjs:os.readdir(...)` through WASI Preview 1 directory
operations. The guest opens the target path with `O_DIRECTORY` and a nonblocking
fd flag, then reads directory entries from the resulting fd. When the live
provider is backed by a real Wanix `WasiCtx`, the root preopen advertises full
directory inheriting rights, so libc requests a broad directory rights mask for
the opened directory.

Wanix previously parsed those raw Preview 1 requests but rejected them when
creating the directory handle because the handle validation only accepted the
narrower `DIRECTORY_BASE` set. Synthetic engine tests could prove the live hook
shape with a smaller test preopen, but Wanix qjs tasks still saw `ENOTCAPABLE`
from `os.readdir(".")`.

## Decision

Preserve live WASI providers as runtime host state attached through
`QuickJsCreateOptions` and `QuickJsRestoreOptions`, not through
`QuickJsHostConfig`.

Teach `wanix-wasi` raw Preview 1 `path_open` handling to preserve
directory-open metadata. `O_DIRECTORY` requires the resolved path to be a
directory, accepts QuickJS libc's nonblocking fd flag for synchronous Wanix
namespace operations, rejects directory create/truncate/append combinations, and
allows libc's broader directory rights when those rights are still within the
parent's inheriting rights. Direct Wanix file-open options remain narrower
because generic file open options do not express directory handles.

`wanix-qjs-engine` stays Wanix-agnostic: it copies Preview 1 directory-open,
`fd_readdir`, and path-stat data between guest memory and the live
`QuickJsWasiHost`. `wanix-qjs` adapts those calls to `wanix_wasi::WasiCtx` so
Wanix owns namespace, hidden-entry filtering, fd rights, stdio, and task
semantics.

## Consequences

JavaScript running as a Wanix `qjs` task can call `qjs:os.readdir(...)` and see
directory entries from its Wanix namespace while output still flows through the
task fd table.

The native CLI now has a `qjs-readdir-demo.js` example proving directory create,
file write, directory listing, and stdio through the outside-Chrome runtime.

This does not add async directory iteration, host-directory demos, richer fd
readiness, or snapshot serialization for open directory fds. Live WASI fd state
remains host state that must be closed or reattached by Wanix policy.
