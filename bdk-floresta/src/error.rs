// SPDX-Licence-Identifier: MIT

// TODO(@luisschwab): make all error variants
// transparent once they get implemented on Floresta.

use thiserror::Error;

/// [`FlorestaNode`] related errors.
#[derive(Debug, Error)]
pub enum NodeError {
    /// Shutdown error.
    #[error("failed to perform a clean shutdown")]
    Shutdown,

    /// Sender dropped without sending.
    #[error(transparent)]
    Receive(#[from] tokio::sync::oneshot::error::RecvError),

    /// DB error during persistence.
    #[error("database error: {0:?}")]
    Database(Box<dyn floresta_chain::DatabaseError>),

    /// I/O error during persistence.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Blockchain related errors.
    #[error("blockchain error: {0:?}")]
    Blockchain(#[from] floresta_chain::BlockchainError),
}

#[allow(unused)]
/// [`FlorestaBuilder`] related errors.
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("failed to create the data directort: {0:?}")]
    CreateDirectory(#[from] std::io::Error),

    #[error("failed to setup the tracing subscriber logger: {0:?}")]
    LoggerSetup(std::io::Error),

    #[error("tracing subscriber logger already initialized")]
    LoggerAlreadySetup,

    #[error("failed to load or create a new chain store: {0:?}")]
    ChainStoreInit(floresta_chain::FlatChainstoreError),

    #[error("node and wallet are not on the same network")]
    NetworkMismatch,
}
