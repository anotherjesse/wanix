# Adding an installable Wanix command

Commands are ordinary `wasm32-wasip1` programs the shell resolves from a `bin`
directory: type `foo`, the shell runs `bin/foo.wasm` as a `#task`. This directory
holds one checked-in `.wasm` per command plus the source crate that built it.

To add a command `foo`:

1. **Create the crate** at `fixtures/commands/foo/` as its *own* cargo workspace
   (so the parent Wanix workspace ignores it), mirroring `jaq/`:

   ```toml
   # fixtures/commands/foo/Cargo.toml
   [package]
   name = "foo"
   version = "0.1.0"
   edition = "2021"
   [[bin]]
   name = "foo"
   path = "src/main.rs"
   [profile.release]
   opt-level = "s"
   strip = true
   [workspace]
   ```

   Write `src/main.rs` as a normal command: read `std::env::args()`, read
   `std::io::stdin()`, write `std::io::stdout()`. The shell wires fd 0/1/2 to the
   pipe/terminal, so plain stdio "just works." Add `.gitignore` with `/target`.

2. **Build and drop the artifact** (or run `just rebuild-commands`):

   ```sh
   cd fixtures/commands/foo
   cargo build --release --target wasm32-wasip1
   cp target/wasm32-wasip1/release/foo.wasm ../foo.wasm
   ```

3. **Register it** — add one line to `COMMANDS` in
   `crates/wanix-wasm/src/commands.rs`:

   ```rust
   ("foo", include_bytes!("../fixtures/commands/foo.wasm")),
   ```

That's it. `command_bin()` now includes `foo`, and any task whose namespace binds
`command_bin()` at `bin` can run `foo` (and pipe it: `echo ... | foo`).

Note: checked-in `.wasm` artifacts add to repo size (`jaq.wasm` is ~1.5 MB from
its JSON/stdlib deps). If the command set grows large, consider git-lfs for
`fixtures/commands/*.wasm`.
