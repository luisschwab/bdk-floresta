alias b := build
alias c := check
alias d := delete
alias e := example
alias f := fmt

_default:
    @just --list --list-heading $'> bdk-floresta\n> A Floresta-powered chain-source crate for BDK\n\n> Available recipes:\n'

# Build `bdk-floresta`
build:
    cargo build

# Check code formatting, compilation, linting, and commit signature
check:
    cargo rbmt lint
    cargo rbmt docs
    just _check-signatures

# Check that all feature combinations compile
check-features:
    cargo rbmt test --toolchain stable --lock-file recent

# Delete files: example, target, lockfiles
delete item="example":
    just _delete-{{ item }}

# Generate documentation
doc:
    cargo rbmt docs
    cargo doc --no-deps

# Generate and open documentation
doc-open: doc
    cargo rbmt docs
    cargo doc --no-deps --open

# Run an example crate
example name="node":
    cargo run --release --manifest-path examples/{{ name }}/Cargo.toml

# Format code
fmt:
    cargo rbmt fmt

# Regenerate `Cargo-recent.lock` and `Cargo-minimal.lock`
lock:
  cargo +nightly rbmt lock

# Verify the library builds with MSRV (1.85.0)
msrv:
    cargo rbmt test --toolchain msrv --lock-file minimal

# Checks whether all commits in this branch are signed
_check-signatures:
    #!/usr/bin/env bash
    TOTAL=$(git log --pretty='format:%H' origin/master..HEAD | wc -l | tr -d ' ')
    UNSIGNED=$(git log --pretty='format:%H %G?' origin/master..HEAD | grep " N$" | wc -l | tr -d ' ')
    if [ "$UNSIGNED" -gt 0 ]; then
        echo "⚠️ Unsigned commits [$UNSIGNED/$TOTAL]"
        exit 1
    else
        echo "🔏 All commits are signed"
    fi

_delete-example:
    rm -rf examples/node/data
    rm -rf examples/block_wallet_sync/data

_delete-target:
    rm -rf target/

_delete-lockfile:
    rm -f Cargo.lock
    rm -f Cargo-recent.lock
    rm -f Cargo-minimal.lock
