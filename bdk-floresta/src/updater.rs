// SPDX-License-Identifier: MIT

use std::sync::Arc;

use bdk_wallet::bitcoin::Block;
use bdk_wallet::Wallet;
use floresta_chain::BlockConsumer;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{RwLock, RwLockWriteGuard};
use tracing::error;

pub struct WalletUpdater {
    wallet: Arc<RwLock<Wallet>>,
    sender: UnboundedSender<()>,
}

impl WalletUpdater {
    pub fn new(
        wallet: Arc<RwLock<Wallet>>,
        sender: UnboundedSender<()>,
    ) -> Self {
        Self { wallet, sender }
    }
}

impl BlockConsumer for WalletUpdater {
    fn wants_spent_utxos(&self) -> bool {
        false
    }

    fn on_block(
        &self,
        block: &Block,
        height: u32,
        _spent_utxos: Option<
            &std::collections::HashMap<
                bdk_wallet::bitcoin::OutPoint,
                floresta_chain::UtxoData,
            >,
        >,
    ) {
        let wallet: Arc<RwLock<Wallet>> = self.wallet.clone();
        let block: Block = block.clone();
        let sender: UnboundedSender<()> = self.sender.clone();

        tokio::spawn(async move {
            // Get a lock on the wallet.
            let mut wallet: RwLockWriteGuard<'_, Wallet> = wallet.write().await;

            // Attempt to apply the block to the wallet.
            if let Err(e) = wallet.apply_block(&block, height) {
                error!(
                    "failed to apply block of height {} to the wallet: {}",
                    height, e
                );
                return;
            }

            // Attempt to send the block over the channel.
            if let Err(e) = sender.send(()) {
                error!("failed to send block update over the channel: {}", e);
            }
        });
    }
}
