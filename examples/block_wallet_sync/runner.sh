#!/bin/sh

# Runner for integration testing between `bdk_floresta` and `utreexod`
#
# | NODE           | P2P   | RPC   |
# | utreexod_alpha | 18333 | 18332 |
# | utreexod_beta  | 28333 | 28332 |
# | utreexod_gamma | 38333 | 38332 |

# Addresses derived from
# `wpkh([925d79b3/84h/1h/0h]tpubDCjEJ6uoRerKY5Wdj2bPdcBDdhgLj6M6nnFNH87gyYRY6FbUqGWQK8WpQt2vtT3xyrqirjmgCGoSaZgoGVecnYouMfXNNHZAQtekm88fE4m/0/*)`
ADDR_0="bcrt1q6rz28mcfaxtmd6v789l9rrlrusdprr9pz3cppk"
ADDR_1="bcrt1qd7spv5q28348xl4myc8zmh983w5jx32cs707jh"
ADDR_2="bcrt1qxdyjf6h5d6qxap4n2dap97q4j5ps6ua8jkxz0z"

# Set up data directory
DATA_DIR="$(pwd)/examples/block_wallet_sync/data"
mkdir -p "$DATA_DIR"/{utreexod_alpha,utreexod_beta,utreexod_gamma}

# Check that `utreexod` is available
echo "Checking if utreexod and utreexoctl are available..."
if ! command -v "utreexod" &> /dev/null; then
  echo "ERROR: utreexod is unavailable at PATH" && exit 1
fi
# Check that `utreexoctl` is available
if ! command -v "utreexoctl" &> /dev/null; then
  echo "ERROR: utreexoctl is unavailable at PATH" && exit 1
fi
echo "OK"

# Kill any lingering `utreexod`s
pkill -9 utreexod
sleep 2

echo "Setting up utreexod alpha..."
utreexod \
  --regtest \
  --datadir="$DATA_DIR/utreexod_alpha" \
  --listen=:18333 \
  --rpclisten=:18332 \
  --rpcuser=glock \
  --rpcpass=glock \
  --flatutreexoproofindex --prune=0 \
  --logdir="$DATA_DIR/utreexod_alpha" \
  --miningaddr="$ADDR_0" \
  >/dev/null 2>&1 &
sleep 2
echo "OK"

# Set up utreexod beta
echo "Setting up utreexod beta..."
utreexod \
  --regtest \
  --datadir="$DATA_DIR/utreexod_beta" \
  --listen=:28333 \
  --rpclisten=:28332 \
  --rpcuser=glock \
  --rpcpass=glock \
  --flatutreexoproofindex --prune=0 \
  --logdir="$DATA_DIR/utreexod_beta" \
  --miningaddr="$ADDR_1" \
  >/dev/null 2>&1 &
sleep 2
echo "OK"

# Set up utreexod gamma
echo "Setting up utreexod gamma..."
utreexod \
  --regtest \
  --datadir="$DATA_DIR/utreexod_gamma" \
  --listen=:38333 \
  --rpclisten=:38332 \
  --rpcuser=glock \
  --rpcpass=glock \
  --flatutreexoproofindex --prune=0 \
  --logdir="$DATA_DIR/utreexod_gamma" \
  --miningaddr="$ADDR_2" \
  >/dev/null 2>&1 &
sleep 2
echo "OK"

echo "Connecting utreexod_alpha, utreexod_beta and utreexod_gamma..."
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:18332 addnode "127.0.0.1:28333" add
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:18332 addnode "127.0.0.1:38333" add
echo "OK"

echo "Mine blocks to\n$ADDR_0,\n$ADDR_1,\n$ADDR_2"
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:18332 generate 102
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:28332 generate 103
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:38332 generate 104

# Run bdk_floresta's `block_wallet_sync` example
echo "Running bdk_floresta regtest example..."
RUST_LOG=debug cargo run --release --example block_wallet_sync
echo "OK"

utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:18332 stop 2>/dev/null
# Stop utreexod beta
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:28332 stop 2>/dev/null
# Stop utreexod gamma
utreexoctl --regtest --rpcuser=glock --rpcpass=glock --rpcserver=127.0.0.1:38332 stop 2>/dev/null
