// SPDX-License-Identifier: MIT OR Apache-2.0

//! An example showcasing the [`Node`]'s [`State`] updating logic.

use core::net::SocketAddr;
use std::env;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

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
const STATUS_POLL_PERIOD: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // On display the example's logs
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
    info!("> NODE STATE: {}", node.get_state().await);

    info!("> Spawning the node...");
    node.run().await?;
    info!("> Node spawned");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
                info!("> /exit");
                node.shutdown().await?;
                return Ok(());
        }
        result = async {
            while node.get_state().await != State::Operational {
                tokio::time::sleep(STATUS_POLL_PERIOD).await;
                info!("> NODE STATE: {}", node.get_state().await);
            }

            info!("> NODE STATE: {}", node.get_state().await);

            info!("> /exit");
            node.shutdown().await?;

            info!("> NODE STATE: {}", node.get_state().await);

            Ok::<_, anyhow::Error>(())
        } => {
            result?;
        }
    }

    Ok(())
}
