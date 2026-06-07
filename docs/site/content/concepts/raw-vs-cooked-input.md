---
title: Raw vs Cooked Input & Control Bytes
slug: concepts/raw-vs-cooked-input
pageType: concept
oneLiner: In raw mode the guest shell owns echo, backspace, Ctrl-C line cancel, and Ctrl-D exit; clients forward 0x03/0x04 as terminal input, never as task cancellation.
audience: [developer]
tags: [terminal, shipped, caveat, cli]
sourceRefs:
  - examples/qjs-term-shell-demo.js:12
  - examples/qjs-term-shell-demo.js:871-913
  - examples/qjs-term-shell-demo.js:915-927
  - examples/qjs-term-shell-demo.js:946-959
  - docs/adrs/0003-terminal-device-and-shell-lifecycle.md:50-57
seeAlso: [concepts/qjs-shell, devices/term, concepts/resize-winch-lifecycle, concepts/guest-js-guardrails]
prerequisites: [concepts/qjs-shell]
usedInFlows: []
honestLimits:
  - "Control bytes (0x03, 0x04) are terminal input, not kernel signals; there is no task cancellation, signal delivery, or process-group contract behind them."
  - "Ctrl-C only cancels the line buffer in the shell; it does not interrupt a running child qjs task, which the shell launches synchronously."
  - "Ctrl-D exits only when the line buffer is empty; mid-line it is ignored, matching the demo shell, not a full POSIX tty discipline."
canonicalCaveatFor: []
---

# Raw vs Cooked Input & Control Bytes

In raw mode the guest shell owns echo, backspace, Ctrl-C line cancel, and Ctrl-D exit; clients forward 0x03/0x04 as terminal input, never as task cancellation.

A terminal session is a stream of bytes flowing two directions through `#term/<id>`. The question this page answers is: who decides what a backspace *means*, what `Ctrl-C` *does*, and where the line breaks fall? In a Wanix shell session that owner is the guest program, not the host and not the browser. The host moves bytes; the guest gives them meaning. Get this boundary right and a browser terminal, a native CLI, and a future VM console all behave the same way, because none of them invents its own editing or signal semantics.

## Show it: two modes, one shell

The bundled QuickJS shell (`examples/qjs-term-shell-demo.js`) reads one environment variable at startup to pick its input discipline:

```js
const rawInput = std.getenv("WANIX_QJS_SHELL_RAW") === "1";
```

That single flag (`examples/qjs-term-shell-demo.js:12`) selects between two read paths off fd 0. Both paths see the *same* bytes from `#term/<id>/program`; they differ only in how the guest processes them.

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

# Cooked: the line arrives all at once, on Enter.
wanix-rust qjs-shell

# Raw: the host feeds each keystroke; the guest echoes and edits.
wanix-rust qjs-shell --raw
```

## Cooked mode: lines arrive whole

In cooked mode the read handler accumulates bytes and only acts on complete lines (`examples/qjs-term-shell-demo.js:915-927`):

```js
function handleLineInput(bytes, count) {
  pending += stringFromBytes(bytes, count);
  let newline;
  while ((newline = pending.indexOf("\n")) >= 0) {
    const line = pending.slice(0, newline);
    // ... dispatch the command, keep the remainder buffered ...
  }
}
```

The shell never echoes individual characters and never interprets a `0x7f` as backspace — it just waits for `\n` and splits. Whatever produced those bytes (a host tty in canonical mode, a paste, a script) already did the editing. This is the easy mode for non-interactive drivers: write a line plus newline, read the response.

## Raw mode: the guest owns editing and control bytes

Set `--raw` (or `WANIX_QJS_SHELL_RAW=1`) and the shell handles input byte-by-byte (`examples/qjs-term-shell-demo.js:871-913`). Now *every* terminal convention is the guest's job:

```js
function handleRawByte(byte) {
  if (byte === 0x03) {                 // Ctrl-C
    pending = "";
    std.out.puts("^C\n");
    prompt();
    return;
  }
  if (byte === 0x04 && pending.length === 0) {  // Ctrl-D on empty line
    runCommand("exit");
    return;
  }
  if (byte === 0x08 || byte === 0x7f) {          // backspace / delete
    if (pending.length > 0) {
      pending = pending.slice(0, -1);
      std.out.puts("\x08 \x08");                 // erase one cell
    }
    return;
  }
  if (byte === 0x09 || byte >= 0x20) {           // printable / tab
    const char = String.fromCharCode(byte);
    pending += char;
    std.out.puts(char);                          // echo
  }
}
```

Read that list against what a Unix tty would do for you and notice it is all *here*, in guest JavaScript: echo (the `std.out.puts(char)`), destructive backspace (`\x08 \x08`), `Ctrl-C` clearing the pending line and reprinting the prompt, `Ctrl-D` exiting only when the line is empty, carriage-return-or-newline ending a line (`examples/qjs-term-shell-demo.js:901`). The host did none of it. The host's only job is the read handler on fd 0 that pulls bytes off the terminal's program side and hands them to whichever discipline is active (`examples/qjs-term-shell-demo.js:946-959`).

## Name it: raw vs cooked, and who forwards control bytes

This is the classic terminal **raw vs cooked** distinction, placed deliberately on the guest side of the `#term` device. ADR 0003 states the rule: "the host feeds bytes through the terminal device and the guest-side shell owns echo, simple editing, newline handling, Ctrl-C line cancellation, Ctrl-D exit, and command dispatch" (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:50-57`).

The corollary is the contract for *clients*. A browser, editor, or VM terminal may translate a platform key event — the user pressing `Ctrl-C` — into the byte `0x03` and write it to `#term/<id>/data`. That is correct. What it must *not* do is interpret `Ctrl-C` itself and try to cancel a Wanix task. The ADR is explicit: "those translations are terminal input, not Wanix task cancellation or signal delivery." Control bytes are payload on the wire to the guest, nothing more. This keeps every terminal frontend thin and identical: they forward bytes, the guest decides.

## See also

- [The qjs-shell](/concepts/qjs-shell) — the guest shell program these two input modes drive, and its built-in command set.
- [The #term device](/devices/term) — the `new` / `ctl` / `data` / `program` / `winch` files that carry these bytes.
- [Resize & the winch lifecycle](/concepts/resize-winch-lifecycle) — the other half of terminal state, delivered through `#term/<id>/winch`.
- [Guest JS guardrails](/concepts/guest-js-guardrails) — why shell behavior uses `qjs:std`, `qjs:os`, and service files rather than host hooks.

## Status / honest limits

- **Control bytes are not signals.** `0x03` and `0x04` are bytes the guest reads from the terminal. There is no kernel signal, no `SIGINT`, no process-group, and no cancellation model behind them (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:50-57`). A frontend that forwards them is correct; a frontend that uses them to kill a task is inventing semantics Wanix does not have.
- **Ctrl-C cancels a line, not a job.** In raw mode `0x03` clears the shell's `pending` line buffer and reprints the prompt (`examples/qjs-term-shell-demo.js:871-877`). It does not interrupt a child task — the shell launches `qjs` children synchronously, so there is nothing concurrent for it to interrupt.
- **Ctrl-D is empty-line only.** `0x04` triggers exit *only* when the line buffer is empty (`examples/qjs-term-shell-demo.js:878`); mid-line it is ignored. This matches the demo shell's discipline, which is a small, explicit subset of a POSIX tty, not a complete line editor.
- **Cooked mode does no editing at all.** It splits on `\n` and nothing else (`examples/qjs-term-shell-demo.js:915-927`); any echo or backspace handling must come from whatever sits upstream of the terminal.
