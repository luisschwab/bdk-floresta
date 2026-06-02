// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Client
//!
//! This module implements a [`Client`], used to request
//! and subscribe to [`Update`]s from a [`Node`].

use core::mem;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use bdk_wallet::chain::keychain_txout::KeychainTxOutIndex;
use bdk_wallet::chain::BlockId;
use bdk_wallet::chain::CheckPoint;
use bdk_wallet::chain::ConfirmationBlockTime;
use bdk_wallet::chain::IndexedTxGraph;
use bdk_wallet::chain::Merge;
use bdk_wallet::chain::TxUpdate;
use bdk_wallet::KeychainKind;
use bdk_wallet::Update;
use bdk_wallet::Wallet;
use bitcoin::Block;
use bitcoin::OutPoint;
use bitcoin::ScriptBuf;
use floresta_chain::BlockConsumer;
use floresta_chain::UtxoData;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::client::error::ClientError;
use crate::node::fsm::State;
use crate::node::Node;

pub mod error;

/// The lookahead value to derive spks during [`ScanKind::FullScan`].
pub const FULL_SCAN_LOOKAHEAD: u32 = 1_000;

/// The kind of scan to perform on the [`Wallet`].
///
/// The chosen variant will define how
/// Compact Block Filter scanning will be performed.
#[derive(Debug)]
pub enum ScanKind {
    /// Scan from the [`Wallet`]'s latest checkpoint up to the chain tip.
    Sync,

    /// Scan from genesis up to the chain tip, using a custom `lookahead` value.
    ///
    /// Full-scans should use a much higher lookahead value than
    /// the [`Wallet`]'s default `lookahead`. If `custom_lookahead` is `None`,
    /// it's set to [`FULL_SCAN_LOOKAHEAD`].
    FullScan { custom_lookahead: Option<u32> },

    /// Scan from a custom start height up to the chain tip, using a custom `lookahead` value.
    Custom { start_height: u32, custom_lookahead: u32 },
}

/// An event emitted from a scan.
#[derive(Clone, Debug)]
pub enum ScanEvent {
    /// An [`Update`] for the [`Wallet`].
    WalletUpdate(Update),

    /// The scan has finished.
    ///
    /// For continuous synching, `keep_going` is set to `true`.
    Finished { keep_going: bool },

    /// An error happened whilst scanning.
    Error(String),
}

/// A handle to a [`Client::scan`] or [`Client::scan_continuous`] task.
///
/// Call [`Client::cancellation_token`] to get the [`CancellationToken`]
/// and create a [`ScanHandle`] with itbefore spawning the scan in an async task.
///
/// An ongoing scan can then be cancelled via [`ScanHandle::stop`].
pub struct ScanHandle {
    token: CancellationToken,
    inner: Option<JoinHandle<Result<(), ClientError>>>,
}

impl ScanHandle {
    /// Create a new [`ScanHandle`] from a [`CancellationToken`] and a join handle.
    pub fn new(token: CancellationToken, inner: JoinHandle<Result<(), ClientError>>) -> Self {
        Self {
            token,
            inner: Some(inner),
        }
    }

    /// Await a scan task to complete.
    ///
    /// Returns [`ClientError::ScanAborted`] if the task was aborted or panicked.
    pub async fn join(&mut self) -> Result<(), ClientError> {
        let Some(inner) = self.inner.as_mut() else {
            return Ok(());
        };
        let result = inner.await.unwrap_or(Err(ClientError::ScanAborted));
        self.inner = None;
        result
    }

    /// Cancel the scan task and immediately return.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Cancel the scan task and wait until it exits.
    pub async fn stop(&mut self) {
        self.token.cancel();
        if let Some(inner) = self.inner.take() {
            let _ = inner.await;
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.token.cancel();
        if let Some(inner) = self.inner.take() {
            inner.abort();
        }
    }
}

/// Forwards validated [`Block`]s from the [`Node`]
/// into a channel read by [`Client::scan_continuous`].
struct BlockSink {
    block_tx: UnboundedSender<(Block, u32)>,
}

impl BlockConsumer for BlockSink {
    fn on_block(&self, block: &Block, height: u32, _spent_utxos: Option<&HashMap<OutPoint, UtxoData>>) {
        let _ = self.block_tx.send((block.clone(), height));
    }

    fn wants_spent_utxos(&self) -> bool {
        false
    }
}

/// A [`Client`] used to request [`Update`]s from a given [`Node`].
pub struct Client {
    /// The associated [`Node`] which this [`Client`] will submit scan requests to.
    node: Arc<Node>,

    /// A subscriber for new [`Block`]s that the [`Node`] validates whist a scan is in progress.
    block_subscriber: Arc<AsyncMutex<UnboundedReceiver<(Block, u32)>>>,

    /// The [`Client`]'s latest [`CheckPoint`].
    checkpoint: CheckPoint,

    /// The [`Client`]'s [transaction graph](IndexedTxGraph).
    tx_graph: IndexedTxGraph<ConfirmationBlockTime, KeychainTxOutIndex<KeychainKind>>,

    /// The channel used to transmit [`ScanEvent`]s.
    event_tx: Arc<Mutex<Option<UnboundedSender<ScanEvent>>>>,

    /// The [`CancellationToken`] used to cancel ongoing scans.
    ///
    /// Used when the [`Client`] is [`Drop`]ped, usually
    /// due to the associated [`Node`] shutting down.
    cancellation_token: CancellationToken,

    /// A handle to a task that watches the [`Node`]'s cancellation token.
    ///
    /// When the [`Node`] begins shutdown, this task cancels the
    /// [`Client`]'s own cancellation token, aborts any in-flight
    /// scans, and drops the event channel sender.
    shutdown_watcher: JoinHandle<()>,
}

impl Client {
    /// Create a new [`Client`] for requesting [`Wallet`] [`Update`]s from the given [`Node`].
    pub fn new(node: Arc<Node>, wallet: &Wallet) -> Result<(Self, UnboundedReceiver<ScanEvent>), ClientError> {
        // The node and wallet must be on the same network
        if wallet.network() != node.config.network {
            return Err(ClientError::NetworkMismatch {
                node_net: node.config.network,
                wallet_net: wallet.network(),
            });
        }

        // Create a channel used to broadcast scan events to subscribers of the client
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let event_tx = Arc::new(Mutex::new(Some(event_tx)));

        // Get the node's cancellation token
        // The client uses it to shut itself down when the associated node shuts down
        let node_cancellation_token = node.cancellation_token();

        // Create a cancellation token for the client
        let cancellation_token = CancellationToken::new();
        let client_cancellation_token = cancellation_token.clone();

        // Spawn a task that terminates the client when the node shuts down
        let shutdown_watcher_tx = event_tx.clone();
        let shutdown_watcher = tokio::spawn(async move {
            node_cancellation_token.cancelled().await;
            client_cancellation_token.cancel();
            if let Ok(mut guard) = shutdown_watcher_tx.lock() {
                guard.take();
            }
        });

        // Clone the wallet's `IndexedTxGraph`
        let tx_graph = IndexedTxGraph::new(wallet.spk_index().clone());
        // Use the wallet's last checkpoint as the client's latest checkpoint
        // Used to set the starting height for client scan requests
        let checkpoint = wallet.latest_checkpoint();

        // Create a channel for receiving new blocks;
        let (block_tx, block_rx) = mpsc::unbounded_channel::<(Block, u32)>();
        let block_subscriber = Arc::new(AsyncMutex::new(block_rx));
        node.block_subscriber(Arc::new(BlockSink { block_tx }));

        let client = Self {
            node,
            block_subscriber,
            checkpoint,
            tx_graph,
            event_tx,
            cancellation_token,
            shutdown_watcher,
        };

        Ok((client, event_rx))
    }

    /// Get this [`Client`]'s latest [`CheckPoint`].
    pub fn checkpoint(&self) -> CheckPoint {
        self.checkpoint.clone()
    }

    /// Returns a clone of this [`Client`]'s [`CancellationToken`].
    ///
    /// Cancelling the token aborts any in-progress [`scan`](Self::scan)
    /// or [`scan_continuous`](Self::scan_continuous).
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    /// Request the [`Node`] to perform a scan using the [`Wallet`] associated with this [`Client`].
    ///
    /// The [`kind`](ScanKind) parameter dictates which kind of scan will be performed:
    ///  - [`ScanKind::Sync`]: scan from the [`Wallet`]'s latest checkpoint up to the chain tip
    ///  - [`ScanKind::FullScan`]: scan from genesis up to the chain tip
    ///  - [`ScanKind::Custom`]: scan from a custom start height up to the chain tip
    pub async fn scan(&mut self, scan_params: ScanKind) -> Result<(), ClientError> {
        // Wait for the node to reach operational
        // state before submitting a scan request
        self.wait_until_ready().await?;

        // Submit a scan request to the node and wait for the result
        let scan_result = self.scan_inner(scan_params).await;

        // Broadcast a notification if the scan is done or an error occurred
        // Actual wallet updates are sent over the channel by `scan_inner`
        let event = match &scan_result {
            Ok(()) => ScanEvent::Finished { keep_going: false },
            Err(e) => ScanEvent::Error(e.to_string()),
        };

        // Send the final scan event over the channel
        self.send_event(event)?;

        scan_result
    }

    /// Request the [`Node`] to perform a continuous scan using the [`Wallet`] associated with this [`Client`].
    ///
    /// This scan happens in two stages:
    /// 1. Scan from either genesis or the [`Wallet`]'s latest checkpoint up to the chain tip with Compact Block Filters
    /// 2. Subscribe to new blocks validated by the `Node` and apply them to the [`Wallet`]
    ///
    /// The [`kind`](ScanKind) parameter dictates which kind of scan will be performed:
    ///  - [`ScanKind::Sync`]: scan from the [`Wallet`]'s latest checkpoint up to the chain tip
    ///  - [`ScanKind::FullScan`]: scan from genesis up to the chain tip
    ///  - [`ScanKind::Custom`]: scan from a custom start height up to the chain tip
    pub async fn scan_continuous(&mut self, scan_params: ScanKind) -> Result<(), ClientError> {
        // Wait for the node to reach operational
        // state before submitting a scan request
        self.wait_until_ready().await?;

        // Run the initial CBF scan; new blocks arriving via `on_block`
        // queue up in `block_subscriber` while this runs
        let scan_result = self.scan_inner(scan_params).await;

        // Broadcast a notification if the CBF scan is done or an error occurred
        // Actual wallet updates are sent over the channel by `scan_inner`
        let event = match &scan_result {
            Ok(()) => ScanEvent::Finished { keep_going: true },
            Err(e) => ScanEvent::Error(e.to_string()),
        };
        self.send_event(event)?;
        scan_result?;

        // Subscribe to new blocks validated by the `Node`
        let block_subscriber = self.block_subscriber.clone();
        loop {
            let (block, height) = {
                let mut guard = block_subscriber.lock().await;
                tokio::select! {
                    biased;
                    _ = self.cancellation_token.cancelled() => {
                        return Err(ClientError::ScanAborted);
                    }
                    b = guard.recv() => match b {
                        Some(b) => b,
                        None => return Err(ClientError::UnresponsiveNode),
                    },
                }
            };

            // Skip blocks that are already known to the `Wallet`
            if height <= self.checkpoint.height() {
                continue;
            }

            // Apply the block to the `tx_graph` and update the `checkpoint`
            let hash = block.block_hash();
            let _ = self.tx_graph.apply_block_relevant(&block, height);
            self.checkpoint = self.checkpoint.clone().insert(BlockId { height, hash });
            let update = self.finish_update();

            self.send_event(ScanEvent::WalletUpdate(update))?;
        }
    }

    /// Send a [`ScanEvent`] over the channel.
    fn send_event(&self, event: ScanEvent) -> Result<(), ClientError> {
        let guard = self.event_tx.lock().map_err(|_| ClientError::PoisonedLock)?;
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(event);
        }
        Ok(())
    }

    /// Wait for the [`Node`] to reach [`State::Operational`] before
    /// submitting any requests to it.
    async fn wait_until_ready(&self) -> Result<(), ClientError> {
        let mut rx = self.node.subscribe_state();
        loop {
            match rx.borrow().clone() {
                State::Operational => return Ok(()),
                State::ShuttingDown => return Err(ClientError::UnresponsiveNode),
                State::Inactive => return Err(ClientError::UnresponsiveNode),
                _ => {}
            }
            tokio::select! {
                _ = self.cancellation_token.cancelled() => return Err(ClientError::ScanAborted),
                changed = rx.changed() => {
                    if changed.is_err() {
                        return Err(ClientError::UnresponsiveNode);
                    }
                }
            }
        }
    }

    /// Build an [`Update`] from the [`Client`]'s [`IndexedTxGraph`].
    ///
    /// This [`Update`] can be directly applied to the [`Wallet`] via [`Wallet::apply_update`].
    fn finish_update(&mut self) -> Update {
        // Construct a `TxUpdate` from this client's `TxGraph`
        let tx_update = TxUpdate::from(self.tx_graph.graph().clone());

        // Fetch this client's current last active keychain indices
        let last_active_indices = self.tx_graph.index.last_used_indices();

        // Fetch this client's current `KeychainTxOutIndex`
        let tx_graph_index = mem::take(&mut self.tx_graph.index);

        // Create a new `IndexedTxGraph` using the current `KeychainTxOutIndex`
        self.tx_graph = IndexedTxGraph::new(tx_graph_index);

        Update {
            tx_update,
            last_active_indices,
            chain: Some(self.checkpoint.clone()),
        }
    }

    /// Derive [script pubkeys](ScriptBuf) from the [`IndexedTxGraph`]'s default `lookahead` value.
    fn derive_spks(&self) -> Result<Vec<ScriptBuf>, ClientError> {
        self.derive_spks_with_custom_lookahead(self.tx_graph.index.lookahead())
    }

    /// Derive [script pubkeys](ScriptBuf) from the [`IndexedTxGraph`] with a custom `lookahead` value.
    fn derive_spks_with_custom_lookahead(&self, custom_lookahead: u32) -> Result<Vec<ScriptBuf>, ClientError> {
        // The wallet's keychain index
        let keychain_index = &self.tx_graph.index;
        // The wallet's last revealed keychain indices
        let last_revealed_indices = keychain_index.last_revealed_indices();

        let mut spks = Vec::new();

        // Derive `external_index` + `custom_lookahead` spks from the external keychain
        let external_index = last_revealed_indices.get(&KeychainKind::External).copied().unwrap_or(0);
        let external_key_iter = keychain_index
            .unbounded_spk_iter(KeychainKind::External)
            .ok_or(ClientError::NoExternalKeychain)?;
        let bound = external_index.saturating_add(custom_lookahead) as usize;
        // Add the external spks to the spk store
        spks.extend(external_key_iter.take(bound).map(|(_, s)| s));

        // Derive `internal_index` + `custom_lookahead` spks from the internal keychain
        let internal_index = last_revealed_indices.get(&KeychainKind::Internal).copied().unwrap_or(0);
        // The internal keychain is optional
        if let Some(internal_key_iter) = keychain_index.unbounded_spk_iter(KeychainKind::Internal) {
            let bound = internal_index.saturating_add(custom_lookahead) as usize;
            // Add the internal spks to the spk store
            spks.extend(internal_key_iter.take(bound).map(|(_, s)| s));
        }

        Ok(spks)
    }

    /// Inner implementation of [`Self::scan`].
    ///
    /// Performs filter matching, [`Block`] fetching,
    /// and update application against the provided [`ScanKind`].
    ///
    /// Separated from [`Self::scan`] to keep the post-scan
    /// event broadcasting logic at the outer level.
    async fn scan_inner(&mut self, kind: ScanKind) -> Result<(), ClientError> {
        // Derive a child cancellation token from the client's
        // cancellation token, used to abort in-progress scans
        let scan_token = self.cancellation_token.child_token();

        // Derive the `start_height` and `spks` from the `ScanKind`
        let (start_height, spks) = match kind {
            // Start the scan from the latest checkpoint
            // and derive spks with the default lookahead value
            ScanKind::Sync => (self.checkpoint.height(), self.derive_spks()?),
            // Start the scan from genesis and derive spks
            // with a custom lookahead, or fallback to `FULL_SCAN_LOOKAHEAD`
            ScanKind::FullScan { custom_lookahead } => (
                0,
                self.derive_spks_with_custom_lookahead(custom_lookahead.unwrap_or(FULL_SCAN_LOOKAHEAD))?,
            ),
            // Start the scan from a custom height and
            // derive spks with a custom lookahead value
            ScanKind::Custom {
                start_height,
                custom_lookahead,
            } => (start_height, self.derive_spks_with_custom_lookahead(custom_lookahead)?),
        };

        // Use the node's filter height as the end height
        let stop_height = self.node.get_filter_height().map_err(ClientError::Node)?;

        // Throw an error if the scan range is negative
        if start_height > stop_height {
            return Err(ClientError::InvalidRange {
                start_height,
                stop_height,
            });
        }

        // Request the node to provide hashes of blocks
        // with relevant transactions containing `spks`
        let hashes = tokio::select! {
            // Early return if the scan was aborted
            biased;
            _ = scan_token.cancelled() => return Err(ClientError::ScanAborted),
            // Request hashes from the node
            result = self.node.match_filters(spks, start_height, stop_height) => {
                result.map_err(ClientError::Node)?
            }
        };

        // Request the node to fetch blocks from its peers
        let blocks = if hashes.is_empty() {
            Vec::new()
        } else {
            tokio::select! {
                // Early return if the scan was aborted
                biased;
                _ = scan_token.cancelled() => return Err(ClientError::ScanAborted),
                // Request blocks from the node in parallel
                result = self.node.fetch_blocks(&hashes) => {
                    result.map_err(ClientError::Node)?
                }
            }
        };

        for block in blocks {
            // Early return if the scan was aborted
            if scan_token.is_cancelled() {
                return Err(ClientError::ScanAborted);
            }

            // Request the node to get the height associated with a block hash
            let hash = block.block_hash();
            let height = self.node.get_block_height(&hash).map_err(ClientError::Node)?;

            // Construct a change set by applying the block to the transaction graph
            let changeset = self.tx_graph.apply_block_relevant(&block, height);

            // An empty change set means the block was a false positive
            // Skip to the next block
            if changeset.is_empty() {
                continue;
            }

            // Update the latest checkpoint to this block
            self.checkpoint = self.checkpoint.clone().insert(BlockId { height, hash });

            // Finalize the update
            let update = self.finish_update();

            // Send the update to subscribers over the channel
            self.send_event(ScanEvent::WalletUpdate(update))?;
        }

        // A checkpoint below the stop height means there are no
        // relevant blocks between [checkpoint, stop_height]
        // Add a final checkpoint at the stop height
        // such that subsequent scans start from this height
        if self.checkpoint.height() < stop_height {
            // Request the node to get the hash associated with a height
            let stop_hash = self.node.get_block_hash(stop_height).map_err(ClientError::Node)?;
            // Update the latest checkpoint to this block
            self.checkpoint = self.checkpoint.clone().insert(BlockId {
                height: stop_height,
                hash: stop_hash,
            });
            // Finalize the update
            let update = self.finish_update();

            self.send_event(ScanEvent::WalletUpdate(update))?;
        }

        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
        self.shutdown_watcher.abort();
        if let Ok(mut guard) = self.event_tx.lock() {
            guard.take();
        }
    }
}
