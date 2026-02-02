use std::fs;
use std::sync::Arc;

use bdk_wallet::Wallet;
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
use crate::logger::setup_logger;
use crate::updater::WalletUpdater;
use crate::Node;
use crate::WalletUpdate;

/// The [`Builder`] builds a [`Node`] from user-defined configuration or
/// deafault values.
pub struct Builder {
    /// What `BlockHash` should be used for AssumeValid.
    /// This will assume all scripts up to block `assume_valid_blockhash`
    /// as valid. This speeds up IBD since no script eval is done.
    assume_valid_blockhash: AssumeValidArg,
    /// The configuration parameters for the node.
    config: UtreexoNodeConfig,
    /// Wheter to build the `tracing_subscriber` logger.
    build_logger: bool,
    /// Whether the log level should be set to debug.
    /// This can be overriden by the `RUST_LOG` environment variable.
    debug: bool,
    /// Whether to log to `stdout`.
    log_to_stdout: bool,
    /// Whether to lgo to a file.
    log_to_file: bool,
    /// The `Wallet` which the node should emit updates about.
    wallet: Option<Wallet>,
}

impl Default for Builder {
    fn default() -> Self {
        // Don't use AssumeValid by default.
        let assume_valid_default: AssumeValidArg = AssumeValidArg::Disabled;

        // Use `Network::Signet` by default.
        let network_default: Network = Network::Signet;

        // Get the hardcoded `AssumeUTreeXO` value from `floresta-chain`.
        // The node will begin to sync from the `assume_utreexo` height,
        // trusting the previous chain history. If the `backfill` option
        // is set, the node will validate the previous history in the
        // background.
        //
        // Currently, only `Network::Bitcoin` is supported for AssumeUTreeXO.
        let assume_utreexo_default: AssumeUtreexoValue =
            ChainParams::get_assume_utreexo(network_default);

        // The default data directory for the node. Currently, `./data`.
        let data_dir_default: String = format!("./data/{}", network_default);

        // The default behavior for the backfill task. Defaults to `true`.
        let backfill_default: bool = true;

        // The default user agent for P2P communication.
        let user_agent_default: String = env!("USER_AGENT").to_string();

        Self {
            assume_valid_blockhash: assume_valid_default,
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
    /// Instantiate a new [`Node`] with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Instantiate a new or set an existing [`Node`] with a new config.
    pub fn from_config(mut self, config: UtreexoNodeConfig) -> Self {
        self.config = config;

        // Set the custom `bdk_floresta` user agent.
        self.config.user_agent = env!("USER_AGENT").to_string();

        self
    }

    /// Couple a [`Wallet`] to the [`Node`], such that it emits relevant updates
    /// to it.
    pub fn with_wallet(mut self, wallet: Wallet) -> Self {
        self.wallet = Some(wallet);

        self
    }

    /// Skip script validation for all blocks that precede [`BlockHash`].
    pub fn with_assumevalid(mut self, blockhash: BlockHash) -> Self {
        self.assume_valid_arg = AssumeValidArg::UserInput(blockhash);

        self
    }

    /// Build the `tracing_subscriber` defined in this crate.
    pub fn build_logger(mut self) -> Self {
        self.build_logger = true;

        self
    }

    /// Set the log level to debug.
    pub fn set_debug(mut self) -> Self {
        self.debug = true;
        self
    }

    /// Build a [`Node`] from a [`Builder`].
    ///
    /// This method will setup the node's logger,
    /// chainstate storage and filter header storage.
    ///
    /// To run the [`Node`], call [`Node::run()`].
    pub fn build(self) -> Result<Node, BuilderError> {
        // Assert that the [`Node`]'s and [`Wallet`]'s [`Network`] match.
        if let Some(ref wallet) = self.wallet {
            if self.config.network != wallet.network() {
                return Err(BuilderError::NetworkMismatch);
            }
        }

        // Create the data directory for [`Node`] and [`Wallet`] data.
        fs::create_dir_all(&self.config.datadir)?;

        // Keep a guard for the logger during the [`Node`]s lifetime.
        let _logger_guard = match self.build_logger {
            true => setup_logger(
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
                error!("Failed to open flat chainstore: {:?}", e);
                return Err(e.into());
            }
        };

        // Create a [`ChainState`] from the [`FlatChainStore`].
        let chain_state: Arc<ChainState<FlatChainStore>> = Arc::new(ChainState::new(
            chain_store,
            self.config.network,
            self.assume_valid_blockhash,
        ));
        info!(
            "ChainState loaded from FlatChainStore at {}",
            self.config.datadir
        );

        // Create configuration for the [`FlatFilterStore`].
        let flat_filters_store =
            FlatFiltersStore::new((self.config.datadir.clone() + "/cbf").into());
        let filters = Arc::new(NetworkFilters::new(flat_filters_store));
        info!("FilterStore loaded at height {}", filters.get_height()?);

        // Create an Arc'ed stop signal that keeps
        // track of whether the [`Node`] should stop.
        let stop_signal: Arc<RwLock<bool>> = Arc::new(RwLock::new(false));

        // Create a [`Pollard`] accumulator
        // for the mempool and transaction cache.
        let pollard: Pollard<BitcoinNodeHash> = Pollard::new();
        let mempool: Arc<Mutex<Mempool>> = Arc::new(Mutex::new(Mempool::new(pollard, 1_000_000)));

        let node_inner = UtreexoNode::<_, RunningNode>::new(
            self.config.clone(),
            chain_state.clone(),
            mempool,
            Some(filters),
            stop_signal.clone(),
            AddressMan::default(),
        )
        .expect("Failed to instantiate the Node");

        // Get a handle from the [`Node`], used to interact with it.
        let node_handle: NodeInterface = node_inner.get_handle();

        info!("Node instantiated successfully");

        // Set up the [`Wallet`]'s update channel, and it's sender and receiver.
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
