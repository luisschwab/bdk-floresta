// SPDX-Licence-Identifier: MIT

#![doc = include_str!("../README.md")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bdk_wallet::Wallet;
use floresta_chain::{
    pruned_utreexo::flat_chain_store::FlatChainStore, BlockchainError,
    ChainState,
};
use floresta_wire::node_interface::NodeInterface;
use floresta_wire::rustreexo::accumulator::stump::Stump;
use tokio::sync::oneshot;
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;

pub(crate) use floresta_chain::{
    pruned_utreexo::{BlockchainInterface, UpdatableChainstate},
    BlockConsumer,
};
pub use floresta_wire::node_interface::PeerInfo;
pub use floresta_wire::rustreexo;
pub use floresta_wire::UtreexoNodeConfig;

use error::NodeError;
pub use updater::WalletUpdate;

pub mod builder;
pub mod error;
mod logger;
mod updater;

/// [`FlorestaNode`] represents the embedded and fully validating
/// Compact State Node.
pub struct FlorestaNode {
    /// Configuration parameters for [`FlorestaNode`].
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
    /// `shutdown_signal` to true, signalling [`FlorestaNode`] for a gracefull
    /// shutdown.
    pub(crate) sigint_task: Option<JoinHandle<()>>,
    /// The `stop_signal` is continuously checked by [`FlorestaNode`].
    /// If set, a gracefull shutdown will be initiated.
    pub(crate) stop_signal: Arc<RwLock<bool>>,
    /// A receiver for the stop notification from the `node_task`.
    pub(crate) stop_receiver: Option<oneshot::Receiver<()>>,
    /// A guard for the logger thread. Must be kept alive for the lifetime of
    /// [`FlorestaNode`].
    pub(crate) _logger_guard: Option<WorkerGuard>,
    /// The `Wallet` to be coupled to the [`FlorestaNode`].
    pub wallet: Option<Arc<RwLock<Wallet>>>,
    /// The `update_subscriber` emits relevant wallet events that can be
    /// applied to the `Wallet`.
    pub update_subscriber: Option<UnboundedReceiver<WalletUpdate>>,
}

impl FlorestaNode {
    /// Set `stop_signal` to `true` to initiate a graceful shutdown.
    async fn stop(&self) {
        info!("Setting the stop signal to true");
        *self.stop_signal.write().await = true;
    }

    /// Wait for the node to finish it's tasks.
    async fn wait_shutdown(mut self) -> Result<(), NodeError> {
        info!("Waiting for shutdown to complete");

        if let Some(receiver) = self.stop_receiver.take() {
            match tokio::time::timeout(Duration::from_secs(30), receiver).await
            {
                Ok(Ok(())) => {
                    info!("Node signaled shutdown completion");
                }
                Ok(Err(_)) => {
                    warn!("Node shutdown channel closed without sending");
                }
                Err(_) => {
                    error!("Node shutdown notification timed out after 30s");
                }
            }
        }

        if let Some(task) = self.task_handle.take() {
            match tokio::time::timeout(Duration::from_secs(5), task).await {
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
                    warn!("Node task join timed out after 5s");
                }
            }
        }

        let _ = self.flush();

        if let Some(sigint) = self.sigint_task.take() {
            let _ = sigint.await;
        }

        info!("Shutdown complete");
        Ok(())
    }

    /// Convenience method: signal that the node
    /// should stop and then wait for tasks to complete.
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

    /// Manually initiate a connection to a peer.
    pub async fn connect_peer(
        &self,
        peer_address: &SocketAddr,
    ) -> Result<bool, NodeError> {
        // Attempt to make an encrypted BIP-0324 P2P V2 connection with the
        // peer. If he does not support it, it will silently fallback to
        // unencrypted P2P V1.
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

    /// Manually disconnect from a peer.
    pub async fn disconnect_peer(
        &self,
        peer_address: &SocketAddr,
    ) -> Result<bool, NodeError> {
        match self
            .node_handle
            .remove_peer(peer_address.ip(), peer_address.port())
            .await
        {
            Ok(true) => {
                debug!("Sucessfull manual disconnection from peer {peer_address:#?}");
                Ok(true)
            }
            Ok(false) => {
                error!(
                    "Failed to manually disconnect from peer {peer_address:#?}"
                );
                Ok(false)
            }
            Err(e) => {
                error!("Failed to manually disconnect from peer {peer_address:#?}: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Get information about peers [`FlorestaNode`] is connected to.
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

    // TODO(@luisschwab): implement methods to pull metrics from the node so
    // users have access to them.
}
