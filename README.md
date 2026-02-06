# bdk-floresta

<p>
    <a href="https://crates.io/crates/bdk_floresta"><img src="https://img.shields.io/crates/v/bdk_floresta.svg"/></a>
    <a href="https://docs.rs/bdk_floresta"><img src="https://img.shields.io/badge/docs.rs-bdk--floresta-green"/></a>
    <a href="https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/"><img src="https://img.shields.io/badge/rustc-1.81.0%2B-lightgrey.svg"/></a>
    <a href="https://github.com/luisschwab/bdk-floresta/blob/master/LICENSE"><img src="https://img.shields.io/badge/License-MIT-red.svg"/></a>
    <a href="https://github.com/luisschwab/bdk-floresta/actions/workflows/ci.yml"><img src="https://github.com/luisschwab/bdk-floresta/actions/workflows/ci.yml/badge.svg"></a>
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

A `justfile` is provided for convenience. Run `just` to see available commands:

```console
% just
bdk_floresta
    build                  # Build `bdk_floresta` [alias: b]
    check                  # Check code formatting, compilation, linting, and commit signature [alias: c]
    check-features         # Check that all feature combinations compile
    delete item="examples" # Delete files: example, target, lockfile [alias: d]
    doc                    # Build documentation
    example name="node"    # Run an example crate [alias: e]
    fmt                    # Format code [alias: f]
```

## Architecture

This crate is implemented on top of [`libfloresta`](https://github.com/getfloresta/Floresta/tree/master/crates),
a set of libraries for building lightweight Bitcoin nodes, and is composed of two main components: the `Builder` and the `Node`.

Wallet updates can either stem from new blocks that the `Node` validates, which can then be applied to the wallet, or
from a blockchain scan using [Compact Block Filters](https://bips.dev/158).

### libfloresta

The crates below are used to implement the `Node`:

- [`floresta-wire`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-wire):
  Implements all of the `Node`'s peer-to-peer logic.

- [`floresta-chain`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-chain):
  Implements the `Node`'s chain state, and validates blocks and transactions.

- [`floresta-mempool`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-mempool):
  Implements the `Node`'s mempool, used to relay and cache transactions, as well as generating fee estimates.

- [`floresta-compact-filters`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-compact-filters):
  Implements a storage mechanism for [Compact Block Filters](https://bips.dev/158), 
  used to figure out which blocks to request from the P2P network to update the wallet.

### Builder

The `Builder` is used to create a new `Node` from parameters defined in `NodeConfig`.

```rs
use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bitcoin::Network;

let config = NodeConfig {
    network: Network::Signet,
    assume_utreexo: true,
    perform_backfill: false,
    mempool_size: 100,
    ..Default::default()
};

let mut node = Builder::new()
    .from_config(config)
    .build()?;
```

### Node

After building the `Node`, you can run and interact with it:

```rs
use bitcoin::Block;
use bitcoin::BlockHash;
use bitcoin::Network;
use floresta_wire::rustreexo::accumulator::stump::Stump;

// Run the node
node.run().await?;

// Get the the hash of block at height 250_000
let hash: BlockHash = node.get_blockhash(250_000).unwrap();

// Get the block at height 250_000, if it exists
let block: Option<Block> = node.get_block(hash).await.unwrap();

// Get the chain's height
let height: u32 = node.get_height().unwrap();

// Get the node's validated height
let validated_height: u32 = node.get_validation_height().unwrap();

// Get the node's current Utreexo accumulator
let stump: Stump = node.get_accumulator().unwrap();
```

### Wallet Synchronization

TODO (needs upstream work on [`floresta-compact-filters`](https://github.com/getfloresta/Floresta/tree/master/crates/floresta-compact-filters))

## License

This library is licensed under the [MIT License](./LICENSE).
