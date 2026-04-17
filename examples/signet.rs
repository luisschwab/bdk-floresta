use std::path::PathBuf;
use std::time::Duration;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bdk_floresta::logger::Logger;
use bdk_floresta::logger::LOG_FILE;
use bitcoin::Network;
use tracing::info;
use tracing::Level;

const NETWORK: Network = Network::Signet;
const DATA_DIR: &str = "./examples/data/signet_ibd/";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up the logger
    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: Some(PathBuf::from(DATA_DIR).join(LOG_FILE)),
    }
    .init()?;

    // Configure the node
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(DATA_DIR),
        ..Default::default()
    };

    // Instantiate and run the node
    info!("> Spawning the node...");
    let mut node = Builder::new().from_config(config).build()?;
    node.run().await?;
    info!("> Node spawned");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
                info!("> Shutting down...");
                node.shutdown().await?;
                return Ok(());
        }
        result = async {
            // Wait for the node to catch up to the chain tip.
            // By default, it will use `AssumeUtreexo` and skip most of it.
            while node.in_ibd() {
                tokio::time::sleep(Duration::from_secs(10)).await;
                info!(
                    "> Waiting for the node to catch up to the chain tip... [{}/{}]",
                    node.get_node_height()?,
                    node.get_chain_height()?,
                );
            }
            info!("> Finished IBD");

            // Get the node's Utreexo accumulator state
            let stump = node.get_accumulator();
            info!("> bdk_floresta accumulator state: {:?}", stump);

            // Print the node's peer information
            let peer_info = node.get_peer_info().await?;
            info!("> Connected peers ({}):", peer_info.len());
            for (i, peer) in peer_info.iter().enumerate() {
                info!("  > Peer {}:", i);
                info!("    > Socket: {}", peer.address);
                info!("    > User Agent: {}", peer.user_agent);
            }

            // Get the hash of the block at the tip
            let tip = node.get_chain_height()?;
            let block_hash = node.get_block_hash(tip)?;
            info!("> Hash of the block at the tip: {}", block_hash);

            // Fetch the block at the tip from a peer
            let block = node.fetch_block(block_hash).await?.unwrap();
            info!("> Header of the block at the tip: {:#?}", block.header);

            info!("> /exit");
            node.shutdown().await?;

            Ok::<_, anyhow::Error>(())
        } => {
            result?;
        }
    }

    Ok(())
}
