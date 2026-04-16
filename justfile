alias b := build
alias c := check
alias d := delete
alias e := example
alias f := fmt
alias l := lock

_default:
    @echo "> bdk-floresta"
    @echo "> A Floresta-powered chain-source crate for BDK"
    @just --list

[doc: "Build `bdk-floresta` and examples"]
build:
    cargo build --release
    cargo build --release --examples

[doc: "Check code formatting, compilation, linting, and commit signatures"]
check:
    cargo rbmt fmt --check
    cargo rbmt lint
    cargo rbmt docs
    just check-sigs

[doc: "Check that all feature combinations compile"]
check-features:
    cargo rbmt test --toolchain stable --lock-file recent

[doc: "Check if commits are PGP-signed"]
check-sigs:
    bash contrib/check-signatures.sh

[doc: "Delete files: data, target, lockfiles"]
delete item="data":
    just _delete-{{ item }}

[doc: "Generate documentation"]
doc:
    cargo rbmt docs

[doc: "Generate and open documentation"]
doc-open:
    cargo rbmt docs --open

[doc: "Run an example: `regtest`, `signet`"]
example name="regtest":
    rm -rf examples/data/regtest_ibd
    cargo run --release --example {{ name }}

[doc: "Format code"]
fmt:
    cargo rbmt fmt

[doc: "Regenerate `Cargo-recent.lock` and `Cargo-minimal.lock`"]
lock:
  cargo +nightly rbmt lock

[doc: "Check if this library builds with the MSRV toolchain"]
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
