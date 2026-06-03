#!/usr/bin/env sh
set -eu

cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo doc --locked --no-deps --all-features
git diff --check
git diff --cached --check
