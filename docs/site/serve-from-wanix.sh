#!/usr/bin/env bash
# Generate the Wanix docs site with an *in-Wanix* wasm task, then serve it from
# Wanix — the whole site is a thing Wanix produces and serves about itself.
#
#   docs/site/serve-from-wanix.sh [WORKDIR]      # WORKDIR defaults to /tmp/wanix-site-demo
#   PORT=8181 docs/site/serve-from-wanix.sh      # override the HTTP port
#
# Nothing here is configured with per-site CLI flags. `--root DIR` is the
# endorsed "expose this host directory as a namespace" operation; serving is
# just exposing that namespace over HTTP (Phase 0: the static route reads
# through the FileSystem trait, not std::fs).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$ROOT/target/release/wanix-rust"
WORK="${1:-/tmp/wanix-site-demo}"
PORT="${PORT:-8181}"

echo "==> building wanix-rust"
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p wanix-cli --bin wanix-rust

echo "==> staging corpus + SSG wasm guest into $WORK"
rm -rf "$WORK"
mkdir -p "$WORK"
cp -R "$ROOT/docs/site/content" "$WORK/content"
cp "$ROOT/crates/wanix-site-gen/fixtures/site-gen.wasm" "$WORK/site-gen.wasm"

echo "==> generating the site IN WANIX (wasm32-wasi task; cwd is its namespace)"
( cd "$WORK" && "$BIN" wasm site-gen.wasm content build )

echo "==> serving the generated site from Wanix at http://127.0.0.1:$PORT/"
echo "    (Ctrl-C to stop)"
exec "$BIN" serve --root "$WORK/build" --wanix-services --listen "127.0.0.1:$PORT"
