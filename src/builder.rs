//! This module implements [`FlorestaClientBuilder`].

use std::sync::Arc;

use anyhow::Result;
use bitcoin::Network;
use floresta_chain::AssumeValidArg;
use futures_channel::oneshot;
use log::info;
use rustreexo::accumulator::pollard::Pollard;
use tokio::sync::RwLock;
use tokio::task;

use floresta_chain::{
    pruned_utreexo::{
        flat_chain_store::{FlatChainStore, FlatChainStoreConfig},
        UpdatableChainstate,
    },
    AssumeValidArg, BlockchainError, ChainParams, ChainState,
};
use floresta_wire::{
    address_man::AddressMan, mempool::Mempool, node::UtreexoNode, running_node::RunningNode,
    UtreexoNodeConfig,
};

use crate::logger;
use crate::FlorestaClient;

pub struct FlorestaClientBuilder {
    config: UtreexoNodeConfig,
    debug: bool,
}

impl Default for FlorestaClientBuilder {
    fn default() -> Self {
        let assume_utreexo_value =
            ChainParams::get_assume_utreexo(Network::Bitcoin).expect("Unsupported network");

        Self {
            config: UtreexoNodeConfig {
                disable_dns_seeds: false,
                network: Network::Bitcoin,
                datadir: format!("./data/{}", Network::Bitcoin),
                compact_filters: false,
                filter_start_height: None,
                assume_utreexo: Some(assume_utreexo_value),
                pow_fraud_proofs: false,
                backfill: true,
                user_agent: String::from("floresta-wire"),
                allow_v1_fallback: true,
                fixed_peer: None,
                proxy: None,
                max_inflight: 10,
                max_outbound: 10,
                max_banscore: 100,
            },
            debug: false,
        }
    }
}

impl FlorestaClientBuilder {
    /// Initialize a [`FlorestaClient`] with the default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize a [`FlorestaClient`] with a custom [`UtreexoNodeConfig`] configuration.
    pub fn with_config(mut self, config: UtreexoNodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Set a custom network to a [`FlorestaClient`].
    pub fn network(mut self, network: bitcoin::Network) -> Self {
        self.config.network = network;
        self.config.datadir = format!("./data/{network}");
        self
    }

    /// Set the log-level to debug.
    pub fn debug(mut self) -> Self {
        self.debug = true;
        self
    }

    /// Build the [`FlorestaClient`].
    pub async fn build(self) -> Result<FlorestaClient> {
        logger::setup_logger(self.debug)?;

        // TODO(@luisschwab): https://github.com/bitcoindevkit/bdk/pull/1582/
        let chain_store_cfg = FlatChainStoreConfig::new(self.config.datadir.clone());
        let chain_store = FlatChainStore::new(chain_store_cfg.clone()).expect("");
        let chain = Arc::new(
            match ChainState::<FlatChainStore>::load_chain_state(
                chain_store,
                self.config.network,
                AssumeValidArg::Disabled,
            ) {
                Ok(chainstate) => {
                    info!("restored chain data persisted at {}", self.config.datadir);
                    chainstate
                }
                Err(err) => match err {
                    BlockchainError::ChainNotInitialized => {
                        let chain_store =
                            FlatChainStore::new(chain_store_cfg).expect("could not read DB");

                        info!("created a new chain on disk at {}", self.config.datadir);
                        ChainState::<FlatChainStore>::new(
                            chain_store,
                            self.config.network,
                            AssumeValidArg::Disabled,
                        )
                    }
                    _ => unreachable!(),
                },
            },
        );

        let kill_signal: Arc<RwLock<bool>> = Arc::new(RwLock::new(false));
        let (sender, _) = oneshot::channel();

        // Pollard is an implementation of the Utreexo accumulator.
        let acc = Pollard::new();

        info!("creating bdk_floresta node");
        let node = UtreexoNode::<_, RunningNode>::new(
            self.config.clone(),
            chain.clone(),
            Arc::new(tokio::sync::Mutex::new(Mempool::new(acc, 300_000_000))),
            None,
            kill_signal.clone(),
            AddressMan::default(),
        )
        .map_err(|e| anyhow::anyhow!("could not create node: {:?}", e))?;

        // Get the node's handle, used to send commands to it.
        let handle = node.get_handle();

        info!("starting bdk_floresta on {}", self.config.network);
        let node_task = task::spawn(node.run(sender));

        // Start the SIGINT handler task
        let sigint_task = {
            let kill_signal = kill_signal.clone();
            let chain = chain.clone();
            Some(task::spawn(async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("failed to initialize SIGINT handler");

                info!("received SIGINT, stopping bdk_floresta");
                info!("flushing chain to disk");
                let _ = chain.flush();

                let mut kill = kill_signal.write().await;
                *kill = true;
            }))
        };

        Ok(FlorestaClient {
            config: self.config,
            debug: self.debug,
            chain,
            handle,
            kill_signal,
            node_task: Some(node_task),
            sigint_task,
        })
    }
}
