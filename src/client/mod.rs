// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Client
//!
//! This module implements a [`Client`], used to request
//! and subscribe to [`Update`]s from a [`Node`].

use core::error;
use core::fmt;
use core::mem;
use std::sync::Arc;

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
use bitcoin::Network;
use bitcoin::ScriptBuf;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::node::error::NodeError;
use crate::node::fsm::State;
use crate::node::Node;

/// The lookahead value to derive spks during [`ScanKind::FullScan`].
pub const FULL_SCAN_LOOKAHEAD: u32 = 1_000;

/// The kind of scan to perform on the [`Wallet`].
///
/// The chosen variant will define how Compact Block Filter
/// scanning will be performed; newly validated blocks will always
/// be used to generate wallet updates after the initial scan finishes.
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
    Update(Update),

    /// The scan has finished.
    Finished,

    /// An error happened whilst scanning.
    Error(String),
}

/// A [`Client`] used to request [`Update`]s from a given [`Node`].
pub struct Client {
    /// The associated [`Node`] the [`Client`] will submit requests to.
    node: Arc<Node>,

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
            shutdown_watcher_tx.lock().await.take();
        });

        // Clone the wallet's `IndexedTxGraph`
        let tx_graph = IndexedTxGraph::new(wallet.spk_index().clone());
        // Use the wallet's last checkpoint as the client's latest checkpoint
        // Used to set the starting height for client scan requests
        let checkpoint = wallet.latest_checkpoint();

        let client = Self {
            node,
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

    /// Request the [`Node`] to perform a scan using the [`Wallet`] associated with this [`Client`].
    ///
    /// The [`kind`](ScanKind) parameter dictates which kind of scan will be performed:
    ///  - [`ScanKind::Sync`]: scan from the [`Wallet`]'s latest checkpoint up to the chain tip
    ///  - [`ScanKind::FullScan`]: scan from genesis up to the chain tip
    ///  - [`ScanKind::Custom`]: scan from a custom start height up to the chain tip
    pub async fn scan(&mut self, scan_params: ScanKind) -> Result<(), ClientError> {
        // Wait for the node to reach operational
        // state before submitting a scan request to it
        self.wait_until_ready().await?;

        // Submit a scan request to the node and wait for the result
        let scan_result = self.scan_inner(scan_params).await;

        // Broadcast a notification if the scan is done or an error occurred
        // Actual wallet updates are sent through the channel by `scan_inner`
        let event = match &scan_result {
            // The scan was successful and is now finished
            Ok(()) => ScanEvent::Finished,
            // An error occurred whilst scanning
            Err(e) => ScanEvent::Error(e.to_string()),
        };
        // Send the final scan event over the channel
        self.send_event(event).await;

        scan_result
    }

    /// Send a [`ScanEvent`] through the channel.
    ///
    /// This is a no-op in the case that the `event_tx` channel
    /// is dropped, usually caused by the [`Node`] being shutdown.
    async fn send_event(&self, event: ScanEvent) {
        if let Some(tx) = self.event_tx.lock().await.as_ref() {
            let _ = tx.send(event);
        }
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
        let ext_key_iter = keychain_index
            .unbounded_spk_iter(KeychainKind::External)
            .ok_or(ClientError::NoExternalKeychain)?;
        let bound = external_index.saturating_add(custom_lookahead) as usize;
        // Add the external spks to the spk store
        spks.extend(ext_key_iter.take(bound).map(|(_, s)| s));

        // Derive `internal_index` + `custom_lookahead` spks from the internal keychain
        let internal_index = last_revealed_indices.get(&KeychainKind::Internal).copied().unwrap_or(0);
        // The internal keychain is optional
        if let Some(int_key_iter) = keychain_index.unbounded_spk_iter(KeychainKind::Internal) {
            let bound = internal_index.saturating_add(custom_lookahead) as usize;
            // Add the internal spks to the spk store
            spks.extend(int_key_iter.take(bound).map(|(_, s)| s));
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
            self.send_event(ScanEvent::Update(update)).await;
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

            // Send the update to subscribers over the channel
            self.send_event(ScanEvent::Update(update)).await;
        }

        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Cancel the client's token when the client is dropped
        // This will trickle down to any child tokens derived by
        // `Client::scan` invocations, aborting any ongoing scans
        self.cancellation_token.cancel();

        // Abort the client's node shutdown watcher task
        self.shutdown_watcher.abort();
    }
}

/// Errors that can occur when interacting with a [`Client`].
#[derive(Debug)]
pub enum ClientError {
    /// The [`Node`] and the [`Wallet`] are not on the same network.
    NetworkMismatch { node_net: Network, wallet_net: Network },

    /// The scan has been aborted by the caller, or the [`Node`] was [`Drop`]ed.
    ScanAborted,

    /// The start height is greater than the stop height.
    InvalidRange { start_height: u32, stop_height: u32 },

    /// The [`Wallet`] has no [external keychain](KeychainKind::External).
    NoExternalKeychain,

    /// The associated [`Node`] is shutting down or inactive.
    UnresponsiveNode,

    /// An error originating from the underlying [`Node`].
    Node(NodeError),
}

#[rustfmt::skip]
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetworkMismatch { node_net, wallet_net } => write!(f, "Network mismatch between node={node_net} and wallet={wallet_net}"),
            Self::ScanAborted => write!(f, "The scan was aborted"),
            Self::InvalidRange { start_height, stop_height } => write!(f, "Invalid scan range: start_height={start_height} > stop_height={stop_height}"),
            Self::NoExternalKeychain => write!(f, "The wallet has no external keychain"),
            Self::UnresponsiveNode => write!(f, "The node is unresponsive"),
            Self::Node(e) => write!(f, "Node Error: {e}"),
        }
    }
}

impl error::Error for ClientError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::NetworkMismatch { .. } => None,
            Self::ScanAborted => None,
            Self::InvalidRange { .. } => None,
            Self::NoExternalKeychain => None,
            Self::UnresponsiveNode => None,
            Self::Node(e) => Some(e),
        }
    }
}

impl From<NodeError> for ClientError {
    fn from(e: NodeError) -> Self {
        Self::Node(e)
    }
}
