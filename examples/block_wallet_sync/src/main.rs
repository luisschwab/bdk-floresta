//! Basic example of creating and operating a [`FlorestaNode`].

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use bdk_floresta::{builder::FlorestaBuilder, UtreexoNodeConfig, WalletUpdate};

use bdk_wallet::bitcoin::Network;
use bdk_wallet::Wallet;
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use tracing::{error, info};

const DESC_EXTERNAL: &str = "wpkh([9cee26c8/84'/1'/0']tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/0/*)";
const DESC_INTERNAL: &str = "wpkh([9cee26c8/84'/1'/0']tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/1/*)";

const NETWORK: Network = Network::Signet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a wallet.
    let wallet: Wallet = Wallet::create(DESC_EXTERNAL, DESC_INTERNAL)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    // Create a custom configuration for the node using `Network::Signet`.
    let node_config: UtreexoNodeConfig = UtreexoNodeConfig {
        network: NETWORK,
        datadir: format!("{}{}", "./data/", NETWORK),
        ..Default::default()
    };

    // Build a [`FlorestaNode`] with a custom configuration.
    let mut node = FlorestaBuilder::new()
        .with_config(node_config)
        .with_wallet(wallet)
        .build()
        .await?;

    // Connect to known Utreexo bridges.
    let bridge_a: SocketAddr = SocketAddr::from_str("195.26.240.213:38433")?;
    node.connect_peer(&bridge_a).await?;

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
                                info!("Sucessfully applied block at height {} to the wallet", height);
                                info!("Wallet Balance: {}", wallet.balance().total());
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
