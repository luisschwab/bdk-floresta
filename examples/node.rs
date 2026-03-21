use std::path::PathBuf;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bitcoin::Network;
use tokio::runtime;

const DATA_DIR: &str = "./examples/data/node/";
const NETWORK: Network = Network::Signet;

fn main() -> anyhow::Result<()> {
    // Define the node's configuration.
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(format!("{}{}", DATA_DIR, NETWORK)),
        ..Default::default()
    };

    // Build a runtime for the node to run on.
    let rt = runtime::Builder::new_multi_thread().enable_all().build()?;

    // Use the builder to build the node from the configuration.
    let mut node = Builder::new().from_config(config).build()?;

    // Run the node inside the runtime.
    rt.block_on(async {
        node.run().await?;
        node.cancelled().await;
        anyhow::Ok(())
    })?;

    drop(rt);
    Ok(())
}
