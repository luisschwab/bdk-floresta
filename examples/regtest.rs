// SPDX-License-Identifier: MIT OR Apache-2.0

//! IBD integration test between `bdk_floresta`, `utreexod` and `bitcoind` on regtest.

use std::path::PathBuf;
use std::time::Duration;

use bdk_floresta::builder::NodeConfig;
use bdk_floresta::logger::Logger;
use bdk_floresta::Builder;
use bitcoin::Network;
use halfin::bitcoind::BitcoinD;
use halfin::utreexod::UtreexoD;
use halfin::wait_for_height;
use tracing::info;
use tracing::Level;

const BLOCK_COUNT: u32 = 144;
const NETWORK: Network = Network::Regtest;
const DATA_DIR: &str = "./examples/data/regtest_ibd/";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up the logger
    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: None,
    }
    .init()?;

    // Spawn a BitcoinD
    info!("> Spawning BitcoinD...");
    let bitcoind = BitcoinD::new()?;
    info!("> BitcoinD spawned");

    // Mine blocks.
    info!("> Mining {} blocks...", BLOCK_COUNT);
    let hashes = bitcoind.generate(BLOCK_COUNT)?;
    info!(
        "Mined {} blocks [{}..{}]",
        BLOCK_COUNT,
        &hashes.first().unwrap().to_string()[..8],
        &hashes.last().unwrap().to_string()[..8],
    );

    // Spawn an UtreexoD
    info!("> Spawning UtreexoD...");
    let utreexod = UtreexoD::new()?;
    info!("> UtreexoD spawned");

    // Connect BitcoinD and UtreexoD
    info!("> Connecting BitcoinD and UtreexoD... ");
    utreexod.add_peer(bitcoind.get_p2p_socket())?;
    assert_eq!(utreexod.get_peer_count()?, 1);
    info!("> BitcoinD and UtreexoD are connected");

    // Wait for UtreexoD to catch up
    info!("> Waiting for UtreexoD to catch up...");
    wait_for_height(&utreexod, BLOCK_COUNT)?;
    info!("> UtreexoD catched up");

    // Configure the bdk_floresta node
    let node_config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(DATA_DIR),
        ..Default::default()
    };

    // Instantiate and run bdk_floresta
    info!("> Spawning bdk_floresta...");
    let mut node = Builder::new().from_config(node_config).build()?;
    node.run().await?;
    info!("> bdk_floresta is spawned");
    std::thread::sleep(Duration::from_secs(2));

    // Connect bdk_floresta to BitcoinD
    info!("> Connecting BitcoinD and bdk_floresta");
    node.add_peer(&bitcoind.get_p2p_socket()).await?;
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(node.get_peer_info().await?.len(), 1);
    info!("> BitcoinD and bdk_floresta are connected");

    // Connect bdk_floresta to UtreexoD
    info!("> Connecting bdk_floresta and BitcoinD...");
    node.add_peer(&utreexod.get_p2p_socket()).await?;
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(node.get_peer_info().await?.len(), 2);

    // Wait for bdk_floresta to catch up
    while node.get_validation_height()? < BLOCK_COUNT {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Print bdk_floresta's peer information
    let peer_info = node.get_peer_info().await?;
    info!("> Connected peers ({}):", peer_info.len());
    for (i, peer) in peer_info.iter().enumerate() {
        info!("  > Peer {}:", i);
        info!("    > User Agent: {}", peer.user_agent);
        info!("    > Socket: {}", peer.address);
        info!("    > Transport Protocol: {:?}", peer.transport_protocol);
        info!("    > Services: {}", peer.services);
    }

    // Get the the hash of the block of arbitrary height from the node's chainstate
    let block_hash = node.get_block_hash(BLOCK_COUNT)?;
    info!("> Hash of the block at the tip: {}", block_hash);

    // Fetch a block of arbitrary height from a peer and show it's header
    let block = node.fetch_block(block_hash).await?.unwrap();
    info!("> Header of the block at the tip: {:#?}", block.header);

    // Get bdk_floresta's Utreexo accumulator state
    let stump = node.get_accumulator()?;
    info!("> bdk_floresta accumulator state: {:?}", stump);

    // TODO(@luisschwab): mine more blocks and have bdk_floresta sync up

    info!("> /exit");
    node.shutdown().await?;

    Ok(())
}
