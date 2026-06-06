// SPDX-License-Identifier: MIT OR Apache-2.0

//! # [`Wallet`] synchronization via the [`Client`] on [`Network::Regtest`].
//!
//! ## Architectural Diagram
//!
//! ```text
//!                                                  Programatically trigger a continuous
//!                                                  wallet scan via [`Client::scan_continuous`]
//!  Utreexo Bridge                  bdk_floresta                    |
//!  ┌────────────┐                   ┌────────┐   Scan Request  ┌───+────┐                 ┌────────┐
//!  |            |       P2P         |        | <-------------- |        |     Channel     |        |
//!  │  UtreexoD  │ <---------------> │  Node  |                 | Client | --------------> | Wallet |
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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bdk_floresta::client::Client;
use bdk_floresta::client::ScanEvent;
use bdk_floresta::client::ScanHandle;
use bdk_floresta::client::ScanKind;
use bdk_floresta::fsm::State;
use bdk_floresta::logger::Logger;
use bdk_wallet::Wallet;
use bdk_wallet::WalletEvent;
use bitcoin::Network;
use halfin::bitcoind::BitcoinD;
use halfin::bitcoind::BitcoinDConf;
use halfin::connect;
use halfin::utreexod::UtreexoD;
use halfin::utreexod::UtreexoDConf;
use halfin::wait_for_filter_height;
use halfin::wait_for_height;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;
use tracing::warn;
use tracing::Level;

const NETWORK: Network = Network::Regtest;
const DATA_DIR: &str = "./examples/data/client_regtest/";

const DESC_EXT: &str = "tr([697412fb/86h/1h/0h]tpubDD5LyUwjWvkndncfQqMcNko6coES1ZehnsqykLLe8E3w7RX5dveHMJrXxhpvYLnSqQBuPVw9Lk7gSYnS9xyoLRQ21xpebdpCFt5ZNyorhHb/0/*)";
const DESC_INT: &str = "tr([697412fb/86h/1h/0h]tpubDD5LyUwjWvkndncfQqMcNko6coES1ZehnsqykLLe8E3w7RX5dveHMJrXxhpvYLnSqQBuPVw9Lk7gSYnS9xyoLRQ21xpebdpCFt5ZNyorhHb/1/*)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env::set_var("RUST_LOG", "client_regtest=info");

    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: Some(PathBuf::from(DATA_DIR).join("client_regtest.log")),
    }
    .init()?;

    let mut wallet = Wallet::create(DESC_EXT, DESC_INT)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    let address_ext = wallet.reveal_next_address(bdk_wallet::KeychainKind::External).address;
    let address_int = wallet.reveal_next_address(bdk_wallet::KeychainKind::Internal).address;

    let bitcoind_conf = BitcoinDConf {
        staticdir: Some(PathBuf::from(DATA_DIR).join("bitcoind")),
        ..Default::default()
    };
    let bitcoind = BitcoinD::new_with_conf(&bitcoind_conf)?;
    info!("> BITCOIND: SPAWNED");

    let utreexod_conf = UtreexoDConf {
        args: vec!["--regtest", "--notls", "--nodnsseed", "--noassumeutreexo", "--cfilters"],
        staticdir: Some(PathBuf::from(DATA_DIR).join("utreexod")),
        ..Default::default()
    };
    let utreexod = UtreexoD::new_with_conf(&utreexod_conf)?;
    info!("> UTREEXOD: SPAWNED");

    bitcoind.generate(5)?;

    info!("> CONNECTING BITCOIND TO UTREEXOD");
    connect(&bitcoind, &utreexod)?;

    wait_for_height(&utreexod, bitcoind.get_chain_tip()?)?;
    wait_for_filter_height(&utreexod, bitcoind.get_filter_tip()?)?;

    let config = NodeConfig {
        network: NETWORK,
        datadir: PathBuf::from(DATA_DIR).join("bdk_floresta"),
        fixed_peers: Some(vec![utreexod.get_p2p_socket()]),
        ..Default::default()
    };
    let node = Builder { config, logger: None }.build()?;
    node.run().await?;
    info!("> BDK_FLORESTA: SPAWNED");

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

    info!(
        "> STARTING CONTINUOUS FULLSCAN FROM CHECKPOINT=[height={} hash={}]",
        client.checkpoint().height(),
        client.checkpoint().hash()
    );

    let scan_token = client.cancellation_token();
    let scan_params = ScanKind::FullScan { custom_lookahead: None };
    let mut scan_handle = ScanHandle::new(
        scan_token,
        tokio::spawn(async move { client.scan_continuous(scan_params).await }),
    );

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("> RECEIVED `SIGINT`");

            scan_handle.stop().await;
            scan_task.await?;

            let wallet = Arc::try_unwrap(wallet).unwrap().into_inner();
            info!(
                "> POST-SCAN BALANCE: {} BTC, {} UTXOs",
                wallet.balance().total().to_btc(),
                wallet.list_unspent().count()
            );

            node.shutdown().await?;

            info!("> /exit");
            return Ok(());
        }
        result = async {
            let mut state_rx = node.subscribe_state();
            while *state_rx.borrow_and_update() != State::Operational {
                state_rx.changed().await?;
            }

            let blocks: u32 = env::var("BLOCKS").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
            for i in 0..blocks {
                if i % 5 == 0 {
                    info!("> BITCOIND: MINING 1 BLOCK TO EXTERNAL ADDRESS={}", address_ext);
                    bitcoind.generate_to_address(1, &address_ext).unwrap();
                } else if i % 3 == 0 {
                    info!("> BITCOIND: MINING 1 BLOCK TO INTERNAL ADDRESS={}", address_int);
                    bitcoind.generate_to_address(1, &address_int).unwrap();
                } else {
                    info!("> BITCOIND: MINING 1 BLOCK");
                    bitcoind.generate(1).unwrap();
                }

                let target = bitcoind.get_chain_tip()?;
                wait_for_height(&utreexod, target)?;

                let timeout = Duration::from_secs(10);
                let poll = Duration::from_millis(200);
                let mut waited = Duration::ZERO;
                while node.get_block_hash(target).is_err() {
                    if waited >= timeout {
                        warn!("> BDK_FLORESTA DID NOT REACH height={target} within {timeout:?}");
                        break;
                    }
                    tokio::time::sleep(poll).await;
                    waited += poll;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            Ok::<(), anyhow::Error>(())
        } => {
            if let Err(e) = result {
                error!("> SCAN ERROR: {e}");
            }
            scan_handle.stop().await;
        }
    }
    Ok(())
}
