// SPDX-Licence-Identifier: MIT

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
use std::sync::Arc;

use bdk_wallet::Wallet;
use bitcoin::BlockHash;
use bitcoin::Network;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStore;
use floresta_chain::pruned_utreexo::flat_chain_store::FlatChainStoreConfig;
use floresta_chain::AssumeUtreexoValue;
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
use floresta_wire::rustreexo::accumulator::node_hash::BitcoinNodeHash;
use floresta_wire::rustreexo::accumulator::pollard::Pollard;
use floresta_wire::UtreexoNodeConfig;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;

use crate::error::BuilderError;
use crate::logger::build_logger;
use crate::updater::WalletUpdater;
use crate::Node;
use crate::WalletUpdate;

/// Builds a [`Node`] from default or custom parameters.
pub struct Builder {
    /// The [`AssumeValidArg`] defines the height where scripts
    /// will start to be evaluated. All scripts on blocks that precede
    /// [`AssumeValidArg`] will not be evaluated, and will be assumed as valid.
    ///
    /// Defaults to [`AssumeValidArg::Hardcoded`], meaning that scripts
    /// will be evaluated from a checkpoint defined in `floresta-chain`.
    assume_valid_arg: AssumeValidArg,
    /// The [`UtreexoNodeConfig`] holds all configuration parameters for the [`Node`],
    /// such as [`Network`], [`AssumeUtreexoValue`], SOCKS5 proxy, backfill and user agent.
    config: UtreexoNodeConfig,
    /// Whether to build the tracing subscriber logger.
    ///
    /// Defaults to `false`.
    build_logger: bool,
    /// Set the log level to DEBUG. Can be overriden by the `RUST_LOG` environment variable.
    ///
    /// Defaults to `false`.
    debug: bool,
    /// Pipe the log to standard output.
    ///
    /// Defaults to `true`.
    log_to_stdout: bool,
    /// Pipe the log to a file (`data_dir/debug.log`).
    ///
    /// Defaults to `true`.
    log_to_file: bool,
    /// A [`Wallet`] that receives updates from the [`Node`].
    wallet: Option<Wallet>,
}

impl Default for Builder {
    fn default() -> Self {
        // Use the hardcoded value set on `floresta-wire`.
        let assume_valid_default: AssumeValidArg = AssumeValidArg::Hardcoded;

        // Use `Network::Signet` as the default `Network` for the `Node` and `Wallet`.
        let network_default: Network = Network::Signet;

        // Use the hardcoded `AssumeUtreexoValue` defined in `floresta-chain`.
        //
        // Currently, only `Network::Bitcoin` has an `AssumeUtreexoValue`
        // defined, other networks will default to the genesis block.
        let assume_utreexo_default: AssumeUtreexoValue =
            ChainParams::get_assume_utreexo(network_default);

        // The default data directory for the node.
        let data_dir_default: String = format!("./data/{}", network_default);

        // Validate blocks previous to the `AssumeUtreexoValue` height after IBD is finished.
        let backfill_default: bool = true;

        // The node's user agent, derived in the `build.rs` script.
        let user_agent_default: String = env!("USER_AGENT").to_string();

        Self {
            assume_valid_arg: assume_valid_default,
            config: UtreexoNodeConfig {
                network: network_default,
                pow_fraud_proofs: true,
                compact_filters: true,
                fixed_peer: None,
                max_banscore: 10,
                datadir: data_dir_default,
                proxy: None,
                assume_utreexo: Some(assume_utreexo_default),
                backfill: backfill_default,
                filter_start_height: None,
                user_agent: user_agent_default,
                allow_v1_fallback: true,
                disable_dns_seeds: false,
            },
            build_logger: false,
            debug: false,
            log_to_stdout: true,
            log_to_file: true,
            wallet: None,
        }
    }
}

impl Builder {
    /// Instantiate a [`Builder`] with [default](Builder::default()) values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Instantiate a [`Builder`] from
    pub fn from_config(mut self, config: UtreexoNodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Add a [`Wallet`] that will receive updates from the [`Node`].
    pub fn with_wallet(mut self, wallet: Wallet) -> Self {
        self.wallet = Some(wallet);

        self
    }

    /// Skip script validation for all blocks that precede [`BlockHash`].
    pub fn with_assumevalid(mut self, blockhash: BlockHash) -> Self {
        self.assume_valid_arg = AssumeValidArg::UserInput(blockhash);

        self
    }

    /// Build the `tracing_subscriber` logger implemented
    /// in this crate's [logger module](crate::logger).
    pub fn build_logger(mut self) -> Self {
        self.build_logger = true;

        self
    }

    /// Set the log level to DEBUG.
    ///
    /// This can be overriden by the `RUST_LOG` environment variable.
    pub fn set_debug(mut self) -> Self {
        self.debug = true;
        self
    }

    /// Build a [`Node`] from a [`Builder`].
    ///
    /// This method will setup the node's chainstate and compact filter
    /// stores, and the logger, if enabled.
    ///
    /// It will not run the [`Node`]. To run it, call [`Node::run()`].
    pub fn build(self) -> Result<Node, BuilderError> {
        // Assert that the node and wallet network are equal.
        if let Some(ref wallet) = self.wallet {
            if self.config.network != wallet.network() {
                return Err(BuilderError::NetworkMismatch);
            }
        }

        // Create the data directory for node and wallet data.
        fs::create_dir_all(&self.config.datadir)?;

        // Keep a guard for the logger during the node's lifetime.
        let _logger_guard = match self.build_logger {
            true => build_logger(
                &self.config.datadir,
                self.log_to_file,
                self.log_to_stdout,
                self.debug,
            )?,
            false => None,
        };

        // Create configuration for the chain store.
        let chain_store_cfg: FlatChainStoreConfig =
            FlatChainStoreConfig::new(self.config.datadir.clone() + "/chain");

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
            self.config.network,
            self.assume_valid_arg,
        ));
        info!(
            "ChainState loaded from FlatChainStore at {}",
            self.config.datadir
        );

        // Create the [`FlatFilterStore`]'s configuration.
        let flat_filters_store =
            FlatFiltersStore::new((self.config.datadir.clone() + "/cbf").into());
        let filters = Arc::new(NetworkFilters::new(flat_filters_store));
        info!("FilterStore loaded at height {}", filters.get_height()?);

        // Create an Arc'ed stop signal that keeps track of whether the [`Node`] should stop.
        let stop_signal: Arc<RwLock<bool>> = Arc::new(RwLock::new(false));

        // Create a [`Pollard`] accumulator for the mempool and transaction cache.
        let pollard: Pollard<BitcoinNodeHash> = Pollard::new();
        let mempool: Arc<Mutex<Mempool>> = Arc::new(Mutex::new(Mempool::new(pollard, 1_000_000)));

        // Encapsulate the actual node as an inner of [`Node`].
        let node_inner = UtreexoNode::<_, RunningNode>::new(
            self.config.clone(),
            chain_state.clone(),
            mempool,
            Some(filters),
            stop_signal.clone(),
            AddressMan::default(),
        )
        .expect("Failed to instantiate the Node");

        // Get a handle for the [`Node`].
        // Used to call methods of the underlying [`UtreexoNode`].
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
            _node_config: self.config,
            _debug: self.debug,
            _logger_guard,
            node_inner: Some(node_inner),
            node_handle,
            chain_state,
            task_handle: None,
            sigint_task: None,
            stop_signal,
            stop_receiver: None,
            wallet: wallet_arc,
            update_subscriber: Some(update_rx),
        })
    }
}
