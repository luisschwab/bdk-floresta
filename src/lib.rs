// SPDX-Licence-Identifier: MIT

#![doc = include_str!("../README.md")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bdk_wallet::Wallet;
use bitcoin::Block;
use bitcoin::BlockHash;
use error::NodeError;
pub use floresta_chain::pruned_utreexo::chainparams::AssumeUtreexoValue;
pub use floresta_chain::pruned_utreexo::chainparams::ChainParams;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStore;
pub(crate) use floresta_chain::pruned_utreexo::BlockchainInterface;
pub(crate) use floresta_chain::pruned_utreexo::UpdatableChainstate;
pub use floresta_chain::BlockConsumer;
use floresta_chain::BlockchainError;
use floresta_chain::ChainState;
pub use floresta_chain::UtxoData;
pub use floresta_wire::node::ConnectionKind;
pub use floresta_wire::node::PeerStatus;
use floresta_wire::node_interface::NodeInterface;
pub use floresta_wire::node_interface::PeerInfo;
pub use floresta_wire::rustreexo;
use floresta_wire::rustreexo::accumulator::stump::Stump;
pub use floresta_wire::TransportProtocol;
pub use floresta_wire::UtreexoNodeConfig;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::trace;
use tracing::warn;
use tracing_appender::non_blocking::WorkerGuard;
pub use updater::WalletUpdate;

pub mod builder;
pub mod error;
mod logger;
mod updater;

const SHUTDOWN_TIMEOUT: u64 = 60;

/// The [`Node`] represents the embedded and
/// fully validating Compact State Node.
pub struct Node {
    /// Configuration parameters for [`Node`].
    pub(crate) _node_config: UtreexoNodeConfig,
    /// Whether to set the log level to debug.
    pub(crate) _debug: bool,
    /// The node's chain state.
    /// TODO(@luisschwab): couple chainstate persistence with
    /// `bdk_chain::ChangeSet` (bdk#1582). Implement a custom `Anchor` with
    /// the inclusion proofs we care about.
    pub(crate) chain_state: Arc<ChainState<FlatChainStore>>,
    /// The `node_handle` is used to send requests and receive responses to the
    /// underlying node.
    pub(crate) node_handle: NodeInterface,
    /// The `task_handle` is a handle for the undelying node's tasks.
    pub(crate) task_handle: Option<JoinHandle<()>>,
    /// The `sigint_task` listens for interrupt signals and sets
    /// `shutdown_signal` to true, signalling [`Node`] for a gracefull
    /// shutdown.
    pub(crate) sigint_task: Option<JoinHandle<()>>,
    /// The `stop_signal` is continuously checked by [`Node`].
    /// If set, a gracefull shutdown will be initiated.
    pub(crate) stop_signal: Arc<RwLock<bool>>,
    /// A receiver for the stop notification from the `node_task`.
    pub(crate) stop_receiver: Option<oneshot::Receiver<()>>,
    /// A guard for the logger thread. Must be kept alive for the lifetime of
    /// [`FlorestaNode`].
    pub(crate) _logger_guard: Option<WorkerGuard>,
    /// The `Wallet` to be coupled to the [`Node`].
    pub wallet: Option<Arc<RwLock<Wallet>>>,
    /// The `update_subscriber` emits relevant wallet events that can be
    /// applied to the `Wallet`.
    pub update_subscriber: Option<UnboundedReceiver<WalletUpdate>>,
}

impl Node {
    /// Set `stop_signal` to `true` to initiate a graceful shutdown.
    async fn stop(&self) {
        info!("Setting the stop signal to true");
        *self.stop_signal.write().await = true;
    }

    /// Wait for the node to finish it's tasks.
    async fn wait_shutdown(mut self) -> Result<(), NodeError> {
        info!("Waiting for shutdown to complete");

        if let Some(receiver) = self.stop_receiver.take() {
            match tokio::time::timeout(
                Duration::from_secs(SHUTDOWN_TIMEOUT),
                receiver,
            )
            .await
            {
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
            match tokio::time::timeout(
                Duration::from_secs(SHUTDOWN_TIMEOUT),
                task,
            )
            .await
            {
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

        let _ = self.flush();

        if let Some(sigint) = self.sigint_task.take() {
            sigint.abort();
        }

        info!("Shutdown complete");
        Ok(())
    }

    /// Signal that [`Node`] should stop,
    /// and then wait for it's tasks to complete.
    ///
    /// This is a convenience method that bundles `stop()` and `wait_shutdown`.
    pub async fn shutdown(self) -> Result<(), NodeError> {
        self.stop().await;
        self.wait_shutdown().await
    }

    /// Check wheter the node should stop based on `stop_signal`'s value.
    pub async fn should_stop(&self) -> bool {
        *self.stop_signal.read().await
    }

    /// Flush the node's state to disk.
    pub fn flush(&mut self) -> Result<(), NodeError> {
        info!("Flushing state to disk...");
        self.chain_state.flush().map_err(|e| {
            error!("Failed to persist chain to disk: {:?}", e);
            match e {
                BlockchainError::Database(db_err) => {
                    NodeError::Database(Arc::new(db_err))
                }
                BlockchainError::Io(io_err) => NodeError::Io(Arc::new(io_err)),
                other => NodeError::Blockchain(Arc::new(other)),
            }
        })?;
        info!("Successfully persisted state to disk");
        Ok(())
    }

    /// Get the [`UtreexoNodeConfig`] from the running node.
    pub async fn get_config(&self) -> Result<UtreexoNodeConfig, NodeError> {
        let config = self.node_handle.get_config().await?;

        Ok(config)
    }

    /// Manually initiate a connection to a peer.
    pub async fn connect_peer(
        &self,
        peer_address: &SocketAddr,
    ) -> Result<bool, NodeError> {
        // Attempt to make an encrypted BIP-0324 P2PV2 connection with
        // the peer. If he does not support it, silently fallback to P2PV1.
        let try_p2p_v2: bool = true;

        match self
            .node_handle
            .add_peer(peer_address.ip(), peer_address.port(), try_p2p_v2)
            .await
        {
            Ok(true) => {
                debug!("Manual connection established with peer {peer_address:#?} sucessfully");
                Ok(true)
            }
            Ok(false) => {
                warn!("Failed to establish manual connection with peer {peer_address:#?}");
                Ok(false)
            }
            Err(e) => {
                error!("Network error while attempting to establish manual connection with peer {peer_address:#?}: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Disconnect from a peer.
    ///
    /// Returns a `bool` indicating whether
    /// disconnection was successful, or an error.
    pub async fn disconnect_peer(
        &self,
        peer_address: &SocketAddr,
    ) -> Result<bool, NodeError> {
        match self
            .node_handle
            .disconnect_peer(peer_address.ip(), peer_address.port())
            .await
        {
            Ok(true) => {
                debug!("Disconnected from peer {peer_address:#?}");
                Ok(true)
            }
            Ok(false) => {
                error!("Failed to disconnect from peer {peer_address:#?}");
                Ok(false)
            }
            Err(e) => {
                error!("Failed disconnect from peer {peer_address:#?}: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Remove a peer from the address manager.
    ///
    /// Returns a `bool` indicating whether
    /// removal was successful, or an error.
    pub async fn remove_peer(
        &self,
        peer_address: &SocketAddr,
    ) -> Result<bool, NodeError> {
        match self
            .node_handle
            .remove_peer(peer_address.ip(), peer_address.port())
            .await
        {
            Ok(true) => {
                debug!("Removed address {peer_address:#?} from the address manager");
                Ok(true)
            }
            Ok(false) => {
                error!(
                    "Failed to remove address {peer_address:#?} from the address manager"
                );
                Ok(false)
            }
            Err(e) => {
                error!("Failed to remove address {peer_address:#?} from the address manager: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Get information about peers the [`Node`] is connected to.
    pub async fn get_peer_info(&self) -> Result<Vec<PeerInfo>, NodeError> {
        match self.node_handle.get_peer_info().await {
            Ok(peer_infos) => {
                debug!("Got peer infos sucesssfully");
                trace!("{peer_infos:#?}");
                Ok(peer_infos)
            }
            Err(e) => {
                error!("Failed to get peer infos: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Ping all peers the node is currenctly connected to.
    pub async fn ping(&self) -> Result<bool, NodeError> {
        match self.node_handle.ping().await {
            Ok(true) => {
                debug!("Sucessfully sent ping to all peers");
                Ok(true)
            }
            Ok(false) => {
                warn!("Failed to send ping to all peers");
                Ok(false)
            }
            Err(e) => {
                error!("Error while sending ping to all peers: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// TODO(@luisschwab):
    /// Connect to a curated list of known Utreexo bridges
    /// for an expedited IBD experience. This is needed because
    /// few DNS seeders index the Utreexo service bits, and
    /// discovering a bridge naturally via P2P has a low P-value
    /// of happening naturally and in a short time frame.
    pub async fn bootstrap_bridges() {}

    ///////////////////// BLOCKCHAIN /////////////////////

    /// Create a subscriber for new blocks.
    /// Blocks are broadcasted to the consumer as they come.
    ///
    /// TODO(@luisschwab): "If a module performs some heavy-lifting on the
    /// block's data, it should pass in a vector or a channel where data can
    /// be transferred to the atual worker, otherwise chainstate will be
    /// stuck for as long as you have work to do." Is processing a block
    /// into a ChangeSet heavy-lifting? The actual consumer must
    /// implement the `BlockConsumer` trait.
    pub fn block_subscriber<T: BlockConsumer + 'static>(
        &self,
        block_consumer: Arc<T>,
    ) {
        self.chain_state.subscribe(block_consumer);
    }

    // /// TODO(@luisschwab): implement a transaction subscriber on Floresta.
    // /// Transactions are broadcasted to the consumer as they come.
    // pub fn transaction_subscriber<T: TransactionConsumer + 'static>(&self,
    // transaction_consumer: Arc<T>) {     self.chain.
    // subscribe(transaction_consumer); }

    /// Check wheter the node is still in Initial Block Download.
    pub fn in_ibd(&self) -> Result<bool, NodeError> {
        let ibd = self.chain_state.is_in_ibd();

        Ok(ibd)
    }

    /// Get the current blockchain height.
    pub fn get_height(&self) -> Result<u32, NodeError> {
        let height = self.chain_state.get_height()?;
        Ok(height)
    }

    /// Get the current validated blockchain height.
    pub fn get_validation_height(&self) -> Result<u32, NodeError> {
        let height = self.chain_state.get_validation_index()?;
        Ok(height)
    }

    /// Get the current Utreexo accumulator state, as a [`Stump`].
    pub fn get_accumulator(&self) -> Result<Stump, NodeError> {
        let stump = self.chain_state.get_acc();

        Ok(stump)
    }

    /// Get a [`BlockHash`] from a block height.
    pub fn get_blockhash(&self, height: u32) -> Result<BlockHash, NodeError> {
        let hash = self.chain_state.get_block_hash(height)?;

        Ok(hash)
    }

    /// Get a [`Block`], given it's [`BlockHash`], from the network.
    pub async fn get_block(
        &self,
        blockhash: BlockHash,
    ) -> Result<Option<Block>, NodeError> {
        let block = self.node_handle.get_block(blockhash).await?;

        Ok(block)
    }
}
