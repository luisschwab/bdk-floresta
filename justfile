alias b := build
alias c := check
alias d := delete
alias e := example
alias f := fmt

_default:
    @just --list

# Build bdk_floresta
build:
    cargo build

# Check code: formatting, compilation, linting, and commit signature
check:
    cargo +nightly fmt --all -- --check
    cargo check --workspace
    cargo clippy --all-targets -- -D warnings
    @[ "$(git log --pretty='format:%G?' -1 HEAD)" = "N" ] && \
       echo "\n⚠️  Unsigned commit: bdk_floresta requires commits to be signed." || \
       true

# Delete files: bitcoin, signet, testnet4, target, lockfile
delete item="data":
    just _delete-{{ item }}

_delete-bitcoin:
    rm -rf data/bitcoin

_delete-signet:
    rm -rf data/signet

_delete-testnet4:
    rm -rf data/testnet4

_delete-target:
    rm -rf target

_delete-lockfile:
    rm -f Cargo.lock

# Run an example crate
example name="example":
    cargo run --example {{ name }} --release

# Format code
fmt:
    cargo +nightly fmt
