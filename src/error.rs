// SPDX-Licence-Identifier: MIT

// TODO(@luisschwab): make all error variants
// transparent once they get implemented on Floresta.

use thiserror::Error;

/// [`FlorestaNode`] related errors.
#[derive(Debug, Error)]
pub enum NodeError {
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
pub enum BuilderError {}
