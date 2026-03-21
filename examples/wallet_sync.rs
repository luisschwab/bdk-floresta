use std::path::PathBuf;
use std::sync::Arc;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bdk_floresta::WalletUpdate;
use bdk_wallet::Wallet;
use bitcoin::Network;
use tokio::runtime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;

const DESC_EXTERNAL: &str = "wpkh([9cee26c8/84'/1'/0']tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/0/*)";
const DESC_INTERNAL: &str = "wpkh([9cee26c8/84'/1'/0']tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/1/*)";

const DATA_DIR: &str = "./examples/data/wallet_sync/";
const NETWORK: Network = Network::Signet;

fn main() -> anyhow::Result<()> {
    // Create a wallet that will receive updates from the node.
    let wallet: Wallet = Wallet::create(DESC_EXTERNAL, DESC_INTERNAL)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    // Define the node's configuration.
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(format!("{}{}", DATA_DIR, NETWORK)),
        ..Default::default()
    };
    // Build a runtime for the node to run on.
    let rt = runtime::Builder::new_multi_thread().enable_all().build()?;

    // Use the builder to build the node from the configuration.
    let mut node = Builder::new()
        .from_config(config)
        .with_wallet(wallet)
        .build()?;

    // Run the node and wait for shutdown (CTRL-C or node.shutdown()),
    // and run the block update subscriber.
    rt.block_on(async {
        // Create the wallet's update subscriber.
        let wallet_arc: Option<Arc<RwLock<Wallet>>> = node.wallet.clone();
        let mut update_subscriber: UnboundedReceiver<WalletUpdate> = node
            .update_subscriber
            .take()
            .expect("update subscriber should be present");

        // Subscribe to new blocks and apply them to the wallet.
        tokio::spawn(async move {
            while let Some(update) = update_subscriber.recv().await {
                if let Some(wallet) = &wallet_arc {
                    let mut wallet = wallet.write().await;
                    match update {
                        WalletUpdate::NewBlock(block, height) => {
                            let res = wallet.apply_block_events(&block, height);
                            match res {
                                Ok(events) if !events.is_empty() => {
                                    info!(
                                        "Block {} | Balance: {} sats",
                                        height,
                                        wallet.balance().total().to_sat(),
                                    );
                                }
                                Ok(_) => {}
                                Err(e) => error!(
                                    "Failed to apply block at height {} to the wallet: {}",
                                    height, e
                                ),
                            }
                        }
                    }
                }
            }
        });
        node.run().await?;
        node.cancelled().await;
        anyhow::Ok(())
    })?;

    drop(rt);
    Ok(())
}
