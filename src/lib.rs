// SPDX-Licence-Identifier: MIT

//! # [`bdk_floresta`]
//!
//! This crate implements a novel chain-source crate for BDK by
//! leveraging [`libfloresta`], a set of reusable libraries for
//! building lightweight and embeddable nodes, allowing local,
//! zero-trust and sovereign blockchain querying and wallet
//! synchronization.
//!
//! Traditional API-based protocols, such as Electrum and Esplora,
//! require either trust on a server you don't control, or running
//! the necessary infrastructure to support them. Most users don't
//! have the required technical expertise to do so, and many of
//! those who do don't want to spend the time needed to set up
//! and maintain indexing servers.
//!
//! Embedding a full Bitcoin Core node inside an application is
//! not feasible in most instances, due to the large storage
//! requirement caused by the ever growing UTXO set, which stands
//! at over 11GB at the end of 2025.
//!
//! [`bdk_floresta`] combines the best of both worlds:
//!  * Local Validation: a full node in every application, meaning
//!    no address leakage to third-party API providers.
//!  * Small Footprint: by leveraging [Utreexo] to maintain a compact
//!    representation of the UTXO set in under 2KB, and
//!    [Compact Block Filters] to fetch wallet updates directly from
//!    the Bitcoin P2P network, the embedded node requires less than
//!    200MB of extra storage to operate, making it viable to run on
//!    a wide range of devices and applications. Every mobile wallet
//!    can have it's own embedded node.
//!
//! TODO: add examples (creating a node and requesting wallet updates via CBF).
//!
//! [`bdk_floresta`]: https://github.com/luisschwab/bdk-floresta/
//! [`libfloresta`]: https://github.com/getfloresta/Floresta/tree/master/crates/
//! [Utreexo]: https://eprint.iacr.org/2019/611/
//! [Compact Block Filters]: https://bips.dev/158/

pub use floresta_chain::pruned_utreexo::chainparams::AssumeUtreexoValue;
pub use floresta_chain::pruned_utreexo::chainparams::ChainParams;
pub use floresta_chain::BlockConsumer;
pub use floresta_chain::UtxoData;
pub use floresta_wire::node::running_ctx::RunningNode;
pub use floresta_wire::node::ConnectionKind;
pub use floresta_wire::node::PeerStatus;
pub use floresta_wire::node::UtreexoNode;
pub use floresta_wire::node_interface::PeerInfo;
pub use floresta_wire::rustreexo;
pub use floresta_wire::TransportProtocol;
pub use floresta_wire::UtreexoNodeConfig;

pub mod builder;
pub mod error;
pub mod logger;
pub mod node;
pub mod updater;

pub use crate::builder::Builder;
pub use crate::error::BuilderError;
pub use crate::error::NodeError;
pub use crate::node::Node;
pub use crate::updater::WalletUpdate;
