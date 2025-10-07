# bdk_floresta

`bdk_floresta` is a Floresta-powered chain-source crate for BDK.

This crate implements a BDK [`Wallet`] tightly coupled to
[Floresta](https://github.com/vinteumorg/Floresta), a lightweight,
fully-validating Bitcoin client that leverages Utreexo.

Instead of relying on external and possibly untrusted chain oracles, such
as an Esplora or Electrum server controled by a third-party, you are now
able to be your own chain oracle, doing block and trasaction validation,
transaction broadcasting, and querying blockchain data in a completely
local and trustless manner. The privacy improvements are huge, since you
don't need to trust a third-party to provide you with the truth, while
also making transaction behavior and IP address correlation pretty much
impossible.
 
All of this is made possible by [Utreexo](https://eprint.iacr.org/2019/611),
a hash-based accumulator for Bitcoin's UTXO set. It allows representing the
set in it's entirety in under 1KB of storage. This way, running fully-validating
clients on devices with reduced and low-performance storage, such as mobile
phones, becomes feasible, as the biggest bottleneck during IBD is not caused
by the CPU, but actually disk I/O during lookups in the UTXO set database
(especially on spinning drives, which have poor performance on non-linear reads),
which the Utreexo accumulator does not keep at all. Instead, it maintains a
compact representation of it, and relies on Merkle inclusion proofs of each UTXO
being spent in order to verify that they belong to the UTXO set. These proofs
must be kept by whoever controls those UTXOs, which are then attached to the
transaction before being sent to another Utreexo peer over the P2P network.

If you wan't to learn more, refer to
[_A Treatise on Utreexo_](https://luisschwab.net/blog/a-treatise-on-utreexo),
my blog post on the topic.

Also refer to the more complete documentation under [`docs/`](./docs/README.md), which go in depth
on this crate's architecture and Utreexo concepts.
