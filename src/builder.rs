// SPDX-License-Identifier: MIT

use std::fs;
use std::sync::Arc;

use bdk_wallet::bitcoin::Network;
use floresta_chain::{
    pruned_utreexo::{
        flat_chain_store::{FlatChainStore, FlatChainStoreConfig},
        UpdatableChainstate,
    },
    AssumeUtreexoValue, AssumeValidArg, ChainParams, ChainState,
};
use floresta_wire::{
    address_man::AddressMan, mempool::Mempool, node::UtreexoNode,
    node_interface::NodeInterface, running_node::RunningNode,
    UtreexoNodeConfig,
};
use rustreexo::accumulator::{node_hash::BitcoinNodeHash, pollard::Pollard};
use tokio::{
    sync::{oneshot, Mutex, RwLock},
    task,
    task::JoinHandle,
};
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;

use crate::error::BuilderError;
use crate::logger::setup_logger;
use crate::FlorestaNode;

/// [`FlorestaBuilder`] builds a node from the `UtreexoConfig` provided, or
/// builds a node from default values.
pub struct FlorestaBuilder {
    /// What `BlockHash` should be used for AssumeValid.
    assume_valid: AssumeValidArg,
    /// The configuration parameters for the node.
    config: UtreexoNodeConfig,
    /// Whether the log level should be set to debug.
    /// This can be overriden by the `RUST_LOG` environment variable.
    debug: bool,
    /// Whether to log to `stdout`.
    log_to_stdout: bool,
    /// Whether to lgo to a file.
    log_to_file: bool,
}

impl Default for FlorestaBuilder {
    fn default() -> Self {
        // Don't use AssumeValid by default.
        let assume_valid_default: AssumeValidArg = AssumeValidArg::Disabled;

        // Use `Network::Bitcoin` by default.
        let network_default: Network = Network::Bitcoin;

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

        // The default behaviour for the backfill job. Default to `false`.
        let backfill_default: bool = false;

        // The default user agent for P2P communications.
        let user_agent_default = env!("USER_AGENT").to_string();

        Self {
            assume_valid: assume_valid_default,
            config: UtreexoNodeConfig {
                network: network_default,
                pow_fraud_proofs: false,
                // TODO(@luisschwab): set this to true when implementing wallet
                // rescan with CBF.
                compact_filters: false,
                // TODO(@luisschwab): set a good bridge as a fixed peer?
                fixed_peer: None,
                max_banscore: 10,
                max_outbound: 100,
                max_inflight: 100,
                datadir: data_dir_default,
                proxy: None,
                assume_utreexo: Some(assume_utreexo_default),
                backfill: backfill_default,
                filter_start_height: None,
                user_agent: user_agent_default,
                allow_v1_fallback: true,
                disable_dns_seeds: false,
            },
            debug: false,
            log_to_stdout: true,
            log_to_file: true,
        }
    }
}

impl FlorestaBuilder {
    /// Instantiate a new [`FlorestaNode`] with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Instantiate a new or set an existing [`FlorestaNode`] with a new config.
    ///
    /// TODO(@luisschwab): what happens if we set a config to an already running
    /// node?
    pub fn with_config(mut self, config: UtreexoNodeConfig) -> Self {
        self.config = config;

        // Set the custom `bdk_floresta` user agent.
        self.config.user_agent = env!("USER_AGENT").to_string();

        self
    }

    /// Set the log level to debug.
    pub fn set_debug(mut self) -> Self {
        self.debug = true;
        self
    }

    /// Build the [`FlorestaNode`] from parameters.
    pub async fn build(self) -> Result<FlorestaNode, BuilderError> {
        // Create the data directory.
        let _ = fs::create_dir_all(&self.config.datadir)
            .map_err(BuilderError::CreateDirectory);

        // Setup the subscriber to tracing events, logging. Returns a guard
        // for the logger, which must be kept for the lifetime of
        // [`FlorestaNode`].
        let _logger_guard: Option<WorkerGuard> = setup_logger(
            &self.config.datadir,
            self.log_to_file,
            self.log_to_stdout,
            self.debug,
        )?;

        // Create configuration for the chain store.
        let chain_store_cfg: FlatChainStoreConfig = FlatChainStoreConfig::new(
            self.config.datadir.clone() + "/chaindata",
        );

        // Try to load an existing chain store from disk, or create a new chain
        // store.
        let chain_store: FlatChainStore =
            match FlatChainStore::new(chain_store_cfg) {
                Ok(store) => store,
                Err(e) => {
                    error!("failed to open chain store: {:?}", e);
                    return Err(BuilderError::ChainStoreInit(e));
                }
            };

        // Build the chain state.
        let chain_state: Arc<ChainState<FlatChainStore>> =
            Arc::new(ChainState::new(
                chain_store,
                self.config.network,
                self.assume_valid,
            ));

        info!("initialized chainstore at {}", self.config.datadir);

        // Create a signal that keeps track of whether [`FlorestaNode`] should
        // stop.
        let stop_signal: Arc<RwLock<bool>> = Arc::new(RwLock::new(false));

        let (sender, _) = oneshot::channel();

        // Create a `Pollard` accumulator to be used for our transaction's
        // mempool.
        let pollard_acc: Pollard<BitcoinNodeHash> = Pollard::new();
        let mempool: Arc<Mutex<Mempool>> =
            Arc::new(Mutex::new(Mempool::new(pollard_acc, 1_000_000)));

        let node = UtreexoNode::<_, RunningNode>::new(
            self.config.clone(),
            chain_state.clone(),
            mempool,
            None, /* TODO(@luisschwab): update this once CBF scanning is
                   * implemented. */
            stop_signal.clone(),
            AddressMan::default(),
        )
        .expect("failed to create node");

        info!("crated bdk_floresta node");

        // Get a handle from [`FlorestaNode`], used to send commands to it.
        let node_handle: NodeInterface = node.get_handle();

        // Spawn a task for [`FlorestaNode`].
        let node_task: JoinHandle<()> = task::spawn(node.run(sender));

        // Spawn a task for the SIGINT handler.
        let sigint_task: JoinHandle<()> = {
            let stop_signal: Arc<RwLock<bool>> = stop_signal.clone();
            let chain_state: Arc<ChainState<FlatChainStore>> =
                chain_state.clone();

            task::spawn(async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("failed to initialize SIGINT handler");

                info!("received SIGINT, initiating graceful shutdown");

                // Flush chain to disk
                if let Err(e) = chain_state.flush() {
                    error!("failed to flush chain to disk: {e}");
                } else {
                    info!("flushed chainstate to disk");
                }

                // Set `stop_signal` so the node does a graceful shutdown.
                *stop_signal.write().await = true;
                info!("stop signal set");
            })
        };

        Ok(FlorestaNode {
            _node_config: self.config,
            _debug: self.debug,
            chain_state,
            node_handle,
            task_handle: Some(node_task),
            sigint_task: Some(sigint_task),
            stop_signal,
            _logger_guard,
        })
    }
}
