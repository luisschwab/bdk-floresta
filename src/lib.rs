//! This module implements the [`FlorestaClient`],
//! a bitcoin client with an embedded node,
//! along with methods to interact with this it.

use std::net::SocketAddr;
use std::sync::Arc;

use floresta_chain::pruned_utreexo::BlockchainInterface;
use floresta_chain::pruned_utreexo::UpdatableChainstate;
use floresta_chain::{BlockConsumer, ChainState, KvChainStore};
use floresta_wire::node_interface::NodeInterface;
use floresta_wire::UtreexoNodeConfig;

use log::{debug, info, warn};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

pub mod builder;
pub mod logger;

pub struct FlorestaClient {
    /// Configuration parameters for [`FlorestaClient`].
    pub config: UtreexoNodeConfig,
    /// Whether to set the log level to debug.
    pub debug: bool,
    /// The [`ChainState`] implementation to be used (persistence
    /// will be shared with [`bdk_wallet::Wallet`] in the future).
    pub chain: Arc<ChainState<KvChainStore<'static>>>,
    /// The handle used to send requests and receive responses from the underlying node.
    pub handle: NodeInterface,
    /// Task handle for the underlying node.
    pub node_task: Option<JoinHandle<()>>,
    /// Stop signal for the node.
    pub kill_signal: Arc<RwLock<bool>>,
    /// SIGINT task that sets the `kill_signal` to true.
    pub sigint_task: Option<JoinHandle<()>>,
}

impl FlorestaClient {
    /// Connect to a peer located at [`SocketAddr`].
    pub async fn add_peer(
        &self,
        peer: SocketAddr,
    ) -> anyhow::Result<bool, tokio::sync::oneshot::error::RecvError> {
        // If the peer is not BIP324-capable, it will fallback to unencrypted communication.
        let v2transport = true;

        if let Ok(true) = self
            .handle
            .add_peer(peer.ip(), peer.port(), v2transport)
            .await
        {
            info!("connected to {peer}");
            Ok(true)
        } else {
            warn!("failed to connect to {peer}");
            Ok(false)
        }
    }

    /// Start the node's SIGINT handler.
    pub async fn start_sigint_handler(&mut self) {
        info!("starting SIGINT handler");
        let chain = self.chain.clone();
        let kill_signal = self.kill_signal.clone();

        self.sigint_task = Some(tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to initialize SIGINT handler");

            info!("received SIGINT, stopping bdk_floresta");
            info!("flushing chain to disk");
            let _ = chain.flush();

            let mut kill = kill_signal.write().await;
            *kill = true;
        }))
    }

    /// Subscribe to new block events. Consumers must implement the [`BlockConsumer`] trait.
    pub fn subscribe_block<T: BlockConsumer + 'static>(&self, consumer: Arc<T>) {
        self.chain.subscribe(consumer);
    }

    /// Check if the node is still in IBD.
    pub async fn is_in_ibd(&self) -> anyhow::Result<bool> {
        let ibd = self.chain.is_in_ibd();

        Ok(ibd)
    }

    /// Get peers the node is currently connected to.
    pub async fn get_peers(&self) -> Vec<String> {
        let peers = self.handle.get_peer_info().await;
        let addresses: Vec<String> = peers
            .unwrap_or_default()
            .iter()
            .map(|peer| peer.address.clone())
            .collect();

        addresses
    }

    /// Get the current chain height.
    pub async fn get_height(&self) -> anyhow::Result<u32> {
        let height = self.chain.get_height().unwrap();

        Ok(height)
    }

    /// Get the current validated height.
    pub async fn get_validation_height(&self) -> anyhow::Result<u32> {
        let validated_height = self.chain.get_validation_index().unwrap();

        Ok(validated_height)
    }

    /// Persist the current chainstate to disk.
    pub fn flush(&mut self) -> anyhow::Result<()> {
        let _ = self.chain.flush();
        debug!("flushed chain to disk");

        Ok(())
    }

    /// Shutdown the node.
    pub async fn shutdown(&mut self) -> anyhow::Result<()> {
        let mut kill = self.kill_signal.write().await;
        *kill = true;

        Ok(())
    }
}
