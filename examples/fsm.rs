// SPDX-License-Identifier: MIT OR Apache-2.0

//! An example showcasing the [`Node`]'s [`State`] updating logic.

use core::net::SocketAddr;
use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bdk_floresta::fsm::State;
use bdk_floresta::logger::Logger;
use bdk_floresta::logger::LOG_FILE;
use bitcoin::Network;
use tracing::info;
use tracing::Level;

const UTREEXOD_CASA21: &str = "189.44.63.101:38333";
const NETWORK: Network = Network::Signet;
const DATA_DIR: &str = "./examples/data/fsm/";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Only display the example's own logs
    env::set_var("RUST_LOG", "fsm=info");

    // Set up the logger
    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: Some(PathBuf::from(DATA_DIR).join("bdk_floresta").join(LOG_FILE)),
    }
    .init()?;

    // Configure the node
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(DATA_DIR).join("bdk_floresta"),
        fixed_peer: Some(SocketAddr::from_str(UTREEXOD_CASA21)?),
        ..Default::default()
    };

    // Instantiate and run the node
    info!("> Instantiating the node...");
    let mut node = Builder::new().from_config(config).build()?;
    info!("> NODE STATE: {}", node.get_state());

    info!("> Spawning the node...");
    node.run().await?;
    info!("> Node spawned");

    // Subscribe to state transitions
    let mut state_rx = node.subscribe_state();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("> /exit");
        }
        result = async {
            // Print every state transition until we hit Operational
            loop {
                let state = state_rx.borrow_and_update().clone();
                info!("> NODE STATE: {}", state);
                if state == State::Operational {
                    break;
                }
                if state_rx.changed().await.is_err() {
                    // Channel closed — node is gone.
                    break;
                }
            }
            info!("> /exit");
            Ok::<_, anyhow::Error>(())
        } => {
            result?;
        }
    }

    node.shutdown().await?;
    info!("> NODE STATE: {}", node.get_state());

    Ok(())
}
