// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Node
//!
//! This module holds all the logic needed to interact with the [`Node`].

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
use floresta_wire::rustreexo::stump::Stump;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;
#[cfg(feature = "logger")]
use tracing_appender::non_blocking::WorkerGuard;

use crate::builder::NodeConfig;
use crate::error::NodeError;
use crate::updater::WalletUpdate;

/// Timeout for the [`Node`]'s shutdown task, in seconds.
const SHUTDOWN_TIMEOUT: u64 = 15;

/// Type alias for the [`Node`]'s inner [`UtreexoNode`].
type NodeInner = UtreexoNode<Arc<ChainState<FlatChainStore>>, RunningNode>;

/// The embedded and fully-validating Compact State [`Node`].
pub struct Node {
    /// The [`Node`]'s configuration settings.
    pub(crate) config: NodeConfig,
    /// The inner, underlying [`UtreexoNode`].
    pub(crate) node_inner: Option<NodeInner>,
    /// The [`Node`]'s blockchain state.
    pub(crate) chain_state: Arc<ChainState<FlatChainStore>>,
    /// A handle used to interact with the [`UtreexoNode`].
    pub(crate) node_handle: NodeInterface,
    /// A cancellation token used to signal shutdown to all tasks,
    /// Triggered via `SIGINT` or [`Node::shutdown()`].
    pub(crate) cancellation_token: CancellationToken,
    /// A kill signal shared with the inner [`UtreexoNode`],
    /// set to `true` to instruct it to stop processing and exit.
    pub(crate) kill_signal: Arc<RwLock<bool>>,
    /// Handle to the background task that drives graceful shutdown:
    /// sets the kill signal, waits for the inner node to stop, and flushes
    /// the chain state to disk. Awaited by [`Node::shutdown()`] and [`Node::cancelled()`].
    pub(crate) shutdown_task: Option<JoinHandle<()>>,
    /// A [`Wallet`] that will receive updates from the [`Node`].
    pub wallet: Option<Arc<RwLock<Wallet>>>,
    /// Receiver for [`WalletUpdate`]s that come from
    /// the [`Node`] and should be applied to the [`Wallet`].
    pub update_subscriber: Option<UnboundedReceiver<WalletUpdate>>,
    /// A guard that ensures the logger remains active for the [`Node`]'s lifetime.
    #[cfg(feature = "logger")]
    pub(crate) _log_guard: Option<WorkerGuard>,
}

impl Node {
    /// Spawn and run the [`Node`].
    ///
    /// This method will spawn a task for the inner [`UtreexoNode`] and the shutdown handler task.
    pub async fn run(&mut self) -> Result<(), NodeError> {
        // Take the inner node to make sure `Node::run()` can only be called once.
        let inner_node = self.node_inner.take().ok_or(NodeError::AlreadyRunning)?;

        // Create a channel to carry shutdown notifications.
        let (node_stopped_tx, node_stopped_rx) = oneshot::channel::<()>();
        // Spawn a task which will run the node.
        let node_task: JoinHandle<()> = tokio::task::spawn(inner_node.run(node_stopped_tx));

        let cancellation_token = self.cancellation_token.clone();
        let kill_signal = self.kill_signal.clone();
        let chain_state = self.chain_state.clone();

        // Spawn a task that listens for SIGINT or `Node::shutdown` and waits for tasks to complete
        let shutdown_task = tokio::task::spawn(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => { info!("Shutting down"); }
                _ = cancellation_token.cancelled() => { info!("Shutting down"); }
            }
            *kill_signal.write().await = true;

            let timeout = Duration::from_secs(SHUTDOWN_TIMEOUT);
            Self::await_with_timeout(node_stopped_rx, timeout).await;
            Self::join_with_timeout(node_task, timeout).await;

            if let Err(e) = chain_state.flush() {
                error!("Error flushing chain state to the file system: {e:?}");
            }

            cancellation_token.cancel();
            info!("Shutdown complete");
        });

        self.shutdown_task = Some(shutdown_task);
        Ok(())
    }

    /// Await the [`oneshot::Receiver`] for a set timeout.
    async fn await_with_timeout(rx: oneshot::Receiver<()>, timeout: Duration) {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => warn!("Shutdown channel closed without sending"),
            Err(_) => error!("Shutdown channel timed out after {SHUTDOWN_TIMEOUT} seconds"),
        }
    }

    /// Await the [`JoinHandle`] task for a timeout.
    async fn join_with_timeout(task: JoinHandle<()>, timeout: Duration) {
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => error!("Node task failed during shutdown: {e:?}"),
            Err(_) => warn!("Node task join timed out after {SHUTDOWN_TIMEOUT} seconds"),
        }
    }

    /// Programatically request the [node](`Node`) to shutdown.
    pub async fn shutdown(&mut self) -> Result<(), NodeError> {
        self.cancellation_token.cancel();
        if let Some(task) = self.shutdown_task.take() {
            let _ = task.await;
        }
        Ok(())
    }

    /// Suspend the caller until the [`Node`] has completed shutdown.
    ///
    /// Shutdown must be triggered externally, either by
    /// sending `SIGINT` or by calling [`Node::shutdown()`].
    pub async fn cancelled(&mut self) {
        if let Some(task) = self.shutdown_task.take() {
            let _ = task.await;
        }
    }

    /// Flush the [`Node`]'s [chainstate](ChainState) to the file system.
    pub fn flush(&mut self) -> Result<(), NodeError> {
        self.chain_state.flush().map_err(|e| {
            error!("Error flushing chain state to the file system: {e:?}");
            match e {
                BlockchainError::Database(e) => NodeError::Persistence(Arc::new(e)),
                BlockchainError::Io(e) => NodeError::Io(Arc::new(e)),
                e => NodeError::Blockchain(Arc::new(e)),
            }
        })?;
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

    /// Check if the [`Node`] is still performing Initial Block Download.
    pub fn in_ibd(&self) -> bool {
        self.chain_state.is_in_ibd()
    }

    /// Get the height of the blockchain tip.
    pub fn get_height(&self) -> Result<u32, NodeError> {
        let height = self.chain_state.get_height()?;
        Ok(height)
    }

    /// Get the [`Node`]'s validation height.
    pub fn get_validation_height(&self) -> Result<u32, NodeError> {
        let height = self.chain_state.get_validation_index()?;
        Ok(height)
    }

    /// Get the [`Node`]'s current accumulator, as a [`Stump`].
    pub fn get_accumulator(&self) -> Result<Stump, NodeError> {
        let stump = self.chain_state.get_acc();
        Ok(stump)
    }

    /// Get the [`BlockHash`] associated with a height.
    pub fn get_block_hash(&self, height: u32) -> Result<BlockHash, NodeError> {
        let hash = self.chain_state.get_block_hash(height)?;

        Ok(hash)
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

    /// Manually connect to a specific peer, given a [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the connection was successfully established.
    pub async fn add_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
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

    /// Fetch a [`Block`] given its [`BlockHash`].
    ///
    /// Since [`floresta-chain`](floresta_chain) does not persist any
    /// [`Block`]s, these must be requested over the wire from a peer.
    pub async fn fetch_block(&self, blockhash: BlockHash) -> Result<Option<Block>, NodeError> {
        let block = self.node_handle.get_block(blockhash).await?;

        Ok(block)
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
}
