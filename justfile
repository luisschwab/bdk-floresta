alias b := build
alias c := check
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

# Format code
fmt:
    cargo +nightly fmt

_delete-lockfile:
    rm -f Cargo.lock

