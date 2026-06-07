---
title: Resize / winch Lifecycle
slug: concepts/resize-winch-lifecycle
pageType: concept
oneLiner: Resize travels as "columns rows\n" through #term/<id>/winch; queued resizes are drained on read-handler turns; sessions release the terminal via ctl close on drop.
audience: [developer]
tags: [terminal, shipped, caveat, cli]
sourceRefs:
  - crates/wanix-term/src/files/winch.rs:43-107
  - crates/wanix-cli/src/qjs_term/pump.rs:146-164
  - crates/wanix-cli/src/qjs_term/session.rs:73-145
  - examples/qjs-term-shell-demo.js:929-959
  - crates/wanix-cli/src/serve/discovery.rs:173-188
seeAlso: [devices/term, concepts/qjs-shell, concepts/raw-vs-cooked-input]
prerequisites: [devices/term]
usedInFlows: []
honestLimits:
  - Resize is drained at the top of the same read-handler that services stdin; a true resize wakeup independent of stdin handling is a queued follow-up, not shipped.
  - winch is a fan-out byte channel, not a SIGWINCH delivery; the guest must open the winch file and poll its read end.
canonicalCaveatFor: []
---

# Resize / winch Lifecycle

Resize travels as `"columns rows\n"` through `#term/<id>/winch`; queued resizes are drained on read-handler turns; sessions release the terminal via ctl close on drop.

A terminal that can be resized has to tell its program the new dimensions. Wanix does not invent a signal or a syscall for this. It exposes a file. The host writes the new size into `#term/<id>/winch`, the queue fans out to every reader, and the guest shell drains that queue the next time it wakes to handle input. The whole resize story is open, write, read, and drop — the same four verbs every other Wanix capability uses.

## Show: the wire form is two numbers and a newline

When a session resizes, the host opens the winch file for writing and writes a single line. The payload builder is exactly this (`crates/wanix-cli/src/qjs_term/pump.rs:162`):

```rust
fn terminal_resize_payload(resize: &TermResize) -> Vec<u8> {
    format!("{} {}\n", resize.columns, resize.rows).into_bytes()
}
```

So a 132x43 resize is the four bytes-plus `132 43\n` written to `#term/<id>/winch` (`pump.rs:146-159`). There is no struct on the wire, no binary frame — columns, a space, rows, a newline. A guest reads that line and parses two integers. That is the entire format.

## winch fans out to subscribers; one writer reaches every reader

`WinchFile` is a normal `File`, but its read and write ends do different jobs. Opening for read registers a *subscriber*: the file allocates a fresh subscriber id and gives it an empty queue (`crates/wanix-term/src/files/winch.rs:23-34`). Opening for write makes the handle a *broadcaster*. A write copies the bytes into **every** subscriber's queue at once (`winch.rs:61-75`):

```rust
fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
    // ...
    for queue in winch.subscribers.values_mut() {
        queue.extend(buf);
    }
    Ok(buf.len())
}
```

A read drains only *that subscriber's* queue (`winch.rs:44-59`), and `read_ready` reports whether the subscriber's queue is non-empty (`winch.rs:81-96`) so a poller can tell there is a pending resize without blocking. When the read handle drops, its subscriber and queue are removed (`winch.rs:99-107`). The Plan 9 name for this shape is a *post-and-deliver* fan-out: one publisher, N independent reader queues, no shared cursor. Resize is multicast, not a single mailbox.

## The shell drains queued resizes before dispatching a command

The qjs shell opens its winch read end once at startup and keeps the fd (`examples/qjs-term-shell-demo.js:929`). It does not get interrupted on resize. Instead, the very first thing its stdin read handler does on every turn is drain the winch queue (`qjs-term-shell-demo.js:946-948`):

```js
os.setReadHandler(0, () => {
  drainWinch();
  // ...then read and dispatch terminal input
});
```

`drainWinch` reads the winch fd until it returns no more bytes, splits on newlines, and keeps the *last* line as the current size (`qjs-term-shell-demo.js:930-942`) — so a burst of resizes collapses to the latest dimensions, which is what a program actually wants. The shell exposes the result through its `size` builtin. This is why resize feels instant in practice but is, mechanically, *coalesced and read on the next input turn*: the resize is queued the moment the host writes winch, and consumed the moment the guest next wakes.

On the host side a `session.resize(columns, rows)` call writes the winch line, then runs ready-IO turns so the guest's read handler fires and drains it, then returns any output the resize produced (`crates/wanix-cli/src/qjs_term/session.rs:73-85`). One host call, one guest drain.

## Sessions release the owned terminal on drop

A `QjsShellSession` owns the `#term/<id>` resource it attached. When the session closes — explicitly via `close_terminal_resource`, or implicitly when the struct drops — it writes the close through the term device, tolerating an already-gone resource (`session.rs:120-145`):

```rust
impl Drop for QjsShellSession {
    fn drop(&mut self) {
        let _ = self.close_terminal_resource();
    }
}
```

`TermDevice::close` removes the resource from the device table and tears it down (`crates/wanix-term/src/lib.rs:96-105`). So the terminal — and with it every winch subscriber queue still hanging off that resource — is reclaimed when the session ends. No orphaned terminals, no leaked winch subscribers. This is the lifecycle discovery advertises to clients as `"terminalLifecycle":"owned-resource-closed-on-session-close"` (`crates/wanix-cli/src/serve/discovery.rs:182`).

## Discovery advertises the resize message forms

A browser or editor client does not have to guess the wire form. The served qjs-shell route publishes both accepted resize encodings in its discovery document (`discovery.rs:179-180`): the JSON control form `{"type":"resize","columns":COLS,"rows":ROWS}` and the bare `resize COLS ROWS` command, alongside the `exit` frame and the cwd query. The client picks a form; the session translates it to the `columns rows\n` winch line.

## See also

- [#term device](/devices/term) — the terminal device that owns `ctl`, `data`, `program`, and `winch`.
- [The qjs-shell](/concepts/qjs-shell) — the guest shell that drains winch on each input turn and owns the terminal resource.
- [Raw vs cooked input](/concepts/raw-vs-cooked-input) — how the same read handler that drains winch also handles control bytes.

## Status / honest limits

- **Resize is coupled to the stdin turn.** The guest drains winch at the top of its stdin read handler (`qjs-term-shell-demo.js:946-948`), so a resize is observed when the next input turn fires, not on an independent wakeup. A true resize wakeup independent of stdin handling is a queued follow-up, not shipped.
- **winch is a byte channel, not a signal.** There is no SIGWINCH delivery; the guest must open `#term/<id>/winch` for read and poll its read end. A program that never opens winch never learns it was resized.
- **Bursts coalesce.** `drainWinch` keeps only the last line in a batch (`qjs-term-shell-demo.js:937-941`); intermediate sizes between two drains are not individually observable. This is intentional, but it means winch reports the latest size, not a resize history.
