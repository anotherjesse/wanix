set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

fmt:
    @cargo fmt --package wanix-9p --package wanix-9p-client --package wanix-agent --package wanix-cas --package wanix-cli --package wanix-cpu --package wanix-fs --package wanix-id --package wanix-job --package wanix-kv --package wanix-mesh --package wanix-mesh-wire --package wanix-module-cache --package wanix-pipe --package wanix-plumb --package wanix-protocol --package wanix-qjs --package wanix-qjs-engine --package wanix-sh --package wanix-site-cas --package wanix-site-fs --package wanix-site-gen --package wanix-sites --package wanix-task --package wanix-term --package wanix-tool --package wanix-vfs --package wanix-wasi --package wanix-wasi-host --package wanix-wasm --check

module-lines:
    @bash tools/check-module-lines.sh

clippy:
    @cargo clippy --workspace --all-targets -- -D warnings

test:
    @cargo test --workspace --locked

check: fmt module-lines clippy test

quality-coverage:
    @mkdir -p target/cargo-crap
    @cargo llvm-cov --all-targets --lcov --output-path target/cargo-crap/lcov.info

quality-crap: quality-coverage
    @cargo crap --workspace \
        --exclude 'examples/**' \
        --exclude '**/tests.rs' \
        --exclude 'src/tests/**' \
        --exclude 'src/*_tests.rs' \
        --exclude 'src/*_tests/**' \
        --exclude 'src/runtime_cleanup_tests/**' \
        --exclude 'src/snapshot_tests/**' \
        --exclude 'src/module_tests/**' \
        --lcov target/cargo-crap/lcov.info \
        --summary

quality-rustqual: quality-coverage
    @rustqual --format text --no-fail --coverage target/cargo-crap/lcov.info crates

quality: quality-crap quality-rustqual
