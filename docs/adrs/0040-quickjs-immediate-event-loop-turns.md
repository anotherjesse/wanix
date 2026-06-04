# ADR 0040: Bounded QuickJS Guest Execution Policy

## Status

Accepted

## Context

QuickJS has timers, promise jobs, fd readiness handlers, interrupt callbacks,
and heap limits. Wanix needs enough host policy to run useful JavaScript demos
without accidentally defining a general scheduler, signal system, cancellation
model, or serialized resource-policy format.

The policy belongs to the task creation/restoration or composition layer. It is
host lifecycle state, not guest VM memory and not deterministic fixture config.

## Decision

Expose bounded guest execution knobs as explicit Wanix host policy:

- synchronous WASI timer sleep is implemented with timer-only `poll_oneoff`;
- due async timers and promise jobs may run through bounded immediate event-loop
  turns after task evaluation;
- future timers may run only when the composition layer grants an explicit wait
  budget;
- `setReadHandler` and `setWriteHandler` callbacks may run for explicit
  nonblocking ready-fd turn budgets;
- self-clearing intervals run inside those bounded timer/event-loop pumps;
- CPU-bound guest code may be stopped by an explicit QuickJS interrupt-poll
  budget; and
- allocation-heavy guest code may be stopped by an explicit QuickJS heap memory
  limit.

CLI and serve surfaces can expose these knobs for deterministic demos and tests.
Create/restore paths must reattach them explicitly rather than serializing them
inside snapshots.

## Consequences

Wanix qjs tasks can run useful async JavaScript, ready-IO handlers, and bounded
resource tests while staying honest about what is not implemented yet. These
knobs are not a process scheduler, async runtime, signal delivery mechanism,
kill/cancel API, or durable task checkpoint format.

Future lifecycle work should either extend Wanix task semantics broadly or add
a new ADR for the changed scheduler/cancellation boundary.

## Replaces

This ADR consolidates ADRs 0031 and 0041 through 0046 into the bounded guest
execution policy. Snapshot reattachment for these knobs is covered by ADR 0003.
