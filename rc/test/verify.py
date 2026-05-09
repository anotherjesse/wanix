#!/usr/bin/env python3
"""verify.py — TDD harness for the wanix rc shell.

Runs each test case through `wanix-cli` (headless Chrome) and compares stdout
against an expected value. Designed for incremental TDD: add failing tests,
fix them, repeat at greater fuzz intensity.

Usage:
    rc/test/verify.py                    # run all cases
    rc/test/verify.py -k echo            # only cases matching "echo" substring
    rc/test/verify.py --fuzz 50          # run 50 randomly-generated cases
    rc/test/verify.py -v                 # verbose: show command + raw output
    rc/test/verify.py --skip-build       # don't rebuild wanix-cli first

Each case spawns one wanix-cli (~5s of overhead). Cases run sequentially so
output is deterministic; a runner-level parallel mode would race the shared
HTTP port allocator. Keep the case list focused.

Conventions:
- The shell prompt `rc% ` is stripped from output before comparison.
- The trailing `\\r` from the in-browser terminal is normalized away.
- Each input is auto-suffixed with `\\nexit\\n` so the task exits cleanly.
"""

from __future__ import annotations

import argparse
import os
import random
import re
import shutil
import string
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Optional

REPO = Path(__file__).resolve().parents[2]
WANIX_CLI = REPO / ".local" / "bin" / "wanix-cli"

# Each case: (name, input_to_shell, expected_output_after_normalization)
# `expected` may be a string (exact match) or a callable(actual: str) -> Optional[str]
# returning None on success or an error message on failure.
@dataclass
class Case:
    name: str
    input: str
    expected: object  # str | Callable[[str], Optional[str]]
    timeout: float = 30.0

# Output normalization: strip prompts and CR.
PROMPT_RE = re.compile(r"rc% ?")

def normalize(raw: str) -> str:
    s = raw.replace("\r\n", "\n").replace("\r", "\n")
    s = PROMPT_RE.sub("", s)
    return s

def equals(expected: str) -> Callable[[str], Optional[str]]:
    def check(actual: str) -> Optional[str]:
        if actual == expected:
            return None
        return f"expected {expected!r}\n  got      {actual!r}"
    return check

def contains(needle: str) -> Callable[[str], Optional[str]]:
    def check(actual: str) -> Optional[str]:
        if needle in actual:
            return None
        return f"expected to contain {needle!r}\n  got     {actual!r}"
    return check

def regex(pattern: str) -> Callable[[str], Optional[str]]:
    rx = re.compile(pattern, re.DOTALL)
    def check(actual: str) -> Optional[str]:
        if rx.search(actual):
            return None
        return f"expected match {pattern!r}\n  got            {actual!r}"
    return check

# ---- Test cases ------------------------------------------------------------

# Start small. As bugs are fixed, more cases are added (and made stricter).
CASES: list[Case] = [
    Case(
        name="bridge.smoke",
        input="pwd\n",
        # In repl-rc the working dir is "/web".
        expected=equals("/web\n"),
    ),
    Case(
        name="echo.single",
        input="echo X\n",
        expected=equals("X\n"),
    ),
    Case(
        name="echo.multi_args",
        input="echo a b c\n",
        expected=equals("a b c\n"),
    ),
    Case(
        name="echo.two_calls",
        input="echo a\necho b\n",
        expected=equals("a\nb\n"),
    ),
    Case(
        name="echo.quoted_spaces",
        input='echo "abc def"\n',
        expected=equals("abc def\n"),
    ),
    Case(
        name="printf.basic",
        input='printf "%s %s\\n" foo bar\n',
        expected=equals("foo bar\n"),
    ),
    Case(
        name="for.loop",
        input='for x in 1 2 3; do echo $x; done\n',
        expected=equals("1\n2\n3\n"),
    ),
    Case(
        name="exit.code_zero",
        input="true\n",
        # Default `exit\n` suffix exits 0.
        expected=lambda s: None,
    ),
    # Variable expansion
    Case(
        name="var.assign_and_echo",
        input='x=hello\necho $x\n',
        expected=equals("hello\n"),
    ),
    Case(
        name="var.with_braces",
        input='x=world\necho ${x}!\n',
        expected=equals("world!\n"),
    ),
    # Pipes
    Case(
        name="pipe.echo_to_cat",
        input='echo hi | cat\n',
        expected=equals("hi\n"),
    ),
    # Redirection: capture echo output to a file under /tmp and read it back.
    Case(
        name="redirect.write_and_read",
        input='echo persisted > /tmp/verify.txt\ncat /tmp/verify.txt\n',
        expected=equals("persisted\n"),
    ),
    # Builtins
    Case(
        name="builtin.true",
        input='true && echo ok\n',
        expected=equals("ok\n"),
    ),
    Case(
        name="builtin.false",
        input='false || echo recovered\n',
        expected=equals("recovered\n"),
    ),
    # Command substitution
    Case(
        name="cmdsub.dollar_paren",
        input='echo "got: $(echo nested)"\n',
        expected=equals("got: nested\n"),
    ),
    # Arithmetic
    Case(
        name="arith.add",
        input='echo $((2 + 3))\n',
        expected=equals("5\n"),
    ),
    # Quoting / escapes
    Case(
        name="quote.single",
        input="echo 'a $x b'\n",
        expected=equals("a $x b\n"),
    ),
    Case(
        name="quote.escaped_dollar",
        input='echo "a \\$x b"\n',
        expected=equals("a $x b\n"),
    ),
    # cd / pwd. /web is the working dir; /web/dom exists (web platform fs).
    Case(
        name="cd.then_pwd",
        input='cd /web/dom\npwd\n',
        expected=equals("/web/dom\n"),
    ),
]

# ---- Runner ----------------------------------------------------------------

def build_cli() -> None:
    print("[verify] building wanix-cli...")
    proc = subprocess.run(
        ["make", "wanix-cli"],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout + "\n" + proc.stderr + "\n")
        sys.exit(f"[verify] make wanix-cli failed (exit {proc.returncode})")
    if not WANIX_CLI.exists():
        sys.exit(f"[verify] wanix-cli not found at {WANIX_CLI}")

def run_case(case: Case, verbose: bool) -> tuple[bool, str, str]:
    """Run a single case through wanix-cli. Returns (passed, normalized_output, message)."""
    stdin_payload = (case.input + "exit\n").encode("utf-8")
    started = time.time()
    try:
        proc = subprocess.run(
            [str(WANIX_CLI)],
            input=stdin_payload,
            capture_output=True,
            timeout=case.timeout,
            cwd=REPO,
        )
    except subprocess.TimeoutExpired:
        return False, "", f"timed out after {case.timeout}s"
    elapsed = time.time() - started

    # Output may contain non-utf8 bytes from stray ANSI/control sequences;
    # decode permissively so the harness can still report a useful diff.
    raw = proc.stdout.decode("utf-8", errors="replace")
    actual = normalize(raw)

    if verbose:
        sys.stderr.write(
            f"\n[verify] === {case.name} === ({elapsed:.1f}s, exit {proc.returncode})\n"
            f"  stdin: {case.input!r}\n"
            f"  raw:   {raw!r}\n"
            f"  norm:  {actual!r}\n"
        )

    check = case.expected
    if isinstance(check, str):
        check = equals(check)
    err = check(actual)  # type: ignore[misc]
    if err is None:
        return True, actual, ""
    return False, actual, err

# ---- Fuzzing ---------------------------------------------------------------

def fuzz_cases(n: int, seed: Optional[int]) -> Iterable[Case]:
    rng = random.Random(seed)
    alphabet = string.ascii_letters + string.digits + " _"
    for i in range(n):
        # Random echo lines: simple words that should round-trip exactly.
        nlines = rng.randint(1, 4)
        lines = []
        expected_lines = []
        for _ in range(nlines):
            nwords = rng.randint(1, 5)
            words = []
            for _ in range(nwords):
                wlen = rng.randint(1, 6)
                w = "".join(rng.choice(alphabet[:62]) for _ in range(wlen))  # no spaces in words
                words.append(w)
            line = "echo " + " ".join(words)
            expected_lines.append(" ".join(words))
            lines.append(line)
        inp = "\n".join(lines) + "\n"
        exp = "\n".join(expected_lines) + "\n"
        yield Case(name=f"fuzz.{i:03d}", input=inp, expected=equals(exp))

# ---- Main ------------------------------------------------------------------

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("-k", "--filter", help="only run cases whose name contains this substring")
    p.add_argument("--fuzz", type=int, default=0, help="generate N random echo-roundtrip cases")
    p.add_argument("--seed", type=int, default=None, help="fuzz RNG seed (default: time-based)")
    p.add_argument("-v", "--verbose", action="store_true")
    p.add_argument("--skip-build", action="store_true", help="don't rebuild wanix-cli")
    p.add_argument("--stop-on-fail", action="store_true")
    args = p.parse_args()

    if not args.skip_build:
        build_cli()
    elif not WANIX_CLI.exists():
        sys.exit(f"[verify] {WANIX_CLI} missing; run without --skip-build")

    cases: list[Case] = list(CASES)
    if args.fuzz:
        cases.extend(fuzz_cases(args.fuzz, args.seed))
    if args.filter:
        cases = [c for c in cases if args.filter in c.name]
    if not cases:
        sys.exit("[verify] no cases selected")

    print(f"[verify] running {len(cases)} cases against {WANIX_CLI}")
    fails: list[tuple[Case, str]] = []
    for c in cases:
        sys.stdout.write(f"  {c.name:<24} ... ")
        sys.stdout.flush()
        ok, _actual, msg = run_case(c, args.verbose)
        if ok:
            sys.stdout.write("PASS\n")
        else:
            sys.stdout.write("FAIL\n")
            sys.stdout.write(f"    {msg}\n")
            fails.append((c, msg))
            if args.stop_on_fail:
                break

    print()
    if fails:
        print(f"[verify] {len(fails)}/{len(cases)} FAILED:")
        for c, m in fails:
            print(f"  - {c.name}: {m.splitlines()[0]}")
        return 1
    print(f"[verify] {len(cases)} passed")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
