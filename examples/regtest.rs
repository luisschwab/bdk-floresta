// SPDX-License-Identifier: MIT OR Apache-2.0

//! Regtest integration test between [`bdk_floresta`], [`bitcoind`] and [`utreexod`].
//!
//! ## Network Topology
//!
//! ```text
//!  ┌──────────┐   P2P    ┌──────────┐        P2P        ┌──────────────┐
//!  │ BitcoinD │ <======> │ UtreexoD │ <===============> │ bdk_floresta │
//!  └──────────┘  blocks  └──────────┘  blocks + proofs  └──────────────┘
//! ```
//!
//! 1. [`bitcoind`] mines blocks and propagates them to [`utreexod`].
//! 2. [`utreexod`] receives these blocks and builds Utreexo Merkle proofs for these blocks.
//! 3. [`bdk_floresta`] requests blocks and Utreexo Merkle proofs (`uproof`) from [`utreexod`].
//!
//! [`bdk_floresta`]: bdk_floresta::Node
//! [`bitcoind`]: halfin::BitcoinD
//! [`utreexod`]: halfin::UtreexoD

use std::path::PathBuf;
use std::time::Duration;

use bdk_floresta::builder::NodeConfig;
use bdk_floresta::logger::Logger;
use bdk_floresta::Builder;
use bitcoin::Network;
use halfin::bitcoind::BitcoinD;
use halfin::bitcoind::BitcoinDConf;
use halfin::utreexod::UtreexoD;
use halfin::utreexod::UtreexoDConf;
use halfin::wait_for_height;
use tracing::info;
use tracing::Level;

const BLOCK_COUNT: u32 = 144;
const NETWORK: Network = Network::Regtest;
const DATA_DIR: &str = "./examples/data/regtest/";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up the logger
    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: None,
    }
    .init()?;

    // Configure and spawn a BitcoinD
    info!("> Spawning BitcoinD...");
    let bitcoind_conf = BitcoinDConf {
        staticdir: Some(PathBuf::from(DATA_DIR).join("bitcoind")),
        ..Default::default()
    };
    let bitcoind = BitcoinD::new_with_conf(&bitcoind_conf)?;
    info!("> Spawned BitcoinD");

    // Mine the first batch of blocks
    info!("> Mining {} blocks...", BLOCK_COUNT);
    let hashes = bitcoind.generate(BLOCK_COUNT)?;
    info!(
        "> Mined {} blocks [{}..{}]",
        BLOCK_COUNT,
        &hashes.first().unwrap().to_string()[..8],
        &hashes.last().unwrap().to_string()[..8],
    );

    // Configure and spawn an UtreexoD
    info!("> Spawning UtreexoD...");
    let utreexod_conf = UtreexoDConf {
        staticdir: Some(PathBuf::from(DATA_DIR).join("utreexod")),
        ..Default::default()
    };
    let utreexod = UtreexoD::new_with_conf(&utreexod_conf)?;
    info!("> Spawned UtreexoD");

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
        data_directory: PathBuf::from(DATA_DIR).join("bdk_floresta").join("regtest"),
        ..Default::default()
    };

    // Instantiate and run bdk_floresta
    info!("> Spawning bdk_floresta...");
    let mut node = Builder::new().from_config(node_config).build()?;
    node.run().await?;
    info!("> bdk_floresta is spawned");
    std::thread::sleep(Duration::from_secs(2));

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("> /exit");
            node.shutdown().await?;
            return Ok(());
        }
        result = async {
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

            // Wait for bdk_floresta to catch up to the first batch
            info!("> Waiting for bdk_floresta to catch up to block {}...", BLOCK_COUNT);
            while node.get_node_height()? < BLOCK_COUNT {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            info!("> bdk_floresta caught up to block {}", BLOCK_COUNT);

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

            // Get the hash of the block at the tip
            let block_hash = node.get_block_hash(BLOCK_COUNT)?;
            info!("> Hash of the block at the tip: {}", block_hash);

            // Fetch the block at the tip from a peer and show its header
            let block = node.fetch_block(block_hash).await?;
            info!("> Header of the block at the tip: {:#?}", block.header);

            // Get bdk_floresta's Utreexo accumulator state
            let stump = node.get_accumulator();
            info!("> bdk_floresta accumulator state: {:?}", stump);

            /*

            // Doesn't work: bdk_floresta won't sync the new
            // blocks after it switches from IBD to running node

            // Mine a second batch of blocks
            info!("> Mining {} more blocks...", BLOCK_COUNT);
            let hashes = bitcoind.generate(BLOCK_COUNT)?;
            info!(
                "> Mined {} blocks [{}..{}]",
                BLOCK_COUNT,
                &hashes.first().unwrap().to_string()[..8],
                &hashes.last().unwrap().to_string()[..8],
            );

            // Wait for UtreexoD to catch up to the second batch
            info!("> Waiting for UtreexoD to catch up to block {}...", 2 * BLOCK_COUNT);
            wait_for_height(&utreexod, 2 * BLOCK_COUNT)?;
            info!("> UtreexoD caught up to block {}", 2 * BLOCK_COUNT);

            // Configure and spawn a second UtreexoD
            info!("> Spawning a second UtreexoD...");
            let utreexod_2_conf = UtreexoDConf {
                staticdir: Some(PathBuf::from(DATA_DIR).join("utreexod_2")),
                ..Default::default()
            };
            let utreexod_2 = UtreexoD::new_with_conf(&utreexod_2_conf)?;
            info!("> Second UtreexoD spawned");

            // Connect BitcoinD and second UtreexoD
            info!("> Connecting BitcoinD and the second UtreexoD... ");
            utreexod_2.add_peer(bitcoind.get_p2p_socket())?;
            assert_eq!(utreexod_2.get_peer_count()?, 1);
            info!("> BitcoinD and second UtreexoD are connected");

            // Wait for the second UtreexoD to catch up
            info!("> Waiting for the second UtreexoD to catch up to block {}...", 2 * BLOCK_COUNT);
            wait_for_height(&utreexod_2, 2 * BLOCK_COUNT)?;
            info!("> Second UtreexoD caught up to block {}", 2 * BLOCK_COUNT);

            // Connect bdk_floresta and the second UtreexoD
            node.add_peer(&utreexod_2.get_p2p_socket()).await?;

            // Give peers time to propagate the new tip to bdk_floresta
            tokio::time::sleep(Duration::from_secs(5)).await;

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

            // Wait for bdk_floresta to catch up to the second batch
            info!("> Waiting for bdk_floresta to catch up to block {}...", 2 * BLOCK_COUNT);
            while node.get_node_height()? < 2 * BLOCK_COUNT {
                tokio::time::sleep(Duration::from_secs(3)).await;
                info!("> Current Height: {}", node.get_chain_height()?);
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
            info!("> bdk_floresta caught up to block {}", 2 * BLOCK_COUNT);

            // Get the hash of the block at the new tip
            let block_hash = node.get_block_hash(2 * BLOCK_COUNT)?;
            info!("> Hash of the block at the tip: {}", block_hash);

            // Fetch the block at the new tip from a peer and show its header
            let block = node.fetch_block(block_hash).await?.unwrap();
            info!("> Header of the block at the tip: {:#?}", block.header);

            // Get bdk_floresta's updated Utreexo accumulator state
            let stump = node.get_accumulator();
            info!("> bdk_floresta accumulator state: {:?}", stump);

            */

            info!("> /exit");
            node.shutdown().await?;

            Ok::<_, anyhow::Error>(())
        } => {
            result?;
        }
    }

    Ok(())
}
