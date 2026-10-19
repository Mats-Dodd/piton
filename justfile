default:
    just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

check: fmt-check clippy
    cargo check --workspace --all-targets --all-features

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo nextest run --workspace --all-features --no-tests pass

test-ci:
    cargo nextest run --workspace --all-features --profile ci --no-tests pass

doctest:
    cargo test --workspace --doc

# The domain and core run anywhere, including wasm hosts such as Durable
# Objects; checking against a target without std proves they need no OS.
core-wasm:
    cargo check -p piton-domain -p piton-core --target wasm32-unknown-unknown

# Rebuilds the core with a deliberate bug and proves the checkers catch it.
# A separate target dir keeps the normal build cache intact.
planted-bug:
    RUSTFLAGS="--cfg piton_planted_bug" CARGO_TARGET_DIR=target/planted-bug cargo nextest run -p piton-model --profile ci

docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

deny:
    cargo deny check

unused-deps:
    cargo machete crates

supply-chain: deny unused-deps

ci: check test-ci doctest core-wasm planted-bug docs supply-chain
