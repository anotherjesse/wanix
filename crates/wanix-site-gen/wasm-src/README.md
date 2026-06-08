# `ssg` — the Wanix SSG wasm guest

This is a standalone Cargo workspace (note the empty `[workspace]` table in
`Cargo.toml`) so the parent Wanix workspace never tries to host-build it. It
depends on the parent `wanix-site-gen` crate by path and exposes a `_start`
entry that drives `generate_site` over `std::fs` (which, on `wasm32-wasip1`,
lowers to WASI calls against the task's preopened namespace).

## Build & vendor the artifact

```sh
rustup target add wasm32-wasip1            # once
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/ssg.wasm ../fixtures/site-gen.wasm
```

The built artifact (`../fixtures/site-gen.wasm`) is checked in so the host build
and `cargo test --workspace --locked` never need the wasm toolchain. Only
`wasm-src/target/` is gitignored; `wasm-src/Cargo.lock` is committed.
