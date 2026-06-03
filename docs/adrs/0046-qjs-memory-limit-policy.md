# ADR 0046: QuickJS Memory Limit Policy

## Status

Accepted

## Context

ADR 0045 added an interrupt-poll budget so Wanix `qjs` tasks can stop
CPU-bound JavaScript. Allocation-heavy JavaScript is a separate host policy
problem: a task can consume QuickJS heap memory even when it is not stuck in an
infinite CPU loop.

The workspace QuickJS engine crate already exposes
`QuickJsRuntime::set_memory_limit(bytes)`. That setting is runtime host policy
applied to the QuickJS VM; it is not Wanix filesystem state, task identity, or a
QuickJS-owned process model. Wanix composition needs a way to apply that policy
when starting a `qjs` task outside Chrome.

## Decision

Expose an optional QuickJS heap memory limit on the Wanix `qjs` task driver.
When configured, the runner applies `QuickJsRuntime::set_memory_limit(bytes)`
immediately after creating the QuickJS runtime and before evaluating the task
script.

`wanix-rust qjs` exposes this policy as `--memory-limit-bytes N`. ADR 0047 later
extends the same explicit policy to `qjs-snapshot` and `qjs-resume` as
restore-time host reattachment. The default is no QuickJS heap limit, preserving
existing behavior. Allocation failures surface as normal QuickJS task failures:
the driver records exit status `1`, preserves stdout/stderr emitted before the
failure, and the CLI reports native exit status `1`.

The memory limit remains host policy. Restore paths that need the same policy
must reapply it when creating or restoring a runtime, just as other host
resources and limits are reattached outside the VM snapshot image.

## Consequences

Native Wanix demos can now run allocation-heavy JavaScript under an explicit
heap bound, for example:

```sh
wanix-rust qjs --memory-limit-bytes 1048576 examples/qjs-memory-limit-demo.js
```

This complements the interrupt-poll budget for CPU-bound code, but it does not
add Wasmtime fuel, linear-memory quotas, OS resource limits, snapshot-embedded
policy, or a general task resource-management framework.
