// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Error
//!
//! This module holds error enums for [`Builder`](crate::builder::Builder)-related
//! or [`Node`](crate::node::Node)-related errors, along with their `From` implementations.

use std::sync::Arc;

use bitcoin::BlockHash;
use thiserror::Error;

/// Errors which might occur when building the
/// [`Node`](crate::node::Node) or [logger](crate::logger::Logger).
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("Failed to create the data directory: {0:?}")]
    CreateDirectory(#[from] std::io::Error),

    #[error("Failed to create a ChainState: {0:?}")]
    ChainState(#[from] floresta_chain::BlockchainError),

    #[error("Failed to setup the tracing subscriber logger: {0:?}")]
    LoggerSetup(std::io::Error),

    #[error("Tracing subscriber logger already initialized")]
    LoggerAlreadySetup,

    #[error("Failed to load or create a new chain store: {0:?}")]
    ChainStoreInit(floresta_chain::FlatChainstoreError),

    #[error("Node and Wallet are not on the same network")]
    NetworkMismatch,

    #[error("Compact Block Filter error: {0:?}")]
    CompactBlockFilter(floresta_compact_filters::IterableFilterStoreError),

    #[error("Failed to build the inner node: {0}")]
    BuildInner(floresta_wire::error::WireError),
}

impl From<floresta_chain::FlatChainstoreError> for BuilderError {
    fn from(err: floresta_chain::FlatChainstoreError) -> Self {
        BuilderError::ChainStoreInit(err)
    }
}

impl From<floresta_compact_filters::IterableFilterStoreError> for BuilderError {
    fn from(err: floresta_compact_filters::IterableFilterStoreError) -> Self {
        BuilderError::CompactBlockFilter(err)
    }
}

/// Errors that can occur whilst interacting with the [`Node`](crate::node::Node).
#[derive(Debug, Error)]
pub enum NodeError {
    /// The [`Node`](crate::node::Node) is already running.
    #[error("The node is already running")]
    AlreadyRunning,

    /// The [`Node`](crate::node::Node) failed to perform a clean shutdown.
    #[error("The node failed to perform a clean shutdown")]
    Shutdown,

    /// The [`Node`](crate::node::Node)'s sender dropped without sending.
    #[error(transparent)]
    Receiver(#[from] tokio::sync::oneshot::error::RecvError),

    /// The [`Node`](crate::node::Node) failed to flush the chain state to disk.
    #[error("Failed to flush chain state: {0:?}")]
    Flush(floresta_chain::BlockchainError),

    /// I/O error during persistence.
    #[error("I/O error: {0}")]
    Io(Arc<std::io::Error>),

    /// Blockchain related errors.
    #[error("Blockchain error: {0:?}")]
    Blockchain(floresta_chain::BlockchainError),

    /// Mempool related errors.
    #[error("Mempool acceptance error: {0:?}")]
    Mempool(floresta_mempool::mempool::AcceptToMempoolError),

    /// Failed to fetch all of the requested blocks.
    #[error("Failed to fetch all all of the requested blocks (the block with hash={0:?} does not exist)")]
    MissingBlock(BlockHash),
}

impl From<std::io::Error> for NodeError {
    fn from(err: std::io::Error) -> Self {
        NodeError::Io(Arc::new(err))
    }
}

impl From<floresta_chain::BlockchainError> for NodeError {
    fn from(err: floresta_chain::BlockchainError) -> Self {
        NodeError::Blockchain(err)
    }
}

impl From<floresta_mempool::mempool::AcceptToMempoolError> for NodeError {
    fn from(err: floresta_mempool::mempool::AcceptToMempoolError) -> Self {
        NodeError::Mempool(err)
    }
}
