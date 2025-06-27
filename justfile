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

# bitcoin, signet, testnet4, target, lockfile
delete item="data":
  just _delete-{{item}}

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
