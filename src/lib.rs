// SPDX-License-Identifier: MIT OR Apache-2.0

#![doc = include_str!("../README.md")]

pub use floresta_chain::BlockConsumer;
pub use floresta_chain::UtxoData;
pub use floresta_chain::pruned_utreexo::chainparams::AssumeUtreexoValue;
pub use floresta_chain::pruned_utreexo::chainparams::ChainParams;
pub use floresta_wire::TransportProtocol;
pub use floresta_wire::UtreexoNodeConfig;
pub use floresta_wire::node::ConnectionKind;
pub use floresta_wire::node::PeerStatus;
pub use floresta_wire::node::UtreexoNode;
pub use floresta_wire::node::running_ctx::RunningNode;
pub use floresta_wire::node_interface::PeerInfo;
pub use floresta_wire::rustreexo;

pub mod client;
#[cfg(feature = "logger")]
pub mod logger;
pub mod node;

pub use node::builder;
pub use node::error;
pub use node::fsm;
#[cfg(feature = "logger")]
pub use tracing::Level;
