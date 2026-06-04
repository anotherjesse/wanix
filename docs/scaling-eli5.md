# How Wanix scales (plain-language)

Website-ready explainer. Numbers come from `performance.md` and are from one
18-core dev machine; they will vary by hardware.

---

## Rooms, not houses

**The old way — one house per program.** Normally, every program you run gets its
own house (an operating-system *process*). Houses are safe — nobody can walk into
yours — but they're expensive. Each one needs about **10 MB** of memory just to
exist, before it does any work. Want 1,000 little programs at once? That's
roughly **10 GB**, mostly spent on empty houses.

**The Wanix way — many locked rooms in one building.** Wanix runs each program in
a tiny sandbox called a *WebAssembly instance* — a **locked room inside a shared
building**. Each room is private: code in one room can't see or touch another,
and it can only use the doors (files, network) you explicitly hand it. But a room
costs about **0.25 MB** instead of 10 MB — roughly **40× smaller**. So the same
1,000 programs fit in **one building using a few hundred MB**, instead of 10 GB
of houses.

## Does it stay fast when the programs actually *do* something?

This is the real question — sleeping programs prove nothing. So we made them work.

**Running many at once scales with your cores, then holds steady.** We gave 1,000
programs a heavy math task at the same time on an 18-core machine. The work got
shared across all the cores, throughput climbed to match the hardware, and then
**stayed flat** — going from a handful to 1,000 programs didn't make each one fall
apart. Extra programs simply wait their turn in an orderly line; they don't drag
each other down. (One program finishes in ~1 second; 18 of them — one per core —
also finish in about the same time, together.)

**Files are fast, too.** A single program reads and writes files through Wanix at
**~500,000 operations per second**, and because every program has its own private
files, 50 of them together hit **~9 million operations per second**.

## The honest part

Wanix doesn't make the *language* fast. The little JavaScript engine in each room
is an interpreter, so heavy number-crunching runs **slower than native code** —
that's the trade for tiny, instant, secure rooms. Wanix's superpower isn't making
one program fast; it's running **thousands of isolated programs cheaply at the
same time**, scaling smoothly up to whatever your hardware can do.

## Why have more than one building?

Safety, chosen on purpose — not one building per user:

- **Blast radius** — if a building has a problem, only the people in *that*
  building are affected, not everyone.
- **Bad neighbors** — one program stuck in a loop or hogging memory is easier to
  contain.
- **Extra-sensitive tenants** — high-value or untrusted code can get its own
  building, or a fully fortified vault (a virtual machine).

**The recipe:** a *small* number of buildings (picked for safety, not user count)
→ each building uses all your cores → and inside each, thousands of cheap, private
rooms.

> **Old way:** ~10 MB per program, always.
> **Wanix:** ~0.25 MB per program + one small building cost — and *you* choose how
> many buildings based on how much isolation you need, not how many users show up.

---

### Note for the team (not for the public page)

The per-room *memory* isolation is real today. The hard CPU and memory *limits*
that make a room fully safe for untrusted code (Wasmtime epoch/fuel preemption and
linear-memory caps) are not wired up yet — see `performance.md`. Keep public copy
to "cheap, scalable isolation"; don't claim "safe for arbitrary untrusted code"
until those land.
