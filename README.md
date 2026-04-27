# bdk-floresta

<p>
    <!-- <a href="https://crates.io/crates/bdk_floresta"><img src="https://img.shields.io/crates/v/bdk_floresta.svg"/></a> -->
    <!-- <a href="https://docs.rs/bdk_floresta"><img src="https://img.shields.io/badge/docs.rs-bdk--floresta-brightgreen"/></a> -->
    <a href="https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/"><img src="https://img.shields.io/badge/MSRV-1.85.0%2B-orange.svg"/></a>
    <a href="https://github.com/luisschwab/bdk-floresta/blob/master/LICENSE"><img src="https://img.shields.io/badge/License-MIT%2FApache--2.0-red.svg"/></a>
    <a href="https://github.com/luisschwab/bdk-floresta/actions/workflows/rust.yml"><img src="https://github.com/luisschwab/bdk-floresta/actions/workflows/rust.yml/badge.svg"></a>
    <a href="https://github.com/luisschwab/bdk-floresta/actions/workflows/integration.yml"><img src="https://github.com/luisschwab/bdk-floresta/actions/workflows/integration.yml/badge.svg"></a>
</p>

A Floresta-powered chain-source crate for BDK.

## Synopsis

This crate is a novel chain-source crate implementation meant for wallet applications
built with [`BDK`](https://github.com/bitcoindevkit/bdk_wallet), allowing for zero-trust
and completely local wallet synchronization, by embedding a lightweight full node direcly
inside a wallet application.

By leveraging [`Utreexo`](https://eprint.iacr.org/2019/611) and [`Floresta`](https://github.com/getfloresta/Floresta),
it's possible to implement a lightweight full node with minimal resource requirements,
making it possible for every wallet to have it's own node.

## Developing

This project uses [`just`](https://github.com/casey/just) for command running, and
[`cargo-rbmt`](https://github.com/rust-bitcoin/rust-bitcoin-maintainer-tools/tree/master/cargo-rbmt)
to manage everything related to `cargo`, such as formatting, linting, testing and CI.
It also uses `uv` to run Zizmor, for GitHub Action static analysis.

To install them with `cargo`, run:

```shell
~$ cargo install just

~$ cargo install cargo-rbmt

~$ cargo install uv
```

Alternatively, use your preferred package manager to install `just` and `uv`.

A `justfile` is provided for convenience. Run `just` to see available commands:

```shell
~$ just
> bdk-floresta
> A Floresta-powered chain-source crate for BDK

Available recipes:
    build               # Build `bdk-floresta` and examples [alias: b]
    check               # Check code formatting, compilation, linting, and commit signatures [alias: c]
    check-features      # Check that all feature combinations compile
    check-sigs          # Check if commits are PGP-signed
    delete item="data"  # Delete files: data, target, lockfiles [alias: d]
    doc                 # Generate documentation
    doc-open            # Generate and open documentation
    example-fsm         # Run the `fsm` example [alias: fsm]
    example-regtest     # Run the `regtest` example [alias: reg]
    example-signet      # Run the `signet` example [alias: sig]
    example-wallet-sync # Run the `sync` example [alias: ws]
    fmt                 # Format code [alias: f]
    lock                # Regenerate `Cargo-recent.lock` and `Cargo-minimal.lock` [alias: l]
    pre-push            # Run pre-push checks [alias: p]
    test                # Run tests across all toolchains and lockfiles [alias: t]
```

## Architecture

This crate is implemented on top of [`libfloresta`](https://github.com/getfloresta/Floresta/tree/master/crates),
a set of libraries for building lightweight Bitcoin nodes, and is composed of two main components: the `Builder` and the `Node`.

Wallet updates can either stem from new blocks that the `Node` validates, which can then be applied to the wallet, or
from a blockchain scan using [Compact Block Filters](https://bips.dev/158).

### libfloresta

The crates below are used to implement the `Node`:

- [`floresta-chain`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-chain):
  Implements the `Node`'s chain state, and validates blocks and transactions.

- [`floresta-compact-filters`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-compact-filters):
  Implements a storage mechanism for [Compact Block Filters](https://bips.dev/158), 
  used to figure out which blocks to request from the P2P network to update the wallet.

- [`floresta-mempool`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-mempool):
  Implements the `Node`'s mempool, used to relay and cache transactions, as well as generating fee estimates.

- [`floresta-wire`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-wire):
  Implements all of the `Node`'s peer-to-peer logic.

## Minimum Supported Rust Version

This library should compile with any combination of features on Rust 1.85.0.

To build with the MSRV toolchain, copy `Cargo-minimal.lock` to `Cargo.lock`.

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
