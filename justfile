alias b := build
alias c := check
alias d := delete
alias f := fmt
alias fsm := example-fsm
alias reg := example-regtest
alias sig := example-signet
alias ws := example-wallet-sync
alias l := lock
alias t := test
alias p := pre-push

_default:
    @echo "> bdk-floresta"
    @echo "> A Floresta-powered chain-source crate for BDK\n"
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

[doc: "Run the `fsm` example"]
example-fsm:
    rm -rf examples/data/fsm
    cargo run --release --example fsm

[doc: "Run the `regtest` example"]
example-regtest:
    rm -rf examples/data/regtest
    cargo run --release --example regtest

[doc: "Run the `signet` example"]
example-signet:
    rm -rf examples/data/signet
    cargo run --release --example signet

[doc: "Format code"]
fmt:
    cargo rbmt fmt

[doc: "Regenerate `Cargo-recent.lock` and `Cargo-minimal.lock`"]
lock:
  cargo +nightly rbmt lock

[doc: "Run tests across all toolchains and lockfiles"]
test:
    cargo rbmt test

[doc: "Run pre-push checks"]
pre-push:
    @just check-sigs
    @just check
    @just doc
    @just test

_delete-data:
    rm -rf examples/data

_delete-target:
    rm -rf target/

_delete-lockfiles:
    rm -f Cargo.lock
    rm -f Cargo-recent.lock
    rm -f Cargo-minimal.lock
