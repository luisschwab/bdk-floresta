alias b := build
alias c := check
alias d := delete
alias e := example
alias f := fmt

_default:
    @just --list --list-heading $'> bdk-floresta\n> A Floresta-powered chain-source crate for BDK\n\n> Available recipes:\n'

# Build `bdk-floresta` and examples
build:
    cargo build --release
    cargo build --release --examples

# Check code formatting, compilation, linting, and commit signature
check:
    cargo rbmt lint
    cargo rbmt docs
    just check-sigs

# Check that all feature combinations compile
check-features:
    cargo rbmt test --toolchain stable --lock-file recent

# Checks whether all commits in this branch are signed
check-sigs:
    #!/usr/bin/env bash
    TOTAL=$(git log --pretty='tformat:%H' origin/master..HEAD | wc -l | tr -d ' ')
    UNSIGNED=$(git log --pretty='tformat:%H %G?' origin/master..HEAD | grep " N$" | wc -l | tr -d ' ')
    if [ "$UNSIGNED" -gt 0 ]; then
        echo "⚠️ Unsigned commits in this branch [$UNSIGNED/$TOTAL]"
        exit 1
    else
        echo "🔏 All commits in this branch are signed [$TOTAL/$TOTAL]"
    fi

# Delete files: data, target, lockfiles
delete item="data":
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
    cargo run --release --example {{ name }}

# Format code
fmt:
    cargo rbmt fmt

# Regenerate `Cargo-recent.lock` and `Cargo-minimal.lock`
lock:
  cargo +nightly rbmt lock

# Verify the library builds with MSRV (1.85.0)
msrv:
    cargo rbmt test --toolchain msrv --lock-file minimal

_delete-data:
    rm -rf examples/data

_delete-target:
    rm -rf target/

_delete-lockfiles:
    rm -f Cargo.lock
    rm -f Cargo-recent.lock
    rm -f Cargo-minimal.lock
