// SPDX-License-Identifier: MIT OR Apache-2.0

#![doc = include_str!("../README.md")]

pub use floresta::chain::BlockConsumer;
pub use floresta::chain::UtxoData;
pub use floresta::chain::pruned_utreexo::chainparams::AssumeUtreexoValue;
pub use floresta::chain::pruned_utreexo::chainparams::ChainParams;
pub use floresta::wire::TransportProtocol;
pub use floresta::wire::UtreexoNodeConfig;
pub use floresta::wire::node::ConnectionKind;
pub use floresta::wire::node::PeerStatus;
pub use floresta::wire::node::UtreexoNode;
pub use floresta::wire::node::running_ctx::RunningNode;
pub use floresta::wire::node_interface::PeerInfo;
pub use floresta::wire::rustreexo;

pub mod client;
#[cfg(feature = "logger")]
pub mod logger;
pub mod node;

pub use node::builder;
pub use node::error;
pub use node::fsm;
#[cfg(feature = "logger")]
pub use tracing::Level;
