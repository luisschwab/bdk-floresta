// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Node
//!
//! This module holds all the logic needed to interact with the [`Node`].

use core::fmt;
use core::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bdk_wallet::Wallet;
use bitcoin::block::Header;
use bitcoin::Block;
use bitcoin::BlockHash;
use bitcoin::Transaction;
use bitcoin::Txid;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStore;
use floresta_chain::pruned_utreexo::BlockchainInterface;
use floresta_chain::pruned_utreexo::UpdatableChainstate;
use floresta_chain::BlockConsumer;
use floresta_chain::ChainState;
use floresta_compact_filters::flat_filters_store::FlatFiltersStore;
use floresta_compact_filters::network_filters::NetworkFilters;
#[allow(unused)]
use floresta_wire::address_man::AddressMan;
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
use crate::fsm::compute_next_state;
use crate::fsm::State;
use crate::updater::WalletUpdate;

/// A conservative value for the maximum chain reorganization depth.
const MAX_REORG_DEPTH: u8 = 7;

/// The period between polls for the `status_update_task`, in milliseconds.
const STATUS_UPDATE_POLL_PERIOD: Duration = Duration::from_millis(500);

/// The timeout for the [`Node`]'s shutdown task, in seconds.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// The set of [`Action`]s the [`Node`] can
/// possibly be performing at any given instant.
///
/// These [`Action`]s are triggered when a user calls
/// [`Node`] methods (e.g. scaning the blockchain with
/// Compact Block Filters).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The [`Node`] is connecting to a peer.
    ConnectingToPeer(String),
    /// The [`Node`] is disconnecting from a peer.
    DisconnectingFromPeer(String),
    /// The [`Node`] is removing a peer from its [address manager](AddressMan).
    RemovingPeer(String),
    /// The [`Node`] is pinging all of its peers.
    Pinging,
    /// The [`Node`] is fetching a [`Block`] from a peers.
    FetchingBlock(String),
    /// The [`Node`] is scanning the blockchain with Compact Block Filters.
    CompactFilterScan((u32, u32)),
    /// The [`Node`] is broadcasting a [`Transaction`].
    BroadcastingTransaction(String),
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectingToPeer(socket) => write!(f, "Connecting to peer={}", socket),
            Self::DisconnectingFromPeer(socket) => write!(f, "Disconnecting from peer={}", socket),
            Self::RemovingPeer(socket) => write!(f, "Removing peer={}", socket),
            Self::Pinging => write!(f, "Pinging all peers"),
            Self::FetchingBlock(hash) => write!(f, "Fetching block with hash={}", hash),
            Self::CompactFilterScan((start_height, end_height)) => write!(f, "Scanning the blockchain with Compact Block Filters from start_height={} up to end_height={}", start_height, end_height),
            Self::BroadcastingTransaction(txid) => write!(f, "Broadcasting transaction with txid={}", txid),
        }
    }
}

/// An alias to the [`Node`]'s inner: [`UtreexoNode`].
type NodeInner = UtreexoNode<Arc<ChainState<FlatChainStore>>, RunningNode>;

/// The [`Node`].
///
/// It groups all of the required pieces for the [`Node`] to function.
pub struct Node {
    /// The inner, underlying [`UtreexoNode`].
    pub(crate) inner: Option<NodeInner>,

    /// A handle used to interact with the [`UtreexoNode`].
    pub(crate) handle: NodeInterface,

    /// The [`Node`]'s configuration settings.
    pub(crate) config: NodeConfig,

    /// The [`Node`]'s current [`State`].
    pub(crate) state: Arc<RwLock<State>>,

    /// A cancellation token used to signal shutdown to all tasks,
    /// Triggered via `SIGINT` or [`Node::shutdown()`].
    pub(crate) cancellation_token: CancellationToken,

    /// A kill signal shared with the inner [`UtreexoNode`],
    /// set to `true` to instruct it to stop processing and exit.
    pub(crate) kill_signal: Arc<RwLock<bool>>,

    /// A handle to the shutdown task.
    ///
    /// It sets the kill signal, waits for the [`NodeInner`] to
    /// stop, and flushes the [`ChainState`] to the file system.
    /// Awaited by [`Node::shutdown()`] and [`Node::cancelled()`].
    pub(crate) shutdown_task: Option<JoinHandle<()>>,

    /// A handle to the status update task.
    ///
    /// This task will update the [`Node`]'s [`State`]
    /// by polling the [`ChainState`] and interpreting
    /// the values returned into a [`State`].
    pub(crate) state_update_task: Option<JoinHandle<()>>,

    /// The [`Node`]'s blockchain state.
    pub(crate) chain_state: Arc<ChainState<FlatChainStore>>,

    /// The [`Node`]'s [Compact Block Filter](https://github.com/bitcoin/bips/blob/master/bip-0158.mediawiki) Store.
    pub(crate) block_filters: Arc<NetworkFilters<FlatFiltersStore>>,

    /// Receiver for [`WalletUpdate`]s that come from
    /// the [`Node`] and should be applied to the [`Wallet`].
    pub update_subscriber: Option<UnboundedReceiver<WalletUpdate>>,

    /// A [`Wallet`] that will receive updates from the [`Node`].
    pub wallet: Option<Arc<RwLock<Wallet>>>,

    /// A guard that ensures the logger remains active for the [`Node`]'s lifetime.
    #[cfg(feature = "logger")]
    pub(crate) _log_guard: Option<WorkerGuard>,
}

impl Node {
    // ----> INTERNAL METHODS

    /// Await the [`oneshot::Receiver`] for a set timeout.
    async fn await_with_timeout(rx: oneshot::Receiver<()>, timeout: Duration) {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => warn!("Shutdown channel closed without sending"),
            Err(_) => error!(
                "Shutdown channel timed out after {} seconds",
                SHUTDOWN_TIMEOUT.as_secs()
            ),
        }
    }

    /// Await the [`JoinHandle`] task for a timeout.
    async fn join_with_timeout(task: JoinHandle<()>, timeout: Duration) {
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => error!("Node task failed during shutdown: {:?}", e),
            Err(_) => warn!(
                "Node task join timed out after {} seconds",
                SHUTDOWN_TIMEOUT.as_secs()
            ),
        }
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

    // ----> CONTROL METHODS

    /// Spawn and run the [`Node`].
    ///
    /// This method will spawn two tasks:
    /// - A task that runs the inner [`UtreexoNode`].
    /// - A task for the shutdown handler, which reacts to a [`Node::shutdown`] call or a `SIGINT`
    ///   signal, and perfoms the graceful shutdown routine.
    pub async fn run(&mut self) -> Result<(), NodeError> {
        // `take()` the inner node to make sure `Node::run()` can only be called once
        let inner_node = self.inner.take().ok_or(NodeError::AlreadyRunning)?;

        // Set the node's state to `State::Active`
        *self.state.write().await = State::Active;

        // Create a channel to carry shutdown notifications
        let (node_stopped_tx, node_stopped_rx) = oneshot::channel::<()>();

        // Spawn a task that will run the node
        let node_task: JoinHandle<()> = tokio::task::spawn(inner_node.run(node_stopped_tx));

        // Spawn a task that will update the node's state
        let state = self.state.clone();
        let cancellation_token = self.cancellation_token.clone();
        let kill_signal = self.kill_signal.clone();
        let chain_state = self.chain_state.clone();

        let state_update_task = tokio::task::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => break,
                    // Sleep for `STATUS_UPDATE_POLL_PERIOD` between polls
                    _ = tokio::time::sleep(STATUS_UPDATE_POLL_PERIOD) => {
                        let node_tip = chain_state.get_validation_index();
                        let Ok(node_tip) = node_tip else {
                            error!("Failed to compute FSM's next state (failed to get node tip): {}", node_tip.unwrap_err());
                            continue
                        };

                        let chain_tip = chain_state.get_best_block();
                        let Ok((chain_tip, _)) = chain_tip else {
                            error!("Failed to compute FSM's next state (failed to get chain tip): {}", chain_tip.unwrap_err());
                            continue
                        };
                        let current_state = state.read().await.clone();

                        if !matches!(current_state, State::PerformingAction(_) | State::ShuttingDown) {
                            *state.write().await = compute_next_state(current_state, node_tip, chain_tip);
                        }
                    }
                }
            }
        });
        self.state_update_task = Some(state_update_task);

        // Spawn a task that listens for SIGINT or `Node::shutdown` and waits for tasks to complete
        let state = self.state.clone();
        let cancellation_token = self.cancellation_token.clone();
        let chain_state = self.chain_state.clone();

        let shutdown_task = tokio::task::spawn(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => { info!("Shutting down"); }
                _ = cancellation_token.cancelled() => { info!("Shutting down"); }
            }
            // Set the node's state to `State::ShuttingDown`
            *state.write().await = State::ShuttingDown;

            *kill_signal.write().await = true;

            Self::await_with_timeout(node_stopped_rx, SHUTDOWN_TIMEOUT).await;
            Self::join_with_timeout(node_task, SHUTDOWN_TIMEOUT).await;

            if let Err(e) = chain_state.flush() {
                error!("Error flushing chain state to the file system: {e:?}");
            }

            cancellation_token.cancel();

            // Set the node's state to `State::Inactive`
            *state.write().await = State::Inactive;

            info!("Shutdown complete");
        });
        self.shutdown_task = Some(shutdown_task);

        Ok(())
    }

    /// Programatically request the [node](`Node`) to shutdown.
    pub async fn shutdown(&mut self) -> Result<(), NodeError> {
        self.cancellation_token.cancel();
        if let Some(task) = self.shutdown_task.take() {
            let _ = task.await;
        }
        Ok(())
    }

    /// Flush the [`Node`]'s [chainstate](ChainState) to the file system.
    pub fn flush(&mut self) -> Result<(), NodeError> {
        self.chain_state.flush().map_err(|e| {
            error!("Error flushing chain state to the file system: {e:?}");
            NodeError::Flush(e)
        })
    }

    // ----> LOCAL METHODS

    /// Get the [`Node`]'s current [`State`].
    pub async fn get_state(&self) -> State {
        self.state.read().await.clone()
    }

    /// Get the [`Node`]'s current [`NodeConfig`].
    pub fn get_config(&self) -> NodeConfig {
        self.config.clone()
    }

    /// Check if the [`Node`] is still performing Initial Block Download.
    pub fn in_ibd(&self) -> bool {
        self.chain_state.is_in_ibd()
    }

    /// Get the height of the blockchain tip.
    pub fn get_chain_height(&self) -> Result<u32, NodeError> {
        self.chain_state.get_validation_index().map_err(Into::into)
    }

    /// Get the [`Node`]'s validation height.
    pub fn get_node_height(&self) -> Result<u32, NodeError> {
        self.chain_state.get_validation_index().map_err(Into::into)
    }

    /// Get the [`Node`]'s current accumulator, as a [`Stump`].
    pub fn get_accumulator(&self) -> Stump {
        self.chain_state.get_acc()
    }

    /// Get the [`BlockHash`] of a [`Block`] at a given height.
    pub fn get_block_hash(&self, height: u32) -> Result<BlockHash, NodeError> {
        self.chain_state.get_block_hash(height).map_err(Into::into)
    }

    /// Get the [`Header`] of a [`Block`], given its [`BlockHash`].
    pub fn get_block_header(&self, hash: &BlockHash) -> Result<Header, NodeError> {
        self.chain_state.get_block_header(hash).map_err(Into::into)
    }

    /// A subscriber for validated [`Block`]s.
    ///
    /// Implements the [`BlockConsumer`] trait from [`floresta-chain`](floresta_chain).
    pub fn block_subscriber<T: BlockConsumer + 'static>(&self, block_consumer: Arc<T>) {
        self.chain_state.subscribe(block_consumer);
    }

    /// Get information about peers the [`Node`] is currently connected to.
    pub async fn get_peer_info(&self) -> Result<Vec<PeerInfo>, NodeError> {
        match self.handle.get_peer_info().await {
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

    // ----> NETWORK METHODS

    /// Fetch a [`Block`] given its [`BlockHash`].
    ///
    /// Since [`floresta-chain`](floresta_chain) does not persist any
    /// [`Block`]s, it must be requested over the wire from a peer.
    pub async fn fetch_block(&self, hash: BlockHash) -> Result<Block, NodeError> {
        let last_state = self.state.read().await.clone();
        *self.state.write().await =
            State::PerformingAction(Action::FetchingBlock(hash.to_string()));

        let block = self.handle.get_block(hash).await?;
        if let Some(block) = block {
            *self.state.write().await = last_state;
            Ok(block)
        } else {
            *self.state.write().await = last_state;
            Err(NodeError::MissingBlock(hash))
        }
    }

    /// Fetch a number [`Block`]s in a batch, given their [`BlockHash`]es.
    ///
    /// Since [`floresta-chain`](floresta_chain) does not persist any
    /// [`Block`]s, they must be requested over the wire from a peer.
    pub async fn fetch_blocks(&self, hashes: Vec<BlockHash>) -> Result<Vec<Block>, NodeError> {
        let last_state = self.state.read().await.clone();

        let mut blocks: Vec<Block> = Vec::with_capacity(hashes.len());
        for hash in hashes {
            *self.state.write().await =
                State::PerformingAction(Action::FetchingBlock(hash.to_string()));

            let block = self.handle.get_block(hash).await?;
            if let Some(block) = block {
                blocks.push(block);
            } else {
                return Err(NodeError::MissingBlock(hash));
            }
        }
        *self.state.write().await = last_state;

        Ok(blocks)
    }

    /// Broadcast a [`Transaction`] to the [`Node`]'s peers.
    ///
    /// Returns the [`Txid`], if the broadcast was successful.
    pub async fn broadcast_tx(&self, tx: Transaction) -> Result<Txid, NodeError> {
        let txid = tx.compute_txid();

        let last_state = self.state.read().await.clone();
        *self.state.write().await =
            State::PerformingAction(Action::BroadcastingTransaction(txid.to_string()));

        let result = match self.handle.broadcast_transaction(tx).await {
            Ok(Ok(txid)) => {
                info!("Successfully broadcast transaction with txid={}", txid);
                Ok(txid)
            }
            Ok(Err(e)) => {
                error!(
                    "Failed to broadcast an invalid transaction with txid={}: {}",
                    txid, e
                );
                Err(NodeError::Mempool(e))
            }
            Err(e) => {
                error!("Failed to broadcast transaction with txid={}: {}", txid, e);
                Err(NodeError::Receiver(e))
            }
        };
        *self.state.write().await = last_state;

        result
    }

    /// Manually connect to a specific peer, given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the connection was successful.
    pub async fn add_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        let last_state = self.state.read().await.clone();
        *self.state.write().await =
            State::PerformingAction(Action::ConnectingToPeer(socket.to_string()));

        let result = match self
            .handle
            .add_peer(socket.ip(), socket.port(), self.config.allow_p2pv1_fallback)
            .await
        {
            Ok(true) => {
                debug!("Connected to peer={}", socket);
                Ok(true)
            }
            Ok(false) => {
                warn!("Failed to connect to peer={}", socket);
                Ok(false)
            }
            Err(e) => {
                error!("Error whilst connecting to peer={}: {}", socket, e);
                Err(NodeError::Receiver(e))
            }
        };
        *self.state.write().await = last_state;

        result
    }

    /// Disconnect from a peer, given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the [`Node`] successfully disconnected from the peer.
    pub async fn disconnect_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        let last_state = self.state.read().await.clone();
        *self.state.write().await =
            State::PerformingAction(Action::DisconnectingFromPeer(socket.to_string()));

        let result = match self
            .handle
            .disconnect_peer(socket.ip(), socket.port())
            .await
        {
            Ok(true) => {
                debug!("Disconnected from peer={socket}");
                Ok(true)
            }
            Ok(false) => {
                error!("Failed to disconnect from peer={socket}");
                Ok(false)
            }
            Err(e) => {
                error!("Error whilst disconnecting from peer={socket}: {e}");
                Err(NodeError::Receiver(e))
            }
        };
        *self.state.write().await = last_state;

        result
    }

    /// Remove a specific peer's address from the [`Node`]'s
    /// (address manager)[AddressMan], given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the address was successfully removed.
    pub async fn remove_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        let last_state = self.state.read().await.clone();
        *self.state.write().await =
            State::PerformingAction(Action::RemovingPeer(socket.to_string()));

        let result = match self.handle.remove_peer(socket.ip(), socket.port()).await {
            Ok(true) => {
                debug!("Removed peer={} from the address manager", socket);
                Ok(true)
            }
            Ok(false) => {
                error!("Failed to remove peer={} from the address manager", socket);
                Ok(false)
            }
            Err(e) => {
                error!(
                    "Failed to remove peer={} from the address manager: {}",
                    socket, e
                );
                Err(NodeError::Receiver(e))
            }
        };
        *self.state.write().await = last_state;

        result
    }

    /// Send a `ping` to all of the [`Node`]'s peers.
    pub async fn ping(&self) -> Result<bool, NodeError> {
        let last_state = self.state.read().await.clone();
        *self.state.write().await = State::PerformingAction(Action::Pinging);

        let result = match self.handle.ping().await {
            Ok(true) => {
                debug!("Sent a ping to all peers");
                Ok(true)
            }
            Ok(false) => {
                warn!("Failed to send a ping to all peers");
                Ok(false)
            }
            Err(e) => {
                error!("Error whilst receiving ping response: {e}");
                Err(NodeError::Receiver(e))
            }
        };
        *self.state.write().await = last_state;

        result
    }
}
