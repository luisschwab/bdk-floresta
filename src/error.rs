// SPDX-Licence-Identifier: MIT

//! # Error
//!
//! This module holds error enums for [`Builder`](crate::builder::Builder)-related
//! or [`Node`](crate::node::Node)-related errors, along with their
//! `Display` and `From` implementations.

use std::sync::Arc;

use thiserror::Error;

/// Errors which might occur when building the [`Node`](crate::node::Node) or
/// [logger](crate::logger::setup_logger).
#[derive(Clone, Debug, Error)]
pub enum BuilderError {
    #[error("Failed to create the data directory: {0:?}")]
    CreateDirectory(Arc<std::io::Error>),

    #[error("Failed to setup the tracing subscriber logger: {0:?}")]
    LoggerSetup(Arc<std::io::Error>),

    #[error("Tracing subscriber logger already initialized")]
    LoggerAlreadySetup,

    #[error("Failed to load or create a new chain store: {0:?}")]
    ChainStoreInit(Arc<floresta_chain::FlatChainstoreError>),

    #[error("Node and Wallet are not on the same network")]
    NetworkMismatch,

    /// Compact Block Filter related errors.
    #[error("Compact Block Filter error: {0:?}")]
    CompactBlockFilter(Arc<floresta_compact_filters::IterableFilterStoreError>),
}

impl From<std::io::Error> for BuilderError {
    fn from(err: std::io::Error) -> Self {
        BuilderError::CreateDirectory(Arc::new(err))
    }
}

impl From<floresta_chain::FlatChainstoreError> for BuilderError {
    fn from(err: floresta_chain::FlatChainstoreError) -> Self {
        BuilderError::ChainStoreInit(Arc::new(err))
    }
}

impl From<floresta_compact_filters::IterableFilterStoreError> for BuilderError {
    fn from(err: floresta_compact_filters::IterableFilterStoreError) -> Self {
        BuilderError::CompactBlockFilter(Arc::new(err))
    }
}

/// Errors which might occur when running the [`Node`](crate::node::Node).
#[derive(Clone, Debug, Error)]
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

    /// The [`Node`](crate::node::Node) threw an error while persisting data to the file system.
    #[error("Persistence error: {0:?}")]
    Persistence(Arc<Box<dyn floresta_chain::DatabaseError>>),

    /// I/O error during persistence.
    #[error("I/O error: {0}")]
    Io(Arc<std::io::Error>),

    /// Blockchain related errors.
    #[error("Blockchain error: {0:?}")]
    Blockchain(Arc<floresta_chain::BlockchainError>),
}

impl From<std::io::Error> for NodeError {
    fn from(err: std::io::Error) -> Self {
        NodeError::Io(Arc::new(err))
    }
}

impl From<floresta_chain::BlockchainError> for NodeError {
    fn from(err: floresta_chain::BlockchainError) -> Self {
        NodeError::Blockchain(Arc::new(err))
    }
}
