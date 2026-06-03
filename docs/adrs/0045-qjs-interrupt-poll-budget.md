# ADR 0045: QuickJS Interrupt Poll Budget

## Status

Accepted

## Context

Wanix `qjs` tasks run JavaScript synchronously through a QuickJS runtime hosted
by Wasmtime. ADRs 0040 through 0044 bounded post-eval event-loop work for ready
jobs, fd handlers, future timers, and self-clearing intervals, but a CPU-bound
script such as `while (true) {}` could still monopolize task startup unless the
embedding layer installed an engine interrupt handler itself.

The QuickJS engine crate already exposes interrupt handlers as runtime host
state. Wanix task execution also uses that interrupt path to stop evaluation
after a WASI `proc_exit` request. The next process-runtime slice needs a
composition policy that can stop runaway CPU-bound JavaScript without defining a
general scheduler, signal model, or asynchronous task cancellation protocol.

## Decision

Expose an optional interrupt poll budget on the Wanix `qjs` task driver. The
budget counts QuickJS interrupt-handler polls during script evaluation. When the
poll count exceeds the configured budget, the combined interrupt handler asks
QuickJS to interrupt the current script.

`wanix-rust qjs` exposes this policy as `--interrupt-after N`. The default is no
interrupt poll budget, preserving existing behavior. Exhausting the budget is a
task failure: the driver records exit status `1`, and the CLI reports the
QuickJS interruption error with native exit status `1`.

The interrupt handler remains host state reattached by Wanix composition. It is
not serialized inside QuickJS VM snapshots.

## Consequences

Native Wanix demos can now run CPU-bound JavaScript under an explicit bound, for
example `wanix-rust qjs --interrupt-after 1 examples/qjs-interrupt-demo.js`.
This makes runaway-script handling observable as Wanix task policy instead of a
QuickJS-owned process model.

The budget is based on QuickJS interrupt polls, not wall-clock time or
instruction fuel. ADR 0046 adds a separate QuickJS heap memory limit for
allocation-heavy JavaScript. Together they are bounded host policies for demos
and tests, but they do not add signals, asynchronous cancellation, preemptive
scheduling, or long-running task liveness management.
