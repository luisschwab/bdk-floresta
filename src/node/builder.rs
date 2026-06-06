// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Builder
//!
//! This module holds all logic needed to instantiate a [`Node`]
//! from default or user defined values, by way of the [`Builder`].

use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use bitcoin::BlockHash;
use bitcoin::Network;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStore;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStoreConfig;
use floresta_chain::AssumeValidArg;
use floresta_chain::ChainParams;
use floresta_chain::ChainState;
use floresta_compact_filters::flat_filters_store::FlatFiltersStore;
use floresta_compact_filters::network_filters::NetworkFilters;
use floresta_mempool::Mempool;
use floresta_wire::address_man::AddressMan;
use floresta_wire::address_man::ReachableNetworks;
use floresta_wire::node::running_ctx::RunningNode;
use floresta_wire::node::UtreexoNode;
use floresta_wire::node_handle::NodeHandle;
use floresta_wire::UtreexoNodeConfig;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "logger")]
use crate::logger::Logger;
use crate::node::error::BuilderError;
use crate::node::fsm::State;
use crate::node::Node;

/// The number of bytes per megabyte.
const BYTES_PER_MB: usize = 1_000_000;

/// Configuration parameters for building a [`Node`].
#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// The [`Network`] which the [`Node`] will operate on.
    pub network: Network,

    /// The path to the directory where [`Node`] data will be persisted to.
    pub datadir: PathBuf,

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

    /// Connect to a pre-defined set of peers. If set, no other P2P connections will be made.
    pub fixed_peers: Option<Vec<SocketAddr>>,

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

    /// The maximum number of addresses held in the [`AddressMan`].
    /// Defaults to 50_000 addresses.
    pub address_man_size: Option<usize>,

    /// The set of networks this node will communicate on.
    /// Currently, only [`ReachableNetworks::IPv4`] and [`ReachableNetworks::IPv6`] are supported.
    pub reachable_nets: Vec<ReachableNetworks>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            network: Network::Signet,
            datadir: PathBuf::from("./data/{}").join(Network::Signet.to_string()),
            enable_powfps: true,
            assume_valid: None,
            assume_utreexo: true,
            perform_backfill: false,
            user_agent: env!("USER_AGENT").to_string(),
            fixed_peers: None,
            max_banscore: 100,
            socks5_proxy: None,
            disable_dns_seeds: false,
            allow_p2pv1_fallback: true,
            mempool_size: 100,
            address_man_size: None,
            reachable_nets: ReachableNetworks::SUPPORTED.to_vec(),
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
            fixed_peers: value
                .fixed_peers
                .into_iter()
                .flatten()
                .map(|addr| addr.to_string())
                .collect(),
            max_banscore: value.max_banscore,
            datadir: value.datadir,
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
        unimplemented!()
    }
}

/// Builds a [`Node`] from default or custom parameters.
#[derive(Default)]
pub struct Builder {
    /// Configuration for building the [`Node`].
    pub config: NodeConfig,
    /// Configuration for building the tracing subscriber logger.
    #[cfg(feature = "logger")]
    pub logger: Option<Logger>,
}

impl Builder {
    /// Instantiate a [`Builder`] with [default](Builder::default()) values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Instantiate a [`Builder`] from a [`NodeConfig`].
    pub fn with_config(mut self, config: NodeConfig) -> Self {
        self.config = config;
        self
    }

    #[cfg(feature = "logger")]
    pub fn with_logger(mut self, logger: Logger) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Build a [`Node`] from a [`Builder`].
    ///
    /// It will not run the [`Node`]. To run it, call [`Node::run()`].
    pub fn build(self) -> Result<Node, BuilderError> {
        // Create a directory for node data.
        fs::create_dir_all(&self.config.datadir).map_err(BuilderError::CreateDirectory)?;

        // Init the logger and keep a guard to it, if logging to a file is enabled
        #[cfg(feature = "logger")]
        let _log_guard = if let Some(logger) = self.logger {
            logger.init()?
        } else {
            None
        };

        // Configure the node's chain store
        let chain_store_config = FlatChainStoreConfig::new(self.config.datadir.join("chain"));

        // Try to load an existing chain store from the file system, or create a new one.
        let chain_store: FlatChainStore =
            FlatChainStore::new(chain_store_config).map_err(BuilderError::ChainStoreInit)?;

        // Initialize a chain state from the chain store
        let chain_state: Arc<ChainState<FlatChainStore>> = Arc::new(
            ChainState::open(chain_store, self.config.network, AssumeValidArg::Hardcoded)
                .map_err(BuilderError::ChainState)?,
        );

        // Configure the compact block filter store
        let block_filter_store = FlatFiltersStore::new(self.config.datadir.join("filters"));
        let block_filters = Arc::new(NetworkFilters::new(block_filter_store));

        // A kill signal that keeps track of whether the node should shutdown
        let kill_signal: Arc<RwLock<bool>> = Arc::new(RwLock::new(false));

        // Create the node's mempool
        let mempool_size_bytes = self.config.mempool_size * BYTES_PER_MB;
        let mempool: Arc<Mutex<Mempool>> = Arc::new(Mutex::new(Mempool::new(mempool_size_bytes)));

        // Encapsulate `UtreexoNode` as an inner of `Node`
        let inner = UtreexoNode::<_, RunningNode>::new(
            self.config.clone().into(),
            chain_state.clone(),
            mempool,
            Some(block_filters.clone()),
            kill_signal.clone(),
            AddressMan::new(self.config.address_man_size, &self.config.reachable_nets),
        )
        .map_err(BuilderError::BuildInner)?;

        // Get a handle to interact with the node
        let handle: NodeHandle = inner.get_handle();

        Ok(Node {
            inner: Mutex::new(Some(inner)),
            handle,
            config: self.config,
            state: Arc::new(watch::Sender::new(State::Inactive)),
            started_at: OnceLock::new(),
            actions: Arc::new(watch::Sender::new(HashSet::new())),
            cancellation_token: CancellationToken::new(),
            kill_signal,
            shutdown_task: Mutex::new(None),
            chain_state,
            block_filters,
            #[cfg(feature = "logger")]
            _log_guard,
        })
    }
}
