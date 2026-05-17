// SPDX-License-Identifier: MIT OR Apache-2.0

//! # [`Wallet`] synchronization via the [`Client`] on [`Network::Regtest`].
//!
//! ## Architectural Diagram
//!
//! ```text
//!                                                  Programatically trigger a
//!                                                  wallet scan via [`Client::scan`]
//! Utreexo Bridge                bdk_floresta                     |
//!  ┌──────────┐                   ┌────────┐   Scan Request  ┌───+────┐                 ┌────────┐
//!  |          |       P2P         |        | <-------------- |        |     Channel     |        |
//!  │ UtreexoD │ <---------------> │  Node  |                 | Client | --------------> | Wallet |
//!  |          |      Blocks       |        | --------------> |        |  Wallet Update  |        |
//!  └──────────┘  Utreexo Proofs   └────────┘     Blocks      └────────┘                 └────────┘
//! ```
//!
//! ## Wallet Scan Workflow
//!
//! 1. Create a [`Wallet].
//! 2. Create and run a [`Node`].
//! 3. Create a [`Client`] associated with the [`Node`] and [`Wallet`].
//! 4. Trigger a wallet scan via [`Client::scan`].
//! 5. Listen for [`ScanEvent`]s via the [`Client::event_tx`] channel and apply them via [`Wallet::apply_update`].
//!
//! [`Node`]: bdk_floresta::node::Node

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use bdk_floresta::builder::Builder;
use bdk_floresta::builder::NodeConfig;
use bdk_floresta::client::Client;
use bdk_floresta::client::ScanEvent;
use bdk_floresta::client::ScanKind;
use bdk_floresta::logger::Logger;
use bdk_floresta::node::fsm::State;
use bdk_wallet::Wallet;
use bitcoin::Amount;
use bitcoin::Network;
use halfin::bitcoind::BitcoinD;
use halfin::bitcoind::BitcoinDConf;
use halfin::connect;
use halfin::utreexod::UtreexoD;
use halfin::utreexod::UtreexoDConf;
use halfin::wait_for_height;
use halfin::Node;
use tokio::sync::RwLock;
use tracing::error;
use tracing::info;
use tracing::warn;
use tracing::Level;

const BLOCK_COUNT: u32 = 200;
const EXPECTED_BALANCE: Amount = Amount::from_int_btc(552);

const NETWORK: Network = Network::Regtest;
const DATA_DIR: &str = "./examples/data/client_regtest/";

const DESC_EXT: &str = "tr([697412fb/86h/1h/0h]tpubDD5LyUwjWvkndncfQqMcNko6coES1ZehnsqykLLe8E3w7RX5dveHMJrXxhpvYLnSqQBuPVw9Lk7gSYnS9xyoLRQ21xpebdpCFt5ZNyorhHb/0/*)";
const DESC_INT: &str = "tr([697412fb/86h/1h/0h]tpubDD5LyUwjWvkndncfQqMcNko6coES1ZehnsqykLLe8E3w7RX5dveHMJrXxhpvYLnSqQBuPVw9Lk7gSYnS9xyoLRQ21xpebdpCFt5ZNyorhHb/1/*)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Only display the example's own logs
    // `warn` is there so the subscriber is
    // not buried by hyper/bitreq/jsonrcp logs
    env::set_var("RUST_LOG", "client_regtest=info,warn");

    // Set up the logger
    let _logger = Logger {
        log_level: Level::INFO,
        log_to_stdout: true,
        log_file: Some(PathBuf::from(DATA_DIR).join("client_regtest.log")),
    }
    .init()?;

    // Create a wallet
    let mut wallet = Wallet::create(DESC_EXT, DESC_INT)
        .network(NETWORK)
        .create_wallet_no_persist()?;

    // Get external and internal addresses from the wallet
    let address_ext_0 = wallet.reveal_next_address(bdk_wallet::KeychainKind::External).address;
    let address_ext_1 = wallet.reveal_next_address(bdk_wallet::KeychainKind::External).address;
    let address_int_0 = wallet.reveal_next_address(bdk_wallet::KeychainKind::Internal).address;
    let address_int_1 = wallet.reveal_next_address(bdk_wallet::KeychainKind::Internal).address;

    // Configure the bitcoind node
    let bitcoind_conf = BitcoinDConf {
        staticdir: Some(PathBuf::from(DATA_DIR).join("bitcoind")),
        ..Default::default()
    };
    // Spawn a bitcoind node
    let bitcoind = BitcoinD::new_with_conf(&bitcoind_conf)?;
    info!("> BITCOIND: SPAWNED");

    // Configure the bridge node
    let utreexod_conf = UtreexoDConf {
        args: vec!["--regtest", "--notls", "--nodnsseed", "--noassumeutreexo", "--cfilters"],
        staticdir: Some(PathBuf::from(DATA_DIR).join("utreexod")),
        ..Default::default()
    };
    // Spawn the bridge node
    let utreexod = UtreexoD::new_with_conf(&utreexod_conf)?;
    info!("> UTREEXOD: SPAWNED");

    info!("> BITCOIND: MINING {BLOCK_COUNT} BLOCKS");
    bitcoind.generate(BLOCK_COUNT)?;

    info!("> BITCOIND: SENDING 21 BTC TO ADDRESS={} [EXTERNAL]", address_ext_0);
    bitcoind.call("sendtoaddress", &[address_ext_0.to_string().into(), 21.into()])?;

    info!("> BITCOIND: MINING 1 BLOCK");
    bitcoind.generate(1)?;

    info!("> BITCOIND: SENDING 42 BTC TO ADDRESS={} [EXTERNAL]", address_ext_1);
    bitcoind.call("sendtoaddress", &[address_ext_1.to_string().into(), 42.into()])?;

    info!("> BITCOIND: MINING 1 BLOCK");
    bitcoind.generate(1)?;

    info!("> BITCOIND: SENDING 69 BTC TO ADDRESS={} [INTERNAL]", address_int_0);
    bitcoind.call("sendtoaddress", &[address_int_0.to_string().into(), 69.into()])?;

    info!("> BITCOIND: MINING 1 BLOCK");
    bitcoind.generate(1)?;

    info!("> BITCOIND: SENDING 420 BTC TO ADDRESS={} [INTERNAL]", address_int_1);
    bitcoind.call("sendtoaddress", &[address_int_1.to_string().into(), 420.into()])?;

    info!("> BITCOIND: MINING 1 BLOCK");
    bitcoind.generate(1)?;

    info!("> CONNECTING BITCOIND TO UTREEXOD");
    connect(&bitcoind, &utreexod)?;

    info!("> WAITING FOR UTREEXOD TO SYNC UP WITH BITCOIND");
    wait_for_height(&utreexod, bitcoind.get_chain_tip()?)?;

    // Configure bdk_floresta
    let config = NodeConfig {
        network: NETWORK,
        data_directory: PathBuf::from(DATA_DIR).join("bdk_floresta"),
        fixed_peer: Some(utreexod.get_p2p_socket()),
        ..Default::default()
    };
    // Build and run the node
    let node = Builder { config, logger: None }.build()?;
    node.run().await?;
    info!("> BDK_FLORESTA: SPAWNED");

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
    let post_scan_balance = wallet.balance().total();
    info!(
        "> POST-SCAN BALANCE: {} BTC, {} UTXOs",
        post_scan_balance.to_btc(),
        wallet.list_unspent().count()
    );
    assert_eq!(post_scan_balance, EXPECTED_BALANCE);

    // Shut the node down
    node.shutdown().await?;

    Ok(())
}
