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

#[cfg(feature = "logger")]
#[allow(unused)]
use crate::logger::Logger;
#[allow(unused)]
use crate::node::fsm::State;
#[allow(unused)]
use crate::node::Action;
#[allow(unused)]
use crate::node::Builder;
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

#[rustfmt::skip]
impl fmt::Display for BuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuilderError::CreateDirectory(e) => write!(f, "Failed to create the data directory: {e}"),
            BuilderError::ChainState(e) => write!(f, "Failed to create a chainstate: {e}"),
            BuilderError::ChainStoreInit(e) => write!(f, "Failed to load or create a new chain store: {e}"),
            BuilderError::CompactBlockFilter(e) => write!(f, "Compact Block Filter error: {e}"),
            BuilderError::BuildInner(e) => write!(f, "Failed to build the inner node: {e}"),
            #[cfg(feature = "logger")]
            BuilderError::LoggerSetup(e) => write!(f, "Failed to setup the tracing subscriber logger: {e}"),
            #[cfg(feature = "logger")]
            BuilderError::LoggerAlreadySetup => write!(f, "A tracing subscriber logger is already initialized"),
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

/// Errors that can occur whilst interacting with the [`Node`].
#[derive(Debug)]
pub enum NodeError {
    /// The [`Node`] is not running.
    NotRunning,

    /// The [`Node`] is already running.
    AlreadyRunning,

    /// The [`Node`] failed to perform a clean shutdown.
    DirtyShutdown(io::Error),

    /// The [`Node`]'s sender dropped without sending.
    UnresponsiveNode(tokio::sync::oneshot::error::RecvError),

    /// Blockchain related errors.
    Blockchain(floresta_chain::BlockchainError),

    /// Mempool related errors.
    Mempool(floresta_mempool::mempool::MempoolError),

    /// Failed to fetch all of the requested blocks.
    MissingBlock(bitcoin::BlockHash),

    /// The requested [`Action`] cannot be performed in the [`Node`]'s current [`State`].
    IllegalAction { state: State, attempted: Action },

    /// No [script pubkeys](ScriptBuf) were provided.
    NoSpksProvided,

    /// Compact Block Filter related errors.
    CompactBlockFilter(floresta_compact_filters::IterableFilterStoreError),

    /// The requested `stop_height` exceeds the Compact Block Filter store's height.
    StopHeightExceedsFilterTip { requested: u32, available: u32 },
}

#[rustfmt::skip]
impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeError::NotRunning => write!(f, "The node is not running"),
            NodeError::AlreadyRunning => write!(f, "The node is already running"),
            NodeError::DirtyShutdown(e) => write!(f, "The node failed to perform a clean shutdown: {e}"),
            NodeError::UnresponsiveNode(e) => write!(f, "The node is unresponsive: {e}"),
            NodeError::Blockchain(e) => write!(f, "Blockchain error: {e}"),
            NodeError::Mempool(e) => write!(f, "Mempool error: {e}"),
            NodeError::MissingBlock(hash) => write!(f, "The block with hash={hash} does not exist in the chain"),
            NodeError::IllegalAction { state, attempted} => write!(f, "The requested action={attempted} cannot be performed at the current state={state}"),
            NodeError::NoSpksProvided => write!(f, "No spks were provided when rescanning the blockchain"),
            NodeError::CompactBlockFilter(e) => write!(f, "Compact Block Filter error: {e}"),
            NodeError::StopHeightExceedsFilterTip { requested, available } => write!(f, "The provided stop_height={requested} exceeds filter store height={available}"),
        }
    }
}

impl error::Error for NodeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            NodeError::NotRunning => None,
            NodeError::AlreadyRunning => None,
            NodeError::DirtyShutdown(_) => None,
            NodeError::UnresponsiveNode(e) => Some(e),
            NodeError::Blockchain(e) => Some(e),
            NodeError::Mempool(e) => Some(e),
            NodeError::MissingBlock(_) => None,
            NodeError::IllegalAction { .. } => None,
            NodeError::NoSpksProvided => None,
            NodeError::CompactBlockFilter(e) => Some(e),
            NodeError::StopHeightExceedsFilterTip { .. } => None,
        }
    }
}
