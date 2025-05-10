_default:
  @just --list

fmt:
  cargo +nightly fmt

check:
  cargo +nightly fmt -- --check
  cargo +nightly clippy -- -D warnings
  cargo +nightly check --all-features

example name="example":
  cargo run --example {{name}} --release

# Options: bitcoin, signet, lockfile
delete item="data":
  just _delete-{{item}}

_delete-bitcoin:
  rm -rf data/bitcoin

_delete-signet:
  rm -rf data/signet

_delete-lockfile:
  rm -f Cargo.lock
