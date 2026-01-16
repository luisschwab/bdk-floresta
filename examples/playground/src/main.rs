use std::str::FromStr;

use bdk_floresta::builder::Builder;
use bdk_floresta::UtreexoNodeConfig;
use bitcoin::BlockHash;
use bitcoin::Network;
use floresta_chain::AssumeValidArg;

const DATA_DIR: &str = "./examples/playground/data/";
const NETWORK: Network = Network::Signet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Block 270_000.
    let assume_valid: AssumeValidArg =
        AssumeValidArg::UserInput(BlockHash::from_str(
            "00000005162ac86112891a296941e965e257d41fb8addeabbb17c6ff88ac840a",
        )?);

    // Create a custom configuration for the node using `Network::Signet`.
    let node_config: UtreexoNodeConfig = UtreexoNodeConfig {
        network: NETWORK,
        datadir: format!("{}{}", DATA_DIR, NETWORK),
        ..Default::default()
    };

    // Build a [`Node`] with a custom configuration.
    let node = Builder::new()
        .from_config(node_config)
        .with_assumevalid(assume_valid)
        .build_logger()
        .build()
        .await?;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Check if the node should stop.
        if node.should_stop().await {
            break;
        }
    }

    node.shutdown().await?;
    Ok(())
}
