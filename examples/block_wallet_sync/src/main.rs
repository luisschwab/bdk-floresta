use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use bdk_floresta::{builder::FlorestaBuilder, UtreexoNodeConfig, WalletUpdate};

use bdk_wallet::bitcoin::{BlockHash, Network};
use bdk_wallet::Wallet;
use floresta_chain::AssumeValidArg;
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use tracing::{error, info};

const DESC_EXTERNAL: &str = "wpkh([9cee26c8/84'/1'/0']tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/0/*)";
const DESC_INTERNAL: &str = "wpkh([9cee26c8/84'/1'/0']tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/1/*)";

const DATA_DIR: &str = "./examples/block_wallet_sync/data/";
const NETWORK: Network = Network::Signet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a wallet.
    let wallet: Wallet = Wallet::create(DESC_EXTERNAL, DESC_INTERNAL)
        .network(NETWORK)
        .create_wallet_no_persist()?;

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

    // Build a [`FlorestaNode`] with a custom configuration.
    let mut node = FlorestaBuilder::new()
        .with_config(node_config)
        .with_assumevalid(assume_valid)
        .with_wallet(wallet)
        .build_logger()
        .build()
        .await?;

    // For now, manually connect to known Utreexo bridges.
    let bridge_a: SocketAddr = SocketAddr::from_str("195.26.240.213:38433")?; // LS
    let bridge_b: SocketAddr = SocketAddr::from_str("194.163.132.180:38333")?; // DS
    let bridge_c: SocketAddr = SocketAddr::from_str("161.97.178.61:38333")?; // DS
    node.connect_peer(&bridge_a).await?;
    node.connect_peer(&bridge_b).await?;
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
                                info!("Sucessfully applied block at height {} to the wallet [ Balance: {} sats ]", height, wallet.balance().total().to_sat());
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
            break;
        }
    }

    node.shutdown().await?;
    Ok(())
}
