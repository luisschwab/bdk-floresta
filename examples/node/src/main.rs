use std::path::PathBuf;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bitcoin::Network;

const DATA_DIR: &str = "./examples/node/data/";
const NETWORK: Network = Network::Signet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Define the node's configuration.
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(format!("{}{}", DATA_DIR, NETWORK)),
        ..Default::default()
    };

    // Use the builder to build the node from the configuration.
    let mut node = Builder::new().from_config(config).build()?;

    // Run the node.
    node.run().await?;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Check if the node should stop.
        if node.should_stop().await {
            break;
        }
    }

    node.shutdown().await?;
    Ok(())
}
