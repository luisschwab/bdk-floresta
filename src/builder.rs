// SPDX-License-Identifier: MIT

//! # Builder
//!
//! This module holds all logic needed to instantiate
//! a [`Node`] from default or user defined values.
//!
//! ```rust
//! use bdk_floresta::builder::Builder;
//! use bdk_floresta::UtreexoNodeConfig;
//! use bitcoin::Network;
//!
//! let config = UtreexoNodeConfig {
//!     network: Network::Signet,
//!     datadir: format!("{}{}", DATA_DIR, NETWORK),
//!     ..Default::default(),
//! };
//!
//! let mut node = Builder::new()
//!     .from_config(config)
//!     .build()
//!     .unwrap();
//! ```

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use bdk_wallet::Wallet;
use bitcoin::BlockHash;
use bitcoin::Network;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStore;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStoreConfig;
use floresta_chain::AssumeValidArg;
use floresta_chain::BlockchainInterface;
use floresta_chain::ChainParams;
use floresta_chain::ChainState;
use floresta_compact_filters::flat_filters_store::FlatFiltersStore;
use floresta_compact_filters::network_filters::NetworkFilters;
use floresta_mempool::Mempool;
use floresta_wire::address_man::AddressMan;
use floresta_wire::node::running_ctx::RunningNode;
use floresta_wire::node::UtreexoNode;
use floresta_wire::node_interface::NodeInterface;
use floresta_wire::UtreexoNodeConfig;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;

use crate::error::BuilderError;
#[cfg(feature = "logger")]
use crate::logger::build_logger;
#[cfg(feature = "logger")]
use crate::logger::LoggerConfig;
use crate::updater::WalletUpdater;
use crate::Node;
use crate::WalletUpdate;

/// Configuration parameters for building a [`Node`].
#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// The [`Network`] which to run the [`Node`] and [`Wallet`] on.
    pub network: Network,
    /// The path to the directory where [`Node`] and [`Wallet`] data will be persisted to.
    pub data_directory: PathBuf,
    /// Proof-of-Work Fraud Proofs allow skipping verification of the
    /// entire blockchain with a better trust assumption than bare SPV.
    pub enable_powfps: bool,
    /// Skip script evaluation and assume all as valid for all preceding blocks.
    /// If set to `None`, all scripts since the genesis block will be evaluated.
    pub assume_valid: Option<BlockHash>,
    /// Skip IBD up until a checkpoint set by the [`Floresta`](https://github.com/getfloresta/Floresta) developers.
    /// See the [`ChainParams::get_assume_utreexo`](ChainParams::get_assume_utreexo)
    /// implementation for more details.
    pub assume_utreexo: bool,
    /// Download and validate all skipped blocks after IBD in the
    /// background, if `AssumeValid` or `AssumeUtreexo` are enabled.
    pub perform_backfill: bool,
    /// The user agent that will be sent to peers in the `version` message.
    pub user_agent: String,
    /// Connect to a single, pre-defined peer. If set, no other P2P connections will be made.
    pub fixed_peer: Option<SocketAddr>,
    /// The maximum banscore a peer can reach before he is banned.
    pub max_banscore: u32,
    /// A `SOCKS5` proxy which to route all traffic through.
    pub socks5_proxy: Option<SocketAddr>,
    /// Whether to disable fetching peers from DNS seeds to bootstrap the [`Node`]'s address
    /// manager.
    pub disable_dns_seeds: bool,
    /// Whether to allow connecting to peers that only support the unencrypted P2PV1 protocol.
    /// The [`Node`] will always attempt to establish an encrypted P2PV2 connection (BIP-0324),
    /// and will fall back to P2PV1 if set to true.
    pub allow_p2pv1_fallback: bool,
    /// The size of the [`Mempool`], in MB. If the [`Mempool`] becomes
    /// full, transactions are evicted based on their fee rate, lowest first.
    pub mempool_size: usize,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            network: Network::Signet,
            data_directory: PathBuf::from(format!("./data/{}", Network::Signet)),
            enable_powfps: true,
            assume_valid: None,
            assume_utreexo: true,
            perform_backfill: true,
            user_agent: env!("USER_AGENT").to_string(),
            fixed_peer: None,
            max_banscore: 100,
            socks5_proxy: None,
            disable_dns_seeds: false,
            allow_p2pv1_fallback: true,
            mempool_size: 100,
        }
    }
}

impl From<NodeConfig> for UtreexoNodeConfig {
    fn from(value: NodeConfig) -> Self {
        let mut assume_utreexo = None;
        if value.assume_utreexo {
            assume_utreexo = Some(ChainParams::get_assume_utreexo(value.network));
        }

        Self {
            network: value.network,
            pow_fraud_proofs: value.enable_powfps,
            compact_filters: true,
            fixed_peer: value.fixed_peer.map(|addr| addr.to_string()),
            max_banscore: value.max_banscore,
            datadir: value.data_directory.to_string_lossy().to_string(),
            proxy: value.socks5_proxy,
            assume_utreexo,
            backfill: value.perform_backfill,
            filter_start_height: Some(0),
            user_agent: value.user_agent,
            allow_v1_fallback: value.allow_p2pv1_fallback,
            disable_dns_seeds: value.disable_dns_seeds,
        }
    }
}

// TODO
impl From<UtreexoNodeConfig> for NodeConfig {
    fn from(_value: UtreexoNodeConfig) -> Self {
        todo!()
    }
}

/// Builds a [`Node`] from default or custom parameters.
#[derive(Default)]
pub struct Builder {
    /// Configuration for building the [`Node`].
    pub node_configuration: NodeConfig,
    /// Configuration for building the tracing subscriber logger.
    #[cfg(feature = "logger")]
    pub logger_configuration: LoggerConfig,
    /// A [`Wallet`] that will receive updates from the [`Node`].
    pub wallet: Option<Wallet>,
}

impl Builder {
    /// Instantiate a [`Builder`] with [default](Builder::default()) values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Instantiate a [`Builder`] from a [`NodeConfig`].
    pub fn from_config(mut self, node_configuration: NodeConfig) -> Self {
        self.node_configuration = node_configuration;
        self
    }

    /// Add a [`Wallet`] that will receive updates from the [`Node`].
    pub fn with_wallet(mut self, wallet: Wallet) -> Self {
        self.wallet = Some(wallet);
        self
    }

    /// Build a [`Node`] from a [`Builder`].
    ///
    /// It will not run the [`Node`]. To run it, call [`Node::run()`].
    pub fn build(self) -> Result<Node, BuilderError> {
        // Assert that the node and wallet network are equal.
        if let Some(ref wallet) = self.wallet {
            if self.node_configuration.network != wallet.network() {
                return Err(BuilderError::NetworkMismatch);
            }
        }

        // Create the data directory for node and wallet data.
        fs::create_dir_all(&self.node_configuration.data_directory)?;

        // Keep a guard for the logger during the node's lifetime.
        #[cfg(feature = "logger")]
        let _logger_guard = build_logger(
            &self.node_configuration.data_directory,
            self.logger_configuration.log_to_file,
            self.logger_configuration.log_to_stdout,
            self.logger_configuration.log_level,
        )?;

        // Create configuration for the chain store.
        let chain_store_cfg = FlatChainStoreConfig::new(
            self.node_configuration
                .data_directory
                .join("chain")
                .to_string_lossy()
                .to_string(),
        );

        // Try to load an existing [`FlatChainStore`]
        // from the file system, or create a new one.
        let chain_store: FlatChainStore = match FlatChainStore::new(chain_store_cfg) {
            Ok(store) => store,
            Err(e) => {
                error!("Failed to open FlatChainStore: {:?}", e);
                return Err(e.into());
            }
        };

        // Create a [`ChainState`] from the [`FlatChainStore`].
        let chain_state: Arc<ChainState<FlatChainStore>> = Arc::new(ChainState::new(
            chain_store,
            self.node_configuration.network,
            AssumeValidArg::Hardcoded,
        ));
        info!(
            "ChainState loaded from FlatChainStore at {}",
            self.node_configuration.data_directory.display()
        );

        // Create the [`FlatFiltersStore`]'s configuration.
        let flat_filters_store =
            FlatFiltersStore::new(self.node_configuration.data_directory.join("cbf"));
        let filters = Arc::new(NetworkFilters::new(flat_filters_store));
        info!("FilterStore loaded at height {}", filters.get_height()?);

        // Create an Arc'ed stop signal that keeps track of whether the [`Node`] should stop.
        let stop_signal: Arc<RwLock<bool>> = Arc::new(RwLock::new(false));

        // Create a [`Mempool`] for the [`Node`].
        let mempool_size_bytes = 1_000_000 * self.node_configuration.mempool_size;
        let mempool: Arc<Mutex<Mempool>> = Arc::new(Mutex::new(Mempool::new(mempool_size_bytes)));

        // Encapsulate the actual [`UtreexoNode`] as an inner of [`Node`].
        let node_inner = UtreexoNode::<_, RunningNode>::new(
            self.node_configuration.clone().into(),
            chain_state.clone(),
            mempool,
            Some(filters),
            stop_signal.clone(),
            AddressMan::default(),
        )
        .expect("Failed to instantiate the Node");

        // Get a handle to interact with the [`Node`].
        let node_handle: NodeInterface = node_inner.get_handle();

        // Set up the [`Wallet`]'s update channel, sender and receiver.
        let (update_tx, update_rx) = unbounded_channel::<WalletUpdate>();
        let wallet_arc = if let Some(wallet) = self.wallet {
            let wallet_arc = Arc::new(RwLock::new(wallet));
            let updater = Arc::new(WalletUpdater::new(update_tx));
            chain_state.subscribe(updater);
            info!("Wallet update subscriber set up successfully");
            Some(wallet_arc)
        } else {
            None
        };

        info!("Node instantiated successfully");

        Ok(Node {
            config: self.node_configuration,
            node_inner: Some(node_inner),
            node_handle,
            chain_state,
            task_handle: None,
            sigint_task: None,
            stop_signal,
            stop_receiver: None,
            #[cfg(feature = "logger")]
            _logger_guard,
            wallet: wallet_arc,
            update_subscriber: Some(update_rx),
        })
    }
}
