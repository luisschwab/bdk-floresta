// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Builder and Node Errors
//!
//! Error types related to the [`Builder`](crate::node::builder::Builder) and [`Node`](crate::node::Node) types.

use core::error;
use core::fmt;
use std::io;

use crate::node::Action;
use crate::node::fsm::State;

/// Errors which might occur when building the [`Node`](crate::node::Node) or
/// [`Logger`](crate::logger::Logger).
#[derive(Debug)]
pub enum BuilderError {
    /// Failed to create the data directory.
    CreateDirectory(io::Error),

    /// Failed to create a `ChainState`.
    ChainState(floresta::chain::BlockchainError),

    /// Failed to load or create a new chain store.
    ChainStoreInit(floresta::chain::FlatChainstoreError),

    /// Compact Block Filter error.
    CompactBlockFilter(floresta::compact_filters::IterableFilterStoreError),

    /// Failed to build the inner node.
    BuildInner(floresta::wire::error::WireError),

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
            Self::CreateDirectory(e) => write!(f, "Failed to create the data directory: {e}"),
            Self::ChainState(e) => write!(f, "Failed to create a chainstate: {e}"),
            Self::ChainStoreInit(e) => write!(f, "Failed to load or create a new chain store: {e}"),
            Self::CompactBlockFilter(e) => write!(f, "Compact Block Filter error: {e}"),
            Self::BuildInner(e) => write!(f, "Failed to build the inner node: {e}"),
            #[cfg(feature = "logger")]
            Self::LoggerSetup(e) => write!(f, "Failed to setup the tracing subscriber logger: {e}"),
            #[cfg(feature = "logger")]
            Self::LoggerAlreadySetup => write!(f, "A tracing subscriber logger is already initialized"),
        }
    }
}

impl error::Error for BuilderError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::CreateDirectory(e) => Some(e),
            #[cfg(feature = "logger")]
            Self::LoggerSetup(e) => Some(e),
            _ => None,
        }
    }
}

/// Errors that can occur whilst interacting with the [`Node`](crate::node::Node).
#[derive(Debug)]
pub enum NodeError {
    /// The [`Node`](crate::node::Node) is not running.
    NotRunning,

    /// The [`Node`](crate::node::Node) is already running.
    AlreadyRunning,

    /// The [`Node`](crate::node::Node) failed to perform a clean shutdown.
    DirtyShutdown(io::Error),

    /// The [`Node`](crate::node::Node)'s sender dropped without sending.
    UnresponsiveNode(tokio::sync::oneshot::error::RecvError),

    /// Blockchain related errors.
    Blockchain(floresta::chain::BlockchainError),

    /// Mempool related errors.
    Mempool(floresta::domain::mempool::error::MempoolError),

    /// Failed to fetch all of the requested blocks.
    MissingBlock(bitcoin::BlockHash),

    /// The requested [`Action`] cannot be performed in the [`Node`](crate::node::Node)'s current [`State`].
    IllegalAction {
        /// The current [`Node`](crate::node::Node) state.
        state: State,
        /// The attempted [`Action`] that was attempted.
        attempted: Action,
    },

    /// No [script pubkeys](bitcoin::ScriptBuf) were provided.
    NoSpksProvided,

    /// Compact Block Filter related errors.
    CompactBlockFilter(floresta::compact_filters::IterableFilterStoreError),

    /// The requested `stop_height` exceeds the Compact Block Filter store's height.
    StopHeightExceedsFilterTip {
        /// The requested stop height.
        requested: u32,
        /// The available compact block filter height.
        available: u32,
    },
}

#[rustfmt::skip]
impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => write!(f, "The node is not running"),
            Self::AlreadyRunning => write!(f, "The node is already running"),
            Self::DirtyShutdown(e) => write!(f, "The node failed to perform a clean shutdown: {e}"),
            Self::UnresponsiveNode(e) => write!(f, "The node is unresponsive: {e}"),
            Self::Blockchain(e) => write!(f, "Blockchain error: {e}"),
            Self::Mempool(e) => write!(f, "Mempool error: {e}"),
            Self::MissingBlock(hash) => write!(f, "The block with hash={hash} does not exist in the chain"),
            Self::IllegalAction { state, attempted} => write!(f, "The requested action={attempted} cannot be performed at the current state={state}"),
            Self::NoSpksProvided => write!(f, "No spks were provided when rescanning the blockchain"),
            Self::CompactBlockFilter(e) => write!(f, "Compact Block Filter error: {e}"),
            Self::StopHeightExceedsFilterTip { requested, available } => write!(f, "The provided stop_height={requested} exceeds filter store height={available}"),
        }
    }
}

impl error::Error for NodeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::NotRunning => None,
            Self::AlreadyRunning => None,
            Self::DirtyShutdown(_) => None,
            Self::UnresponsiveNode(e) => Some(e),
            Self::Blockchain(e) => Some(e),
            Self::Mempool(e) => Some(e),
            Self::MissingBlock(_) => None,
            Self::IllegalAction { .. } => None,
            Self::NoSpksProvided => None,
            Self::CompactBlockFilter(e) => Some(e),
            Self::StopHeightExceedsFilterTip { .. } => None,
        }
    }
}
