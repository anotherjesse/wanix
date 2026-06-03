#!/usr/bin/env sh
set -eu

if ! command -v cargo-deny >/dev/null 2>&1; then
    echo "cargo-deny is required; install it with:" >&2
    echo "  cargo install cargo-deny --version 0.19.8 --locked" >&2
    exit 1
fi

cargo deny --locked --all-features check advisories sources bans
