set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

fmt:
    @cargo fmt --package wanix-9p --package wanix-cli --package wanix-fs --package wanix-protocol --package wanix-qjs --package wanix-qjs-engine --package wanix-task --package wanix-term --package wanix-vfs --package wanix-wasi --check

module-lines:
    @bash tools/check-module-lines.sh

clippy:
    @cargo clippy --workspace --all-targets -- -D warnings

test:
    @cargo test --workspace --locked

check: fmt module-lines clippy test
