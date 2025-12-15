// SPDX-Licence-Identifier: MIT

// TODO(@luisschwab): make all error variants
// transparent once they get implemented on Floresta.

use std::sync::Arc;
use thiserror::Error;

#[allow(unused)]
use crate::builder::FlorestaBuilder;
#[allow(unused)]
use crate::FlorestaNode;

/// [`FlorestaNode`] related errors.
#[derive(Clone, Debug, Error)]
pub enum NodeError {
    /// Shutdown error.
    #[error("Failed to perform a clean shutdown")]
    Shutdown,

    /// Sender dropped without sending.
    #[error(transparent)]
    Receive(#[from] tokio::sync::oneshot::error::RecvError),

    /// DB error during persistence.
    #[error("Database error: {0:?}")]
    Database(Arc<Box<dyn floresta_chain::DatabaseError>>),

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

/// [`FlorestaBuilder`] related errors.
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
