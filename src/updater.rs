// SPDX-License-Identifier: MIT

//! TODO

use bitcoin::Block;
use floresta_chain::BlockConsumer;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;
use tracing::error;

/// Structures that represent an update to the wallet.
pub enum WalletUpdate {
    /// A new block update, represented by a `Block` and it's height.
    NewBlock(Block, u32),
}

/// The `WalletUpdater` sends wallet updates over the channel to a subscriber.
pub struct WalletUpdater {
    /// The sender which wallet updates should be forwarded by.
    sender: UnboundedSender<WalletUpdate>,
}

impl WalletUpdater {
    pub fn new(sender: UnboundedSender<WalletUpdate>) -> Self {
        Self { sender }
    }
}

impl BlockConsumer for WalletUpdater {
    /// This is unecessary, so just return `false`.
    fn wants_spent_utxos(&self) -> bool {
        false
    }

    /// The action to be taken when a new `Block` is received.
    /// In our case, the `Block` is wrapped in an `Update::Block` along with
    /// it's height.
    fn on_block(
        &self,
        block: &Block,
        height: u32,
        _spent_utxos: Option<
            &std::collections::HashMap<bdk_wallet::bitcoin::OutPoint, floresta_chain::UtxoData>,
        >,
    ) {
        // Create the `Update`.
        let update = WalletUpdate::NewBlock(block.clone(), height);
        let update_height = height;

        // Send the update over the channel.
        match self.sender.send(update) {
            Ok(_) => {
                debug!(
                    "Sent block update at height {} over the channel",
                    update_height
                );
            }
            Err(e) => {
                error!(
                    "Failed to send block update at height {} over the channel: {}",
                    update_height, e
                );
            }
        }
    }
}
