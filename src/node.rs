// SPDX-License-Identifier: MIT

//! # Node
//!
//! This module holds all the logic needed to interact with the embedded [`Node`].
//!
//! # Building, running, and interacting with a Node
//!
//! ```rust,no_run
//! use bdk_floresta::builder::Builder;
//! use bdk_floresta::builder::NodeConfig;
//! use bitcoin::Block;
//! use bitcoin::BlockHash;
//! use bitcoin::Network;
//! use floresta_wire::rustreexo::accumulator::stump::Stump;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Define configuration parameters for the node
//!     let config = NodeConfig {
//!         network: Network::Signet,
//!         assume_utreexo: true,
//!         enable_powfps: true,
//!         perform_backfill: false,
//!         mempool_size: 300,
//!         ..Default::default()
//!     };
//!
//!     // Build the node
//!     let mut node = Builder::new().from_config(config).build().unwrap();
//!
//!     // Run the node
//!     node.run().await.unwrap();
//!
//!     // Get the hash of block at height 250_000
//!     let hash: BlockHash = node.get_blockhash(250_000).unwrap();
//!
//!     // Get the block at height 250_000, if it exists
//!     let block: Option<Block> = node.get_block(hash).await.unwrap();
//!
//!     // Get the chain's height
//!     let height: u32 = node.get_height().unwrap();
//!
//!     // Get the node's validated height
//!     let validated_height: u32 = node.get_validation_height().unwrap();
//!
//!     // Get the node's current Utreexo accumulator
//!     let stump: Stump = node.get_accumulator().unwrap();
//! }
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bdk_wallet::Wallet;
use bitcoin::Block;
use bitcoin::BlockHash;
use bitcoin::Transaction;
use bitcoin::Txid;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStore;
use floresta_chain::pruned_utreexo::BlockchainInterface;
use floresta_chain::pruned_utreexo::UpdatableChainstate;
use floresta_chain::BlockConsumer;
use floresta_chain::BlockchainError;
use floresta_chain::ChainState;
use floresta_wire::node::running_ctx::RunningNode;
use floresta_wire::node::UtreexoNode;
use floresta_wire::node_interface::NodeInterface;
use floresta_wire::node_interface::PeerInfo;
use floresta_wire::rustreexo::accumulator::stump::Stump;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;
#[cfg(feature = "logger")]
use tracing_appender::non_blocking::WorkerGuard;

use crate::builder::NodeConfig;
use crate::error::NodeError;
use crate::updater::WalletUpdate;

/// How long to wait for the [`Node`]'s shutdown task before aborting, in seconds.
const SHUTDOWN_TIMEOUT: u64 = 15;

/// Type alias for the [`Node`]'s inner [`UtreexoNode`].
type NodeInner = UtreexoNode<Arc<ChainState<FlatChainStore>>, RunningNode>;

/// The embedded and fully-validating Compact State [`Node`].
pub struct Node {
    /// The [`Node`]'s configuration settings.
    pub(crate) config: NodeConfig,
    /// The inner, underlying [`UtreexoNode`].
    pub(crate) node_inner: Option<NodeInner>,
    /// A handle used to interact with the [`UtreexoNode`].
    pub(crate) node_handle: NodeInterface,
    /// The [`Node`]'s blockchain state.
    pub(crate) chain_state: Arc<ChainState<FlatChainStore>>,
    /// Handle to the spawned [`Node`] task, used for graceful shutdown coordination.
    pub(crate) task_handle: Option<JoinHandle<()>>,
    /// Handle to the `SIGINT` handler task, used to detect
    /// a `SIGINT` (CTRL-C) and set the `stop_signal` to true.
    pub(crate) sigint_task: Option<JoinHandle<()>>,
    /// Signal used by the [`Node`] to check if it
    /// should stop operations and perform a graceful shutdown.
    pub(crate) stop_signal: Arc<RwLock<bool>>,
    /// Receiver for shutdown completion notification from the [`Node`]'s task.
    pub(crate) stop_receiver: Option<oneshot::Receiver<()>>,
    /// A guard that ensures the logger remains active for the [`Node`]'s lifetime.
    #[cfg(feature = "logger")]
    pub(crate) _logger_guard: Option<WorkerGuard>,
    /// A [`Wallet`] that will receive updates from the [`Node`].
    pub wallet: Option<Arc<RwLock<Wallet>>>,
    /// Receiver for [`WalletUpdate`]s that come from
    /// the [`Node`] and should be applied to the [`Wallet`].
    pub update_subscriber: Option<UnboundedReceiver<WalletUpdate>>,
}

impl Node {
    /// Spawn the [`Node`]'s background tasks and run it.
    ///
    /// This method will setup the [`Node`]'s shutdown notification channel,
    /// spawn the [`Node`]'s tokio task, and the [`Node`]'s `SIGINT` handler task.
    pub async fn run(&mut self) -> Result<(), NodeError> {
        // Take the [`Node`]'s inner. This asserts that [`Node::run()`] can only be called once.
        let inner_node = self.node_inner.take().ok_or(NodeError::AlreadyRunning)?;

        // Create channel for shutdown notifications.
        let (sender, receiver) = oneshot::channel();
        self.stop_receiver = Some(receiver);

        // Spawn the [`Node`]'s task.
        let node_task: JoinHandle<()> = tokio::task::spawn(inner_node.run(sender));
        self.task_handle = Some(node_task);
        debug!("Node task spawned successfully");

        // Spawn a task for the `SIGINT` handler.
        let sigint_task: JoinHandle<()> = {
            let stop_signal: Arc<RwLock<bool>> = self.stop_signal.clone();

            tokio::task::spawn(async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to initialize SIGINT handler");

                info!("Received SIGINT, initiating graceful shutdown");

                // Set `stop_signal` to trigger a graceful shutdown.
                *stop_signal.write().await = true;
            })
        };
        self.sigint_task = Some(sigint_task);

        Ok(())
    }

    /// Set the `stop_signal` to trigger a graceful shutdown.
    async fn stop(&self) {
        debug!("Setting the stop signal to true");
        *self.stop_signal.write().await = true;
    }

    /// Wait for the [`Node`] to finish its routines before shutting down.
    async fn wait_shutdown(mut self) -> Result<(), NodeError> {
        // Wait for up to `SHUTDOWN_TIMEOUT` seconds for the [`Node`]
        // to send a notification of shutdown completion through the `stop_receiver`.
        if let Some(receiver) = self.stop_receiver.take() {
            match tokio::time::timeout(Duration::from_secs(SHUTDOWN_TIMEOUT), receiver).await {
                Ok(Ok(())) => {
                    info!("Node signaled shutdown completion");
                }
                Ok(Err(_)) => {
                    warn!("Node shutdown channel closed without sending");
                }
                Err(_) => {
                    error!("Node shutdown notification timed out after {SHUTDOWN_TIMEOUT} seconds");
                }
            }
        }

        if let Some(task) = self.task_handle.take() {
            match tokio::time::timeout(Duration::from_secs(SHUTDOWN_TIMEOUT), task).await {
                Ok(Ok(_)) => {
                    info!("Node task completed successfully");
                }
                Ok(Err(e)) if e.is_panic() => {
                    error!("Node task panicked during shutdown: {:?}", e);
                    return Err(NodeError::Shutdown);
                }
                Ok(Err(e)) => {
                    error!("Node task failed: {:?}", e);
                    return Err(NodeError::Shutdown);
                }
                Err(_) => {
                    warn!("Node task join timed out after {SHUTDOWN_TIMEOUT} seconds");
                }
            }
        }

        // Flush the chainstate to disk.
        let _ = self.flush();

        // Stop the `SIGINT` listener task.
        if let Some(sigint) = self.sigint_task.take() {
            sigint.abort();
        }

        info!("Shutdown complete");
        Ok(())
    }

    /// Signal the [`Node`] to perform a graceful shutdown.
    pub async fn shutdown(self) -> Result<(), NodeError> {
        self.stop().await;
        self.wait_shutdown().await
    }

    /// Check if the [`Node`] should stop based on the `stop_signal`'s value.
    ///
    /// A separate thread should be created to continuously perform this check.
    pub async fn should_stop(&self) -> bool {
        *self.stop_signal.read().await
    }

    /// Flush the [`Node`]'s state to the file system.
    pub fn flush(&mut self) -> Result<(), NodeError> {
        debug!("Flushing state to disk...");
        self.chain_state.flush().map_err(|e| {
            error!("Failed to persist chain to disk: {:?}", e);
            match e {
                BlockchainError::Database(e) => NodeError::Persistence(Arc::new(e)),
                BlockchainError::Io(e) => NodeError::Io(Arc::new(e)),
                e => NodeError::Blockchain(Arc::new(e)),
            }
        })?;
        debug!("Successfully persisted chain state to disk");

        Ok(())
    }

    /// A subscriber for validated [`Block`]s.
    ///
    /// Implements the [`BlockConsumer`] trait from [`floresta-chain`](floresta_chain).
    pub fn block_subscriber<T: BlockConsumer + 'static>(&self, block_consumer: Arc<T>) {
        self.chain_state.subscribe(block_consumer);
    }

    /// Get the [`Node`]'s current [`NodeConfig`].
    pub async fn get_config(&self) -> Result<NodeConfig, NodeError> {
        let config = self.config.clone();
        Ok(config)
    }

    /// Connect to a specific peer, given a [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the connection was successfully established.
    pub async fn connect_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        match self
            .node_handle
            .add_peer(socket.ip(), socket.port(), self.config.allow_p2pv1_fallback)
            .await
        {
            Ok(true) => {
                debug!("Manual connection established with peer {socket:#?} successfully");
                Ok(true)
            }
            Ok(false) => {
                warn!("Failed to establish manual connection with peer {socket:#?}");
                Ok(false)
            }
            Err(e) => {
                error!("Network error while attempting to establish manual connection with peer {socket:#?}: {e}");
                Err(NodeError::Receiver(e))
            }
        }
    }

    /// Disconnect from a specific peer, given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the peer was successfully disconnected from.
    pub async fn disconnect_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        match self
            .node_handle
            .disconnect_peer(socket.ip(), socket.port())
            .await
        {
            Ok(true) => {
                debug!("Disconnected from peer {socket:#?}");
                Ok(true)
            }
            Ok(false) => {
                error!("Failed to disconnect from peer {socket:#?}");
                Ok(false)
            }
            Err(e) => {
                error!("Failed disconnect from peer {socket:#?}: {e}");
                Err(NodeError::Receiver(e))
            }
        }
    }

    /// Remove a specific peer's address from the
    /// [`AddressMan`](floresta_wire::address_man::AddressMan), given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the address was successfully removed.
    pub async fn remove_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        match self
            .node_handle
            .remove_peer(socket.ip(), socket.port())
            .await
        {
            Ok(true) => {
                debug!("Removed address {} from the address manager", socket);
                Ok(true)
            }
            Ok(false) => {
                error!(
                    "Failed to remove address {} from the address manager",
                    socket
                );
                Ok(false)
            }
            Err(e) => {
                error!(
                    "Failed to remove address {} from the address manager: {}",
                    socket, e
                );
                Err(NodeError::Receiver(e))
            }
        }
    }

    /// Get information about peers the [`Node`] is currently connected to.
    pub async fn get_peer_info(&self) -> Result<Vec<PeerInfo>, NodeError> {
        match self.node_handle.get_peer_info().await {
            Ok(peer_infos) => {
                debug!("Got peer information: {:?}", peer_infos);
                Ok(peer_infos)
            }
            Err(e) => {
                error!("Error whilst receiving peer information: {}", e);
                Err(NodeError::Receiver(e))
            }
        }
    }

    /// Ping all of the [`Node`]'s peers.
    pub async fn ping(&self) -> Result<bool, NodeError> {
        match self.node_handle.ping().await {
            Ok(true) => {
                debug!("Sent a ping to all peers");
                Ok(true)
            }
            Ok(false) => {
                warn!("Failed to send a ping to all peers");
                Ok(false)
            }
            Err(e) => {
                error!("Error whilst receiving ping response: {}", e);
                Err(NodeError::Receiver(e))
            }
        }
    }

    /// Broadcast a [`Transaction`] to the [`Node`]'s peers.
    ///
    /// Returns the [`Txid`], if the broadcast was successful.
    pub async fn broadcast_tx(&self, tx: Transaction) -> Result<Txid, NodeError> {
        match self.node_handle.broadcast_transaction(tx).await {
            Ok(Ok(txid)) => {
                info!("Broadcast transaction {}", txid);
                Ok(txid)
            }
            Ok(Err(e)) => {
                error!("Invalid transaction: {}", e);
                Err(NodeError::Mempool(e.into()))
            }
            Err(e) => {
                error!("Failed to broadcast transaction: {}", e);
                Err(NodeError::Receiver(e))
            }
        }
    }

    /// Check if the [`Node`] is still performing Initial Block Download.
    pub fn in_ibd(&self) -> bool {
        self.chain_state.is_in_ibd()
    }

    /// Get the blockchain's tip height.
    pub fn get_height(&self) -> Result<u32, NodeError> {
        let height = self.chain_state.get_height()?;
        Ok(height)
    }

    /// Get the [`Node`]'s validation height.
    pub fn get_validation_height(&self) -> Result<u32, NodeError> {
        let height = self.chain_state.get_validation_index()?;
        Ok(height)
    }

    /// Get the [`Node`]'s current accumulator as a [`Stump`].
    pub fn get_accumulator(&self) -> Result<Stump, NodeError> {
        let stump = self.chain_state.get_acc();
        Ok(stump)
    }

    /// Get the [`BlockHash`] associated with a height.
    pub fn get_blockhash(&self, height: u32) -> Result<BlockHash, NodeError> {
        let hash = self.chain_state.get_block_hash(height)?;

        Ok(hash)
    }

    /// Get a [`Block`] given its [`BlockHash`].
    ///
    /// Since [`floresta-chain`](floresta_chain) does not
    /// persist any [`Block`]s, these must be requested from a peer.
    pub async fn get_block(&self, blockhash: BlockHash) -> Result<Option<Block>, NodeError> {
        let block = self.node_handle.get_block(blockhash).await?;

        Ok(block)
    }
}
