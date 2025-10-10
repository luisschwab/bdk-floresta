// SPDX-Licence-Identifier: MIT

#![doc = include_str!("../README.md")]

// TODO(@luisschwab): make example documentation code (see
// bdk-kyoto/src/lib.rs).

use std::net::SocketAddr;
use std::sync::Arc;

#[allow(unused_imports)]
use bdk_wallet::Wallet;
use floresta_chain::{
    pruned_utreexo::flat_chain_store::FlatChainStore, BlockchainError,
    ChainState,
};
use floresta_wire::node_interface::{NodeInterface, PeerInfo};
#[allow(unused_imports)]
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot::error::RecvError,
    RwLock,
};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;

pub use floresta_chain::{
    pruned_utreexo::{BlockchainInterface, UpdatableChainstate},
    BlockConsumer,
};
pub use floresta_wire::UtreexoNodeConfig;

use error::NodeError;

pub mod builder;
mod error;
mod logger;

pub struct FlorestaNode {
    /// Configuration parameters for [`FlorestaNode`].
    pub node_config: UtreexoNodeConfig,
    /// Whether to set the log level to debug.
    pub debug: bool,
    /// The node's chain state.
    /// TODO(@luisschwab): couple chainstate persistence with
    /// `bdk_chain::ChangeSet` (bdk#1582). Implement a custom `Anchor` with
    /// the inclusion proofs we care about.
    pub chain_state: Arc<ChainState<FlatChainStore>>,
    /// The `node_handle` is used to send requests and receive responses to the
    /// underlying node.
    pub node_handle: NodeInterface,
    /// The `task_handle` is a handle for the undelying node's tasks.
    pub task_handle: Option<JoinHandle<()>>,
    /// The `sigint_task` listens for interrupt signals and sets
    /// `shutdown_signal` to true, signalling [`FlorestaNode`] for a gracefull
    /// shutdown.
    pub sigint_task: Option<JoinHandle<()>>,
    /// The `stop_signal` is continuously checked by [`FlorestaNode`].
    /// If set, a gracefull shutdown will be initiated.
    pub stop_signal: Arc<RwLock<bool>>,
    /// A guard for the logger thread. Must be kept alive for the lifetime of
    /// [`FlorestaNode`].
    pub logger_guard: Option<WorkerGuard>,
    // TODO(@luisschwab): add a channel that will forward wallet updates here.
}

impl FlorestaNode {
    ///////////////////// NODE CONTROL /////////////////////

    /// Set the `stop_signal`, signaling [`FlorestaNode`] to perform a graceful
    /// shutdown.
    pub async fn stop(&mut self) {
        *self.stop_signal.write().await = true;
    }

    ///////////////////// P2P NETWORK /////////////////////

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
                info!("manual connection established with peer {peer_address:#?} sucessfully");
                Ok(true)
            }
            Ok(false) => {
                warn!("failed to establish manual connection with peer {peer_address:#?}");
                Ok(false)
            }
            Err(e) => {
                error!("network error while attempting to establish manual connection with peer {peer_address:#?}: {e}");
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
                info!("sucessfull manual disconnection from peer {peer_address:#?}");
                Ok(true)
            }
            Ok(false) => {
                error!(
                    "failed to manually disconnect from peer {peer_address:#?}"
                );
                Ok(false)
            }
            Err(e) => {
                error!("failed to manually disconnect from peer {peer_address:#?}: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Get information about peers [`FlorestaNode`] is connected to.
    pub async fn get_peer_info(&self) -> Result<Vec<PeerInfo>, NodeError> {
        match self.node_handle.get_peer_info().await {
            Ok(peer_infos) => {
                debug!("got peer infos sucesssfully");
                trace!("{peer_infos:#?}");
                Ok(peer_infos)
            }
            Err(e) => {
                error!("failed to get peer infos: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// Ping all peers the node is currenctly connected to.
    pub async fn ping(&self) -> Result<bool, NodeError> {
        match self.node_handle.ping().await {
            Ok(true) => {
                debug!("sucessfully sent ping to all peers");
                Ok(true)
            }
            Ok(false) => {
                warn!("failed to send ping to all peers");
                Ok(false)
            }
            Err(e) => {
                error!("error while sending ping to all peers: {e}");
                Err(NodeError::Receive(e))
            }
        }
    }

    /// TODO(@luisschwab):
    /// Connect to a curated list of known Utreexo bridges
    /// for an expedited IBD experience. This is needed because
    /// few DNS seeders index the Utreexo service bits.
    pub async fn bootstrap_bridges() {}

    ///////////////////// BLOCKCHAIN /////////////////////

    /// Create a subscriber for new blocks.
    /// Blocks are broadcasted to the consumer as they come.
    ///
    /// TODO(@luisschwab): "If a module performs some heavy-lifting on the
    /// block's data, it should pass in a vector or a channel where data can
    /// be transferred to the atual worker, otherwise chainstate will be
    /// stuck for as long as you have work to do." Is processing a block
    /// into a [`ChangSet`] heavy-lifting? The actual consumer must
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

    /// Persist the current blockchain state to disk.
    pub fn flush(&mut self) -> Result<(), NodeError> {
        info!("persisting chain to disk...");
        match self.chain_state.flush() {
            Ok(_) => {
                info!("sucessfully persisted chain to disk");
                Ok(())
            }
            Err(e) => {
                error!("failed to persist chain to disk");
                match e {
                    BlockchainError::Database(e) => Err(NodeError::Database(e)),
                    BlockchainError::Io(e) => Err(NodeError::Io(e)),
                    e => Err(NodeError::Blockchain(e)),
                }
            }
        }
    }

    /// Get the current blockchain height.
    pub fn get_height(&self) -> Result<u32, NodeError> {
        self.chain_state.get_height().map_err(|e| {
            error!("failed to get block height");
            NodeError::Blockchain(e)
        })
    }

    /// Get the current validated blockchain height.
    pub fn get_validation_height(&self) -> Result<u32, NodeError> {
        self.chain_state.get_validation_index().map_err(|e| {
            error!("failed to get validated block height");
            NodeError::Blockchain(e)
        })
    }

    // TODO(@luisschwab): implement a [`UnboundedSender`] that will fetch new
    // blocks, convert them into a [`bdk_chain::ChangeSet`] and send them
    // over the channel.

    ///////////////////// METRICS /////////////////////

    // TODO(@luisschwab): implement methods to pull metrics from the node so
    // users have access to them.
}
