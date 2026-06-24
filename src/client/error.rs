// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Client Errors
//!
//! Error types related to the [`Client`] type.

use core::error;
use core::fmt;

use bitcoin::Network;

use crate::node::error::NodeError;

/// Errors that can occur when interacting with a [`Client`](crate::client::Client).
#[derive(Debug)]
pub enum ClientError {
    /// The [`Node`](crate::node::Node) and the [`Wallet`](bdk_wallet::Wallet) are not on the same network.
    NetworkMismatch { node_net: Network, wallet_net: Network },

    /// The scan has been aborted by the caller, or the [`Node`](crate::node::Node) was [`Drop`]ed.
    ScanAborted,

    /// The start height is greater than the stop height.
    InvalidRange { start_height: u32, stop_height: u32 },

    /// The [`Wallet`] has no [external keychain](bdk_wallet::KeychainKind::External).
    NoExternalKeychain,

    /// The associated [`Node`](crate::node::Node) is shutting down or inactive.
    UnresponsiveNode,

    /// The lock is poisoned.
    PoisonedLock,

    /// An error originating from the underlying [`Node`](crate::node::Node).
    Node(NodeError),
}

#[rustfmt::skip]
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetworkMismatch { node_net, wallet_net } => write!(f, "Network mismatch between node={node_net} and wallet={wallet_net}"),
            Self::ScanAborted => write!(f, "The scan was aborted"),
            Self::InvalidRange { start_height, stop_height } => write!(f, "Invalid scan range: start_height={start_height} > stop_height={stop_height}"),
            Self::NoExternalKeychain => write!(f, "The wallet has no external keychain"),
            Self::UnresponsiveNode => write!(f, "The node is unresponsive"),
            Self::PoisonedLock => write!(f, "The event channel's lock is poisoned"),
            Self::Node(e) => write!(f, "Node Error: {e}"),
        }
    }
}

impl error::Error for ClientError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::NetworkMismatch { .. } => None,
            Self::ScanAborted => None,
            Self::InvalidRange { .. } => None,
            Self::NoExternalKeychain => None,
            Self::UnresponsiveNode => None,
            Self::PoisonedLock => None,
            Self::Node(e) => Some(e),
        }
    }
}

impl From<NodeError> for ClientError {
    fn from(e: NodeError) -> Self {
        Self::Node(e)
    }
}
