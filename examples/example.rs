use std::{net::SocketAddr, str::FromStr, sync::Arc};

use anyhow::Result;
use bitcoin::{Block, Network};
use floresta_chain::{
    pruned_utreexo::{BlockchainInterface, UpdatableChainstate},
    BlockConsumer,
};
use log::info;

use bdk_floresta::builder::FlorestaClientBuilder;

/// TODO: remove this
struct BlockPrinter;
impl BlockConsumer for BlockPrinter {
    fn consume_block(&self, _block: &Block, height: u32) {
        info!("new block @ {}!", height);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = FlorestaClientBuilder::default()
        .network(Network::Bitcoin)
        .build()
        .await?;

    // Create the block consumer
    let block_printer = Arc::new(BlockPrinter);
    // Subscribe to new blocks.
    client.subscribe_block(block_printer);

    if client.config.network == Network::Signet {
        client
            .connect(SocketAddr::from_str("209.126.80.42:39333")?)
            .await?;
        client
            .connect(SocketAddr::from_str("1.228.21.110:38333")?)
            .await?;
    } else if client.config.network == Network::Bitcoin {
        client
            .connect(SocketAddr::from_str("1.228.21.110:8333")?)
            .await?;
        client
            .connect(SocketAddr::from_str("181.191.0.133:8333")?)
            .await?;
    }

    let mut i = 0;
    loop {
        tokio::select! {
            _ = &mut client.sigint_task.as_mut().unwrap() => {
                client.shutdown().await?;
                break;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                if client.chain.is_in_ibd() {
                    info!(
                        "bdk_floresta is still in IBD [headers: {}, validated: {}]",
                        client.get_height().await?,
                        client.get_validation_height().await?
                    );

                    if i % 10 == 0 && i != 0 {
                        let _ = client.chain.flush();
                        info!("flushed chain to disk");
                    }

                    if i % 20 == 0 {
                        let peers = client.handle.get_peer_info().await;
                        let addresses: Vec<String> = peers.unwrap_or_default().iter().map(|peer| peer.address.clone()).collect();
                        info!("peers: {:?}", addresses);
                    }

                    i += 1;
                } else {
                    info!("finished IBD");

                }
            }
        }
    }

    Ok(())
}
