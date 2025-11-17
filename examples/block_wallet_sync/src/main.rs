use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use bdk_floresta::{builder::FlorestaBuilder, UtreexoNodeConfig, WalletUpdate};

use bdk_wallet::bitcoin::Network;
use bdk_wallet::Wallet;
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use tracing::{error, info};

const NETWORK: Network = Network::Regtest;
const DATA_DIR: &str = "./examples/block_wallet_sync/data/";
const DESC_EXTERNAL: &str = "wpkh([925d79b3/84h/1h/0h]tpubDCjEJ6uoRerKY5Wdj2bPdcBDdhgLj6M6nnFNH87gyYRY6FbUqGWQK8WpQt2vtT3xyrqirjmgCGoSaZgoGVecnYouMfXNNHZAQtekm88fE4m/0/*)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a wallet.
    let wallet: Wallet = Wallet::create_single(DESC_EXTERNAL)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    // Create a custom configuration for the node.
    let node_config: UtreexoNodeConfig = UtreexoNodeConfig {
        network: NETWORK,
        datadir: format!("{}{}", DATA_DIR, NETWORK),
        ..Default::default()
    };

    // Build a [`FlorestaNode`] with a custom configuration.
    let mut node = FlorestaBuilder::new()
        .with_config(node_config)
        .with_wallet(wallet)
        .build()
        .await?;

    // Connect to local bridges
    let bridge_a: SocketAddr = SocketAddr::from_str("127.0.0.1:18333")?;
    node.connect_peer(&bridge_a).await?;
    let bridge_b: SocketAddr = SocketAddr::from_str("127.0.0.1:28333")?;
    node.connect_peer(&bridge_b).await?;
    let bridge_c: SocketAddr = SocketAddr::from_str("127.0.0.1:38333")?;
    node.connect_peer(&bridge_c).await?;

    let wallet_arc: Option<Arc<RwLock<Wallet>>> = node.wallet.clone();
    let mut update_subscriber: UnboundedReceiver<WalletUpdate> = node
        .update_subscriber
        .take()
        .expect("update subscriber should be present");

    tokio::spawn(async move {
        while let Some(update) = update_subscriber.recv().await {
            if let Some(wallet) = &wallet_arc {
                let mut wallet = wallet.write().await;

                // Perform the related action on the
                // `Wallet` based on the `WalletUpdate` variant.
                match update {
                    WalletUpdate::NewBlock(block, height) => {
                        let res = wallet.apply_block(&block, height);
                        match res {
                            Ok(_) => {
                                info!(
                                    "Sucessfully applied block at height {} to the wallet [ Balance: {} sats ]",
                                    height, wallet.balance().total().to_sat()
                                );
                            }
                            Err(e) => error!("Failed to apply block at height {} to the wallet: {}", height, e),
                        }
                    }
                }
            }
        }
    });

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Check if the node should stop.
        if node.should_stop().await {
            info!("Detected SIGINT, stopping node");
            break;
        }
    }

    node.shutdown().await?;
    Ok(())
}
