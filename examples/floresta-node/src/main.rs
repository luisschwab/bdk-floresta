//! Basic example of creating and operating a [`FlorestaNode`].

use std::{net::SocketAddr, str::FromStr};

use bdk_floresta::{builder::FlorestaBuilder, UtreexoNodeConfig};
use bdk_wallet::bitcoin::Network;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a custom configuration for the node using `Network::Signet`.
    let node_config = UtreexoNodeConfig {
        network: Network::Signet,
        datadir: String::from("./data/example-node/signet"),
        ..Default::default()
    };

    // Build a [`FlorestaNode`] with a custom configuration.
    let floresta_node = FlorestaBuilder::new()
        .with_config(node_config)
        .build()
        .await?;

    // Connect to known Utreexo bridges.
    let bridge_alpha = SocketAddr::from_str("195.26.240.213:38433").unwrap(); // luisschwab
    floresta_node.connect_peer(&bridge_alpha).await?;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Check if the node should stop.
        if floresta_node.should_stop().await {
            info!("Detected SIGINT, stopping node");
            break;
        }

        // Check the chain height.
        let height = floresta_node.get_height()?;
        info!("height: {}", height);

        // Check the validation height.
        let val_height = floresta_node.get_validation_height()?;
        info!("validated height: {}", val_height);

        // Check peer status.
        let peers = floresta_node.get_peer_info().await?;
        let peer_addrs: Vec<String> =
            peers.iter().map(|peer| peer.address.clone()).collect();
        info!("peers: {:?}", peer_addrs);
    }

    floresta_node.shutdown().await?;
    Ok(())
}
