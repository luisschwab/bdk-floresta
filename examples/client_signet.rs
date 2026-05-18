// SPDX-License-Identifier: MIT OR Apache-2.0

//! # [`Wallet`] synchronization via the [`Client`] on [`Network::Signet`].
//!
//! ## Architectural Diagram
//!
//! ```text
//!                                                    Programatically trigger a
//!                                                    wallet scan via [`Client::scan`]
//!  Utreexo Bridge                  bdk_floresta                    |
//!  ┌────────────┐                   ┌────────┐   Scan Request  ┌───+────┐                 ┌────────┐
//!  |            |       P2P         |        | <-------------- |        |     Channel     |        |
//!  │ `utreexod` │ <---------------> │  Node  |                 | Client | --------------> | Wallet |
//!  |            |      Blocks       |        | --------------> |        |  Wallet Update  |        |
//!  └────────────┘  Utreexo Proofs   └────────┘     Blocks      └────────┘                 └────────┘
//! ```
//!
//! ## Wallet Scan Workflow
//!
//! 1. Create a [`Wallet`].
//! 2. Create and run a [`Node`].
//! 3. Create a [`Client`] associated with the [`Node`] and [`Wallet`].
//! 4. Trigger a wallet scan via [`Client::scan`].
//! 5. Listen for [`ScanEvent`]s via the [`Client::event_tx`] channel and apply them via [`Wallet::apply_update`].
//!
//! [`Node`]: bdk_floresta::node::Node

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bdk_floresta::client::Client;
use bdk_floresta::client::ScanEvent;
use bdk_floresta::client::ScanKind;
use bdk_floresta::logger::Logger;
use bdk_floresta::node::fsm::State;
use bdk_wallet::Wallet;
use bitcoin::Network;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;
use tracing::warn;
use tracing::Level;

const UTREEXO_BRIDGE: &str = "195.26.240.213:38433";

const NETWORK: Network = Network::Signet;
const DATA_DIR: &str = "./examples/data/client_signet/";

const DESC_EXT: &str = "wpkh([9cee26c8/84h/1h/0h]tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/0/*)";
const DESC_INT: &str = "wpkh([9cee26c8/84h/1h/0h]tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/1/*)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Only display the example's own logs
    env::set_var("RUST_LOG", "client_signet=info");

    // Set up the logger
    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: Some(PathBuf::from(DATA_DIR).join("client_signet.log")),
    }
    .init()?;

    // Configure bdk_floresta
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(DATA_DIR).join("bdk_floresta"),
        fixed_peer: Some(SocketAddr::from_str(UTREEXO_BRIDGE)?),
        ..Default::default()
    };

    // Build and run the node
    let node = Builder { config, logger: None }.build()?;
    node.run().await?;

    // Print node state transitions
    let mut state_rx = node.subscribe_state();
    tokio::spawn(async move {
        loop {
            let state = state_rx.borrow_and_update().clone();
            info!("> NODE STATE: {}", state);
            if state_rx.changed().await.is_err() {
                break;
            }
        }
    });

    // Print node actions
    let mut action_rx = node.subscribe_action();
    tokio::spawn(async move {
        let mut last = String::new();
        loop {
            let actions = action_rx.borrow_and_update().clone();
            let line = actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>().join(", ");
            if line != last {
                info!("> NODE ACTION: [{}]", line);
                last = line;
            }
            if action_rx.changed().await.is_err() {
                break;
            }
        }
    });

    // Wrap the node in an Arc
    let node = Arc::new(node);

    // Create a wallet
    let wallet = Wallet::create(DESC_EXT, DESC_INT)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    // Build a client associated with the node
    let (mut client, mut update_events) = Client::new(node.clone(), &wallet)?;

    // Display pre-scan balance
    info!(
        "> PRE-SCAN BALANCE: {} BTC, {} UTXOs",
        wallet.balance().total().to_btc(),
        wallet.list_unspent().count()
    );

    // Arc the wallet to allow shared access
    let wallet = Arc::new(RwLock::new(wallet));

    // Spawn a task to subscribe to updates and apply them to the wallet
    let scan_wallet = wallet.clone();
    let scan_task = tokio::spawn(async move {
        while let Some(event) = update_events.recv().await {
            let mut wallet = scan_wallet.write().await;
            match event {
                ScanEvent::Update(update) => match wallet.apply_update(update) {
                    Ok(()) => {
                        let balance = wallet.balance().total().to_btc();
                        info!("> CURRENT BALANCE: {balance} BTC");
                    }
                    Err(e) => warn!("> FAILED TO APPLY UPDATE: {e}"),
                },
                ScanEvent::Finished => {
                    info!("> SCAN COMPLETE");
                    break;
                }
                ScanEvent::Error(e) => {
                    warn!("> SCAN ERROR: {e}");
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("> RECEIVED `SIGINT`");
            scan_task.abort();
            node.shutdown().await?;
            info!("> /exit");

            return Ok(());
        }
        result = async {
            // Subscribe to the node's state changes
            let mut state_rx = node.subscribe_state();
            // Wait for the node to reach operational state before requesting a scan
            while *state_rx.borrow_and_update() != State::Operational {
                if state_rx.changed().await.is_err() {
                    return Ok::<(), anyhow::Error>(());
                }
            }

            // Perform a full scan (genesis to tip)
            let scan_params = ScanKind::FullScan { custom_lookahead: None };
            info!("> STARTING FULLSCAN FROM CHECKPOINT=[height={} hash={}]", client.checkpoint().height(), client.checkpoint().hash());
            client.scan(scan_params).await?;
            info!("> FINISHED FULLSCAN AT CHECKPOINT=[height={} hash={}]", client.checkpoint().height(), client.checkpoint().hash());

            // Perform a sync from where the full scan left off (up to tip)
            let scan_params = ScanKind::Sync;
            info!("> STARTING SYNC FROM CHECKPOINT=[height={} hash={}]", client.checkpoint().height(), client.checkpoint().hash());
            client.scan(scan_params).await?;
            info!("> FINISHED SYNC AT CHECKPOINT=[height={} hash={}]", client.checkpoint().height(), client.checkpoint().hash());

            Ok::<(), anyhow::Error>(())
        } => {
            if let Err(e) = result {
                error!("> SCAN ERROR: {e}");
            }
        }
    }

    // Wait for the scan task to finish
    scan_task.await?;

    // Display the post-scan balance
    let wallet = Arc::try_unwrap(wallet).expect("ref count is non-zero").into_inner();
    info!(
        "> POST-SCAN BALANCE: {} BTC, {} UTXOs",
        wallet.balance().total().to_btc(),
        wallet.list_unspent().count()
    );

    // Shut the node down
    node.shutdown().await?;

    Ok(())
}
