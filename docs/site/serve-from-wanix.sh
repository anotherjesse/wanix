#!/usr/bin/env bash
# Generate the Wanix docs site with an *in-Wanix* wasm task, serve it from Wanix,
# and register it as a site by *writing a file* into the running serve over 9P —
# the whole site is a thing Wanix produces, serves, and is configured about,
# entirely through files.
#
#   docs/site/serve-from-wanix.sh [WORKDIR]      # WORKDIR defaults to /tmp/wanix-site-demo
#   PORT=8282 P9_PORT=9999 docs/site/serve-from-wanix.sh
#
# Nothing here is a per-site CLI flag. `--root DIR` exposes a (disposable, empty)
# host directory; `--p9 ADDR` opens a loopback raw-9P door; and the site is
# registered by writing a `dir <path>` descriptor to `#sites/blog.localhost`
# with `mount-write` — a live filesystem operation against the running server
# (ADR 0006: one 9P session core, raw-9P is a serve mode, --wanix-services is
# loopback-only).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$ROOT/target/release/wanix"
WORK="${1:-/tmp/wanix-site-demo}"
PORT="${PORT:-8282}"
P9_PORT="${P9_PORT:-9999}"

echo "==> building wanix"
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p wanix-cli --bin wanix

echo "==> staging corpus + SSG wasm guest into $WORK"
rm -rf "$WORK"
mkdir -p "$WORK/empty-root"
cp -R "$ROOT/docs/site/content" "$WORK/content"
cp "$ROOT/crates/wanix-site-gen/fixtures/site-gen.wasm" "$WORK/site-gen.wasm"

echo "==> generating the site IN WANIX (wasm32-wasi task; cwd is its namespace)"
( cd "$WORK" && "$BIN" wasm site-gen.wasm content build )

echo "==> starting serve on an EMPTY root + a loopback raw-9P door"
# Empty --root on purpose: the site must come from the live #sites write below,
# not from --root, which proves the file-driven registration.
"$BIN" serve --root "$WORK/empty-root" --wanix-services \
    --listen "127.0.0.1:$PORT" --p9 "127.0.0.1:$P9_PORT" &
SERVE_PID=$!
trap 'kill "$SERVE_PID" 2>/dev/null || true' EXIT

echo "==> waiting for the HTTP door"
until curl -s -o /dev/null "http://127.0.0.1:$PORT/" 2>/dev/null; do
    kill -0 "$SERVE_PID" 2>/dev/null || { echo "serve exited early"; exit 1; }
    sleep 0.2
done

echo "==> registering the site by WRITING #sites over raw 9P (no CLI flag)"
"$BIN" mount-write "tcp://127.0.0.1:$P9_PORT" \
    '#sites/blog.localhost' "dir $WORK/build"

echo
echo "    Site is live. In a browser (*.localhost needs no DNS):"
echo "        http://blog.localhost:$PORT/"
echo "    The default host (empty --root) 404s; the site exists only because of"
echo "    the #sites file write. Ctrl-C to stop."
echo
wait "$SERVE_PID"
