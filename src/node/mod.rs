// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Node
//!
//! This module holds all the logic needed to interact with the [`Node`].

use core::fmt;
use core::net::SocketAddr;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use bitcoin::block::Header;
use bitcoin::Block;
use bitcoin::BlockHash;
use bitcoin::ScriptBuf;
use bitcoin::Transaction;
use bitcoin::Txid;
use builder::Builder;
use builder::NodeConfig;
use error::NodeError;
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
use futures::future;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tracing::info;
use tracing::warn;
#[cfg(feature = "logger")]
use tracing_appender::non_blocking::WorkerGuard;

use crate::node::fsm::compute_next_state;
use crate::node::fsm::State;

pub mod builder;
pub mod error;
pub mod fsm;

/// The period between polls for the `status_update_task`, in milliseconds.
const STATUS_UPDATE_POLL_PERIOD: Duration = Duration::from_millis(500);

/// The timeout for the [`Node`]'s shutdown task, in seconds.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// The set of [`Action`]s the [`Node`] can be performing at any given instant.
///
/// These [`Action`]s are triggered when a user calls [`Node`] methods, such as
/// scanning the blockchain with Compact Block Filters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// The [`Node`] is connecting to a peer with [`SocketAddr`].
    ConnectingToPeer(SocketAddr),

    /// The [`Node`] is disconnecting from a peer.
    DisconnectingFromPeer(SocketAddr),

    /// The [`Node`] is removing a peer from its [address manager](AddressMan).
    RemovingPeer(SocketAddr),

    /// The [`Node`] is pinging all of its peers.
    Pinging,

    /// The [`Node`] is fetching a [`Block`] from a peers.
    FetchingBlock(BlockHash),

    /// The [`Node`] is broadcasting a [`Transaction`].
    BroadcastingTransaction(Txid),

    /// The [`Node`] is matching [`ScriptBuf`] to [`BlockHash`]es.
    MatchingFilters,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectingToPeer(socket) => write!(f, "Connecting to peer={socket}"),
            Self::DisconnectingFromPeer(socket) => write!(f, "Disconnecting from peer={socket}"),
            Self::RemovingPeer(socket) => write!(f, "Removing peer={socket}"),
            Self::Pinging => write!(f, "Pinging all peers"),
            Self::FetchingBlock(hash) => write!(f, "Fetching block with hash={hash}"),
            Self::BroadcastingTransaction(txid) => {
                write!(f, "Broadcasting transaction with txid={txid}")
            }
            Self::MatchingFilters => write!(f, "Matching filters to block hashes"),
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
    pub(crate) inner: Mutex<Option<NodeInner>>,

    /// A handle used to interact with the [`UtreexoNode`].
    pub(crate) handle: NodeInterface,

    /// The [`Node`]'s configuration settings.
    pub(crate) config: NodeConfig,

    /// The [`Node`]'s current [`State`].
    pub(crate) state: Arc<watch::Sender<State>>,

    /// The [`Instant`] at which the [`Node`] was started.
    pub(crate) started_at: OnceLock<Instant>,

    /// The set of [`Action`]s that the [`Node`] is currently performing.
    pub(crate) action: Arc<watch::Sender<HashSet<Action>>>,

    /// A cancellation token used to signal shutdown to all tasks,
    /// Triggered via `SIGINT` or [`Node::shutdown()`].
    pub(crate) cancellation_token: CancellationToken,

    /// A kill signal shared with the inner [`UtreexoNode`],
    /// set to `true` to instruct it to stop processing and exit.
    pub(crate) kill_signal: Arc<RwLock<bool>>,

    /// A handle to the [`Node`]'s shutdown task,
    /// awaited by [`Node::shutdown`].
    pub(crate) shutdown_task: Mutex<Option<JoinHandle<()>>>,

    /// The [`Node`]'s blockchain state.
    pub(crate) chain_state: Arc<ChainState<FlatChainStore>>,

    /// The [`Node`]'s [Compact Block Filter](https://github.com/bitcoin/bips/blob/master/bip-0158.mediawiki) Store.
    pub(crate) block_filters: Arc<NetworkFilters<FlatFiltersStore>>,

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
                timeout.as_secs()
            ),
        }
    }

    /// Await the [`JoinHandle`] task for a timeout.
    async fn join_with_timeout(task: JoinHandle<()>, timeout: Duration) {
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => error!("Node task failed during shutdown: {e:?}"),
            Err(_) => warn!(
                "Node task join timed out after {} seconds",
                timeout.as_secs()
            ),
        }
    }

    /// Mark an [`Action`] as in-flight.
    fn track_action(&self, action: &Action) {
        self.action.send_modify(|set| {
            set.insert(action.clone());
        });
    }

    /// Mark an [`Action`] as no longer in-flight.
    fn untrack_action(&self, action: &Action) {
        self.action.send_modify(|set| {
            set.remove(action);
        });
    }

    // ----> CONTROL METHODS

    /// Spawn and run the [`Node`].
    ///
    /// This method will spawn three tasks:
    /// - A task that runs the inner [`UtreexoNode`].
    /// - A task that polls chain/filter state and updates the [`Node`]'s [`State`].
    /// - A task for the shutdown handler, which reacts to a [`Node::shutdown`] call or a `SIGINT`
    ///   signal, and performs the graceful shutdown routine.
    pub async fn run(&self) -> Result<(), NodeError> {
        // `take()` the inner node to make sure `Node::run()` can only be called once
        let inner_node = self
            .inner
            .lock()
            .await
            .take()
            .ok_or(NodeError::AlreadyRunning)?;

        // Register the `Instant` the node started running
        self.started_at
            .set(Instant::now())
            .map_err(|_| NodeError::AlreadyRunning)?;

        // Create a channel to carry shutdown notifications
        let (node_stopped_tx, node_stopped_rx) = oneshot::channel::<()>();

        // Spawn a task that will run the node
        let node_task: JoinHandle<()> = tokio::task::spawn(inner_node.run(node_stopped_tx));

        // Spawn a task that will update the node's state
        let state = self.state.clone();
        let cancellation_token = self.cancellation_token.clone();
        let node_handle = self.handle.clone();
        let chain_state = self.chain_state.clone();
        let block_filters = self.block_filters.clone();

        let state_update_task = tokio::task::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => break,
                    // Sleep for `STATUS_UPDATE_POLL_PERIOD` between polls
                    _ = tokio::time::sleep(STATUS_UPDATE_POLL_PERIOD) => {
                        // Check if the inner node is ready to receive requests
                        //
                        // This is a best-effort way to detect if it's ready
                        // To implement this with 100% certainty, FSM logic
                        // would need to be upstreamed to `floresta-wire`
                        let wire_ready = node_handle.get_peer_info().await
                            .map(|peers| !peers.is_empty())
                            .unwrap_or(false);

                        // Get the chain's tip
                        let chain_tip = match chain_state.get_best_block() {
                            Ok((tip, _)) => tip,
                            Err(e) => {
                                error!("Failed to compute FSM's next state (failed to get chain tip): {e}");
                                continue;
                            }
                        };

                        // Get the node's tip
                        let node_tip = match chain_state.get_validation_index() {
                            Ok(tip) => tip,
                            Err(e) => {
                                error!("Failed to compute FSM's next state (failed to get node tip): {e}");
                                continue;
                            }
                        };

                        // Get the filter's tip
                        let filter_tip = match block_filters.get_height() {
                            Ok(tip) => tip,
                            Err(e) => {
                                error!("Failed to compute FSM's next state (failed to get filter tip): {e}");
                                continue;
                            }
                        };

                        // Compute the next state and send it on state transitions
                        state.send_if_modified(|current_state| {
                            if matches!(current_state,  State::ShuttingDown) {
                                return false;
                            }
                            let next_state = compute_next_state(wire_ready, current_state, node_tip, chain_tip, filter_tip);
                            if *current_state != next_state {
                                *current_state = next_state;
                                true
                            } else {
                                false
                            }
                        });
                    }
                }
            }
        });

        // Spawn a task that listens for SIGINT or `Node::shutdown` and waits for tasks to complete
        let state = self.state.clone();
        let chain_state = self.chain_state.clone();
        let kill_signal = self.kill_signal.clone();
        let cancellation_token = self.cancellation_token.clone();

        let shutdown_task = tokio::task::spawn(async move {
            let reason = tokio::select! {
                _ = tokio::signal::ctrl_c() => "SIGINT",
                _ = cancellation_token.cancelled() => "SHUTDOWN request"
            };
            info!("Received {}, shutting the node down...", reason);

            *kill_signal.write().await = true;
            state.send_replace(State::ShuttingDown);

            // Cancel the node's cancellation token
            // This allows clients to react to it and shut themselves down
            cancellation_token.cancel();

            // Wait for the node's tasks to finish
            Self::await_with_timeout(node_stopped_rx, SHUTDOWN_TIMEOUT).await;
            Self::join_with_timeout(node_task, SHUTDOWN_TIMEOUT).await;
            Self::join_with_timeout(state_update_task, SHUTDOWN_TIMEOUT).await;

            // Flush the chain state to the file system
            if let Err(e) = chain_state.flush() {
                error!("Failed to flush chain state during shutdown: {e}");
            }

            state.send_replace(State::Inactive);

            info!("Node shutdown complete");
        });
        *self.shutdown_task.lock().await = Some(shutdown_task);

        // Set the node's state to active
        self.state.send_replace(State::Active);

        Ok(())
    }

    /// Flush the [`Node`]'s [chainstate](ChainState) to the file system.
    ///
    /// This is triggered automatically during shutdown,
    /// but can also be invoked while the [`Node`] is running.
    pub fn flush(&self) -> Result<(), NodeError> {
        self.chain_state.flush().map_err(NodeError::Blockchain)
    }

    /// Programatically request the [node](`Node`) to shutdown.
    pub async fn shutdown(&self) -> Result<(), NodeError> {
        self.cancellation_token.cancel();
        if let Some(task) = self.shutdown_task.lock().await.take() {
            let _ = task.await;
        }
        Ok(())
    }

    /// Returns a clone of the [`Node`]'s cancellation token.
    ///
    /// The returned token fires when the [`Node`] begins shutdown.
    /// This can be used by external tasks that want to react to
    /// the [`Node`]'s shutdown.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    // ----> LOCAL METHODS

    /// How [long](Duration) the [`Node`] has been running.
    pub fn uptime(&self) -> Result<Duration, NodeError> {
        self.started_at
            .get()
            .map(|t| t.elapsed())
            .ok_or(NodeError::NotRunning)
    }

    /// Subscribe to [`State`] transitions.
    ///
    /// Returns a [`watch::Receiver`] that yields the
    /// current [`State`] and wakes on every transition.
    pub fn subscribe_state(&self) -> watch::Receiver<State> {
        self.state.subscribe()
    }

    /// Get the [`Node`]'s current [`State`].
    pub fn get_state(&self) -> State {
        self.state.borrow().clone()
    }

    /// Subscribe to the [`Node`]'s in-flight [`Action`]s.
    ///
    /// Returns a [`watch::Receiver`] that yields the set
    /// of [`Action`]s currently being performed by the [`Node`].
    pub fn subscribe_action(&self) -> watch::Receiver<HashSet<Action>> {
        self.action.subscribe()
    }

    /// Get the [`Node`]'s currently in-flight [`Action`]s.
    pub fn get_actions(&self) -> HashSet<Action> {
        self.action.borrow().clone()
    }

    /// Get the [`Node`]'s current [`NodeConfig`].
    pub fn get_config(&self) -> NodeConfig {
        self.config.clone()
    }

    /// Check if the [`Node`] is still performing Initial Block Download.
    pub fn in_ibd(&self) -> bool {
        self.chain_state.is_in_ibd()
    }

    /// Get the [`Node`]'s header height.
    pub fn get_chain_height(&self) -> Result<u32, NodeError> {
        self.chain_state.get_height().map_err(NodeError::Blockchain)
    }

    /// Get the [`Node`]'s validation height.
    pub fn get_node_height(&self) -> Result<u32, NodeError> {
        self.chain_state
            .get_validation_index()
            .map_err(NodeError::Blockchain)
    }

    /// Get the [`Node`]'s [`NetworkFilters`] height.
    pub fn get_filter_height(&self) -> Result<u32, NodeError> {
        self.block_filters
            .get_height()
            .map_err(NodeError::CompactBlockFilter)
    }

    /// Get the [`Node`]'s current accumulator, as a [`Stump`].
    pub fn get_accumulator(&self) -> Stump {
        self.chain_state.get_acc()
    }

    /// Get the height of a [`Block`] given its [`BlockHash`].
    pub fn get_block_height(&self, hash: &BlockHash) -> Result<u32, NodeError> {
        self.chain_state
            .get_block_height(hash)
            .map_err(NodeError::Blockchain)?
            .ok_or(NodeError::MissingBlock(*hash))
    }

    /// Get the [`BlockHash`] of a [`Block`] at a given height.
    pub fn get_block_hash(&self, height: u32) -> Result<BlockHash, NodeError> {
        self.chain_state
            .get_block_hash(height)
            .map_err(NodeError::Blockchain)
    }

    /// Get the [`Header`] of a [`Block`], given its [`BlockHash`].
    pub fn get_block_header(&self, hash: &BlockHash) -> Result<Header, NodeError> {
        self.chain_state
            .get_block_header(hash)
            .map_err(NodeError::Blockchain)
    }

    /// Get information about peers the [`Node`] is currently connected to.
    pub async fn get_peer_info(&self) -> Result<Vec<PeerInfo>, NodeError> {
        self.handle
            .get_peer_info()
            .await
            .map_err(NodeError::UnresponsiveNode)
    }

    /// Subscribe and receive every new [`Block`] the [`Node`] validates.
    ///
    /// Implements the [`BlockConsumer`] trait from [`floresta-chain`](floresta_chain).
    pub fn block_subscriber<T: BlockConsumer + 'static>(&self, block_consumer: Arc<T>) {
        self.chain_state.subscribe(block_consumer);
    }

    /// Find [`Block`]s that contain relevant [`Transaction`]s
    /// to a given set of [`ScriptBuf`].
    ///
    /// Returns an error if the Compact Block Filters are not yet downloaded.
    pub async fn match_filters(
        &self,
        spks: Vec<ScriptBuf>,
        start_height: u32,
        stop_height: u32,
    ) -> Result<Vec<BlockHash>, NodeError> {
        // Throw an error if the node is not yet operational
        let last_state = self.state.borrow().clone();
        if !matches!(last_state, State::Operational) {
            return Err(NodeError::IllegalAction {
                state: last_state,
                attempted: Action::MatchingFilters,
            });
        }

        // Get the filter tip height from the node
        let filter_tip = self
            .block_filters
            .get_height()
            .map_err(NodeError::CompactBlockFilter)?;

        // Can't perform CBF scans past the filter tip height
        if stop_height > filter_tip {
            return Err(NodeError::StopHeightExceedsFilterTip {
                requested: stop_height,
                available: filter_tip,
            });
        }

        let action = Action::MatchingFilters;
        self.track_action(&action);

        // Find blocks with relevant transactions to these spks
        let spks: Vec<&[u8]> = spks.iter().map(|s| s.as_bytes()).collect();
        let result = self
            .block_filters
            .match_any(
                spks,
                Some(start_height),
                Some(stop_height),
                self.chain_state.clone(),
            )
            .map_err(NodeError::CompactBlockFilter);
        self.untrack_action(&action);

        result
    }

    // ----> P2P NETWORK METHODS

    /// Fetch a [`Block`] given its [`BlockHash`].
    ///
    /// Since [`floresta-chain`](floresta_chain) does not persist any
    /// [`Block`]s, it must be requested over the wire from a peer.
    pub async fn fetch_block(&self, hash: BlockHash) -> Result<Block, NodeError> {
        let action = Action::FetchingBlock(hash);
        self.track_action(&action);
        let result = self
            .handle
            .get_block(hash)
            .await
            .map_err(NodeError::UnresponsiveNode)
            .and_then(|opt| opt.ok_or(NodeError::MissingBlock(hash)));
        self.untrack_action(&action);

        result
    }

    /// Fetch a number [`Block`]s in a batch, given their [`BlockHash`]es.
    ///
    /// The [`Block`]s are returned in the same ordering of the provided hashes.
    ///
    /// Since [`floresta-chain`](floresta_chain) does not persist any
    /// [`Block`]s, they must be requested over the wire from a peer.
    pub async fn fetch_blocks(&self, hashes: &[BlockHash]) -> Result<Vec<Block>, NodeError> {
        let actions: Vec<Action> = hashes.iter().map(|h| Action::FetchingBlock(*h)).collect();
        for action in &actions {
            self.track_action(action);
        }

        let result = future::try_join_all(hashes.iter().map(|hash| {
            let handle = self.handle.clone();
            let hash = *hash;
            async move {
                handle
                    .get_block(hash)
                    .await
                    .map_err(NodeError::UnresponsiveNode)?
                    .ok_or(NodeError::MissingBlock(hash))
            }
        }))
        .await;

        for action in &actions {
            self.untrack_action(action);
        }

        result
    }

    /// Broadcast a [`Transaction`] to the [`Node`]'s peers.
    ///
    /// Returns the [`Txid`], if the broadcast was successful.
    pub async fn broadcast_tx(&self, tx: Transaction) -> Result<Txid, NodeError> {
        let txid = tx.compute_txid();
        let action = Action::BroadcastingTransaction(txid);
        self.track_action(&action);
        let result = self
            .handle
            .broadcast_transaction(tx)
            .await
            .map_err(NodeError::UnresponsiveNode)
            .and_then(|inner| inner.map_err(NodeError::Mempool));
        self.untrack_action(&action);

        result
    }

    /// Manually connect to a specific peer, given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the connection was successful.
    pub async fn add_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        let action = Action::ConnectingToPeer(*socket);
        self.track_action(&action);
        let result = self
            .handle
            .add_peer(socket.ip(), socket.port(), self.config.allow_p2pv1_fallback)
            .await
            .map_err(NodeError::UnresponsiveNode);
        self.untrack_action(&action);

        result
    }

    /// Disconnect from a peer, given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the [`Node`] successfully disconnected from the peer.
    pub async fn disconnect_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        let action = Action::DisconnectingFromPeer(*socket);
        self.track_action(&action);
        let result = self
            .handle
            .disconnect_peer(socket.ip(), socket.port())
            .await
            .map_err(NodeError::UnresponsiveNode);
        self.untrack_action(&action);

        result
    }

    /// Remove a specific peer's address from the [`Node`]'s
    /// [`AddressMan`], given its [`SocketAddr`].
    ///
    /// Returns a `bool` indicating whether the address was successfully removed.
    pub async fn remove_peer(&self, socket: &SocketAddr) -> Result<bool, NodeError> {
        let action = Action::RemovingPeer(*socket);
        self.track_action(&action);
        let result = self
            .handle
            .remove_peer(socket.ip(), socket.port())
            .await
            .map_err(NodeError::UnresponsiveNode);
        self.untrack_action(&action);

        result
    }

    /// Send a `ping` to all of the [`Node`]'s peers.
    pub async fn ping(&self) -> Result<bool, NodeError> {
        let action = Action::Pinging;
        self.track_action(&action);
        let result = self
            .handle
            .ping()
            .await
            .map_err(NodeError::UnresponsiveNode);
        self.untrack_action(&action);

        result
    }
}
