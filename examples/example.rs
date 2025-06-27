use std::{net::SocketAddr, str::FromStr, sync::Arc};

use anyhow::Result;
use bitcoin::{Block, Network};
use log::info;

use bdk_floresta::{builder::FlorestaClientBuilder, FlorestaClient};
use bdk_floresta::{BlockConsumer, BlockchainInterface, UpdatableChainstate};

/// TODO: remove this
struct BlockPrinter;
impl BlockConsumer for BlockPrinter {
    fn consume_block(&self, _block: &Block, height: u32) {
        info!("new block @ {}!", height);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let network = Network::Signet;

    let mut client = FlorestaClientBuilder::default()
        .network(Network::Signet)
        .build()
        .await?;

    bootstrap_utreexo_peers(&client, &network).await;

    // Create the block consumer
    let block_printer = Arc::new(BlockPrinter);
    // Subscribe to new blocks.
    client.subscribe_block(block_printer);

    let mut i = 0;
    loop {
        tokio::select! {
            _ = &mut client.sigint_task.as_mut().unwrap() => {
                client.shutdown().await?;
                break;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                if client.chain.is_in_ibd() {
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
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Bootstrap to known Utreexo-capable peers.
async fn bootstrap_utreexo_peers(client: &FlorestaClient, network: &Network) {
    match network {
        Network::Bitcoin => {
            let _ = client
                .add_peer(SocketAddr::from_str("1.228.21.110:8333").unwrap())
                .await;
            let _ = client
                .add_peer(SocketAddr::from_str("181.191.0.133:8333").unwrap())
                .await;
            let _ = client
                .add_peer(SocketAddr::from_str("85.239.240.4:8333").unwrap())
                .await;
        }
        Network::Signet => {
            let _ = client
                .add_peer(SocketAddr::from_str("209.126.80.42:39333").unwrap())
                .await;
            let _ = client
                .add_peer(SocketAddr::from_str("1.228.21.110:38333").unwrap())
                .await;
            let _ = client
                .add_peer(SocketAddr::from_str("85.239.240.4:38333").unwrap())
                .await;
        }
        Network::Testnet4 => {
            let _ = client
                .add_peer(SocketAddr::from_str("85.239.240.4:48333").unwrap())
                .await;
        }
        _ => panic!("Unsupported network"),
    }
}
