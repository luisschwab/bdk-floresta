// SPDX-License-Identifier: MIT OR Apache-2.0

//! # [`Wallet`] synchronization via the [`Client`] on [`Network::Signet`].
//!
//! ## Architectural Diagram
//!
//! ```text
//!                                                    Programatically trigger a continuous
//!                                                    wallet scan via [`Client::scan_continuous`]
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
//! 4. Trigger a wallet scan via [`Client::scan_continuous`].
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
use bdk_floresta::client::ScanHandle;
use bdk_floresta::client::ScanKind;
use bdk_floresta::logger::Logger;
use bdk_wallet::Wallet;
use bdk_wallet::WalletEvent;
use bitcoin::Network;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;
use tracing::warn;
use tracing::Level;

const UTREEXO_BRIDGE: &str = "utreexod.signet.lab.vinteum.org";

const NETWORK: Network = Network::Signet;
const DATA_DIR: &str = "./examples/data/client_signet/";

const DESC_EXT: &str = "wpkh([9cee26c8/84h/1h/0h]tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/0/*)";
const DESC_INT: &str = "wpkh([9cee26c8/84h/1h/0h]tpubDDuCfGKBYo4pQjNcpVkdLktdYm9wZiowEXMKM4Nn9QBcbnu5ikxmqZyXuhDgcdfr8zcuR66iLCmManN9XguSpP2m2SZyUsJsdCKQkcru6VG/1/*)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env::set_var("RUST_LOG", "client_signet=info");

    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: Some(PathBuf::from(DATA_DIR).join("client_signet.log")),
    }
    .init()?;

    let config = NodeConfig {
        network: NETWORK,
        datadir: PathBuf::from(DATA_DIR).join("bdk_floresta"),
        fixed_peers: Some(vec![SocketAddr::from_str(UTREEXO_BRIDGE)?]),
        ..Default::default()
    };

    let node = Builder { config, logger: None }.build()?;
    node.run().await?;

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

    let wallet = Wallet::create(DESC_EXT, DESC_INT)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    let node = Arc::new(node);
    let (mut client, mut update_events) = Client::new(node.clone(), &wallet)?;

    info!(
        "> PRE-SCAN BALANCE: {} BTC, {} UTXOs",
        wallet.balance().total().to_btc(),
        wallet.list_unspent().count()
    );

    let wallet = Arc::new(RwLock::new(wallet));
    let scan_wallet = wallet.clone();
    let scan_task = tokio::spawn(async move {
        while let Some(event) = update_events.recv().await {
            let mut wallet = scan_wallet.write().await;
            match event {
                ScanEvent::WalletUpdate(update) => match wallet.apply_update_events(update) {
                    Ok(events) => {
                        for event in events {
                            match event {
                                WalletEvent::ChainTipChanged { new_tip, .. } => {
                                    info!(
                                        "> WALLET: NEW CHECKPOINT: height={} hash={}",
                                        new_tip.height, new_tip.hash
                                    );
                                }
                                WalletEvent::TxConfirmed { txid, block_time, .. } => {
                                    info!(
                                        "> WALLET: TX CONFIRMED: txid={txid} height={} | BALANCE: {} BTC",
                                        block_time.block_id.height,
                                        wallet.balance().total().to_btc()
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => warn!("> FAILED TO APPLY UPDATE: {e}"),
                },
                ScanEvent::Finished { .. } => info!("> SCANNED UP TO TIP, CONTINUING..."),
                ScanEvent::Error(e) => {
                    warn!("> SCAN ERROR: {e}");
                    break;
                }
            }
        }
    });

    let scan_params = ScanKind::FullScan { custom_lookahead: None };
    info!(
        "> STARTING CONTINUOUS FULLSCAN FROM CHECKPOINT=[height={} hash={}]",
        client.checkpoint().height(),
        client.checkpoint().hash()
    );
    let scan_token = client.cancellation_token();
    let mut scan_handle = ScanHandle::new(
        scan_token,
        tokio::spawn(async move { client.scan_continuous(scan_params).await }),
    );

    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("> RECEIVED `SIGINT`"),
        res = scan_handle.join() => {
            if let Err(e) = res {
                error!("> SCAN ERROR: {e}");
            }
        }
    }

    scan_handle.stop().await;
    scan_task.await?;

    let wallet = Arc::try_unwrap(wallet).expect("ref count is non-zero").into_inner();
    info!(
        "> POST-SCAN BALANCE: {} BTC, {} UTXOs",
        wallet.balance().total().to_btc(),
        wallet.list_unspent().count()
    );

    node.shutdown().await?;

    Ok(())
}
