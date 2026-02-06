alias b := build
alias c := check
alias d := delete
alias e := example
alias f := fmt

_default:
    @just --list --list-heading $'bdk_floresta\n'

# Build `bdk_floresta`
build:
    cargo build

# Check code formatting, compilation, linting, and commit signature
check:
    cargo +nightly fmt --all -- --check
    cargo test --doc
    just check-features
    cargo clippy --all-targets -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
    @[ "$(git log --pretty='format:%G?' -1 HEAD)" = "N" ] && \
       echo "\n⚠️  Unsigned commit: bdk_floresta requires commits to be signed." || \
       true

# Check that all feature combinations compile
check-features:
    cargo check --no-default-features
    cargo check --no-default-features --features logger
    cargo check --workspace --all-targets

# Delete files: example, target, lockfile
delete item="examples":
    just _delete-{{ item }}

# Build documentation
doc:
    cargo test --doc
    cargo doc --no-deps --open

# Run an example crate
example name="node":
    cargo run --release --manifest-path examples/{{ name }}/Cargo.toml

# Format code
fmt:
    cargo +nightly fmt

_delete-examples:
    rm -rf examples/node/data
    rm -rf examples/block_wallet_sync/data

_delete-target:
    rm -rf target/

_delete-lockfile:
    rm -f Cargo.lock
