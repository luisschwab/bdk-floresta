// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Errors
//!
//! This module holds error enums for [`Builder`]
//! and [`Node`]-related along with their `From` implementations.

use core::error;
use core::fmt;
use std::io;

#[allow(unused)]
use bitcoin::ScriptBuf;

#[allow(unused)]
use crate::builder::Builder;
#[allow(unused)]
use crate::fsm::State;
#[cfg(feature = "logger")]
#[allow(unused)]
use crate::logger::Logger;
#[allow(unused)]
use crate::node::Action;
#[allow(unused)]
use crate::node::Node;

/// Errors which might occur when building the [`Node`] or [`Logger`].
#[derive(Debug)]
pub enum BuilderError {
    /// Failed to create the data directory.
    CreateDirectory(io::Error),
    /// Failed to create a ChainState.
    ChainState(floresta_chain::BlockchainError),
    /// Failed to load or create a new chain store.
    ChainStoreInit(floresta_chain::FlatChainstoreError),
    /// Node and Wallet are not on the same network.
    NetworkMismatch,
    /// Compact Block Filter error.
    CompactBlockFilter(floresta_compact_filters::IterableFilterStoreError),
    /// Failed to build the inner node.
    BuildInner(floresta_wire::error::WireError),
    #[cfg(feature = "logger")]
    /// Failed to setup the tracing subscriber logger.
    LoggerSetup(io::Error),
    #[cfg(feature = "logger")]
    /// Tracing subscriber logger already initialized.
    LoggerAlreadySetup,
}

impl fmt::Display for BuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuilderError::CreateDirectory(e) => {
                write!(f, "Failed to create the data directory: {e:?}")
            }
            BuilderError::ChainState(e) => write!(f, "Failed to create a ChainState: {e:?}"),
            BuilderError::ChainStoreInit(e) => {
                write!(f, "Failed to load or create a new chain store: {e:?}")
            }
            BuilderError::NetworkMismatch => {
                write!(f, "Node and Wallet are not on the same network")
            }
            BuilderError::CompactBlockFilter(e) => write!(f, "Compact Block Filter error: {e:?}"),
            BuilderError::BuildInner(e) => write!(f, "Failed to build the inner node: {e}"),
            #[cfg(feature = "logger")]
            BuilderError::LoggerSetup(e) => {
                write!(f, "Failed to setup the tracing subscriber logger: {e:?}")
            }
            #[cfg(feature = "logger")]
            BuilderError::LoggerAlreadySetup => {
                write!(f, "Tracing subscriber logger already initialized")
            }
        }
    }
}

impl error::Error for BuilderError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            BuilderError::CreateDirectory(e) => Some(e),
            #[cfg(feature = "logger")]
            BuilderError::LoggerSetup(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for BuilderError {
    fn from(e: io::Error) -> Self {
        BuilderError::CreateDirectory(e)
    }
}

impl From<floresta_chain::BlockchainError> for BuilderError {
    fn from(e: floresta_chain::BlockchainError) -> Self {
        BuilderError::ChainState(e)
    }
}

impl From<floresta_chain::FlatChainstoreError> for BuilderError {
    fn from(e: floresta_chain::FlatChainstoreError) -> Self {
        BuilderError::ChainStoreInit(e)
    }
}

impl From<floresta_compact_filters::IterableFilterStoreError> for BuilderError {
    fn from(e: floresta_compact_filters::IterableFilterStoreError) -> Self {
        BuilderError::CompactBlockFilter(e)
    }
}

/// Errors that can occur whilst interacting with the [`Node`].
#[derive(Debug)]
pub enum NodeError {
    /// The [`Node`] is already running.
    AlreadyRunning,
    /// The [`Node`] failed to perform a clean shutdown.
    Shutdown,
    /// The [`Node`]'s sender dropped without sending.
    Receiver(tokio::sync::oneshot::error::RecvError),
    /// The [`Node`] failed to flush the chain state to disk.
    Flush(floresta_chain::BlockchainError),
    /// I/O error during persistence.
    Io(io::Error),
    /// Blockchain related errors.
    Blockchain(floresta_chain::BlockchainError),
    /// Mempool related errors.
    Mempool(floresta_mempool::mempool::AcceptToMempoolError),
    /// Failed to fetch all of the requested blocks.
    MissingBlock(bitcoin::BlockHash),
    /// The requested [`Action`] cannot be performed in the [`Node`]'s current [`State`].
    IllegalAction,
    /// No [scriptPubkeys](ScriptBuf) were provided.
    NoSpksProvided,
    /// Error whilst scanning the blockchain with Compact Block Filters.
    CompactFilterScan(floresta_compact_filters::IterableFilterStoreError),
    /// Failed to join a task whilst fetching blocks.
    FetchTask(tokio::task::JoinError),
    /// Compact Block Filter-related errors.
    CompactBlockFilterStore(floresta_compact_filters::IterableFilterStoreError),
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeError::AlreadyRunning => write!(f, "The node is already running"),
            NodeError::Shutdown => write!(f, "The node failed to perform a clean shutdown"),
            NodeError::Receiver(e) => write!(f, "{e}"),
            NodeError::Flush(e) => write!(f, "Failed to flush chain state: {e:?}"),
            NodeError::Io(e) => write!(f, "I/O error: {e}"),
            NodeError::Blockchain(e) => write!(f, "Blockchain error: {e:?}"),
            NodeError::Mempool(e) => write!(f, "Mempool acceptance error: {e:?}"),
            NodeError::MissingBlock(h) => {
                write!(f, "The block with hash={h:?} does not exist in the chain)")
            }
            NodeError::IllegalAction => write!(
                f,
                "The requested action cannot be performed in the node's current state"
            ),
            NodeError::NoSpksProvided => write!(
                f,
                "No scriptPubkeys were provided when rescanning the blockchain"
            ),
            NodeError::CompactFilterScan(e) => write!(
                f,
                "Error whilst scanning the blockchain with Compact Block Filters: {e:?}"
            ),
            NodeError::FetchTask(e) => write!(f, "Block fetch task failed: {e}"),
            NodeError::CompactBlockFilterStore(e) => write!(f, "Compact Block Filter error: {e}"),
        }
    }
}

impl error::Error for NodeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            NodeError::Receiver(e) => Some(e),
            NodeError::FetchTask(e) => Some(e),
            NodeError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for NodeError {
    fn from(e: io::Error) -> Self {
        NodeError::Io(e)
    }
}

impl From<tokio::sync::oneshot::error::RecvError> for NodeError {
    fn from(e: tokio::sync::oneshot::error::RecvError) -> Self {
        NodeError::Receiver(e)
    }
}

impl From<tokio::task::JoinError> for NodeError {
    fn from(e: tokio::task::JoinError) -> Self {
        NodeError::FetchTask(e)
    }
}

impl From<floresta_chain::BlockchainError> for NodeError {
    fn from(e: floresta_chain::BlockchainError) -> Self {
        NodeError::Blockchain(e)
    }
}

impl From<floresta_mempool::mempool::AcceptToMempoolError> for NodeError {
    fn from(e: floresta_mempool::mempool::AcceptToMempoolError) -> Self {
        NodeError::Mempool(e)
    }
}

impl From<floresta_compact_filters::IterableFilterStoreError> for NodeError {
    fn from(e: floresta_compact_filters::IterableFilterStoreError) -> Self {
        NodeError::CompactBlockFilterStore(e)
    }
}
