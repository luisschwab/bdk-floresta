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

# Check code: formatting, compilation, linting, and commit signature
check:
    cargo +nightly fmt --all -- --check
    cargo check --workspace
    cargo clippy --all-targets -- -D warnings
    @[ "$(git log --pretty='format:%G?' -1 HEAD)" = "N" ] && \
       echo "\n⚠️  Unsigned commit: bdk_floresta requires commits to be signed." || \
       true

# Delete files: example, target, lockfile
delete item="example":
    just _delete-{{ item }}

# Run an example crate
example name="node":
    cargo run --example {{ name }} --release

# Format code
fmt:
    cargo +nightly fmt

_delete-example:
    rm -rf data/example-node

_delete-target:
    rm -rf target/

_delete-lockfile:
    rm -f Cargo.lock

