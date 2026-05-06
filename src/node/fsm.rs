// SPDX-License-Identifier: MIT OR Apache-2.0

//! # Finite-State Machine Logic for the [`Node`]
//!
//! The [`Node`] can be thought of as a [Finite-State Machine].
//!
//! This module implements all of the possible [`State`]s that the
//! [`Node`] can be in, as well as the corresponding next-state logic.
//!
//! [Finite-State Machine]: https://en.wikipedia.org/wiki/Finite-state_machine

use core::fmt;

#[allow(unused)]
use bitcoin::Block;
#[allow(unused)]
use floresta_wire::address_man::AddressMan;
use tracing::warn;

#[allow(unused)]
use crate::node::Node;

/// How many blocks behind the chain tip the node can be while still being
/// considered [`State::Operational`]. Prevents flapping at the chain tip
/// when a new block arrives and validation briefly lags.
const OPERATIONAL_TOLERANCE: u32 = 6;

/// The set of [`State`]s the [`Node`] can possibly be in.
///
/// The [`Node`] can be modelled as a Finite State Machine.
/// As such, we can enumerate the set of possible states it
/// can be in. This allows restricting certain interactions
/// with the [`Node`] to when it's in a well-defined set of
/// [`State`]s. Upstream applications can also use this to
/// display the [`Node`]'s [`State`] to their users.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// S0: The [`Node`] is not running.
    Inactive,

    /// S1: The [`Node`] is active, but not in a well-defined state.
    Active,

    // TODO(@luisschwab): how do we figure out when we are in this state?
    /// S2: The [`Node`] is bootstrapping its [address manager](AddressMan) from DNS seeders.
    DnsBootstrapping,

    /// S3: The [`Node`] is synchronizing headers from its peers.
    HeaderSync { height: u32 },

    /// S4: The [`Node`] is performing Initial Block Download.
    InitialBlockDownload { chain_height: u32, node_height: u32 },

    /// S5: The [`Node`] is downloading Compact Block Filters from its peers.
    CompactBlockFilterDownload { filter_height: u32 },

    // TODO(@luisschwab): how do we figure out when we are in this state?
    /// S6: The [`Node`] is performing backfill.
    Backfill,

    /// S7: The [`Node`] is fully operational.
    Operational,

    /// S8: The [`Node`] is in the process of shutting down.
    ShuttingDown,
}

#[rustfmt::skip]
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inactive => write!(f, "Inactive"),
            Self::Active => write!(f, "Active"),
            Self::DnsBootstrapping => write!(f, "Address Manager Bootstrap"),
            Self::HeaderSync { height } => write!(f, "Header Sync at height={height}"),
            Self::InitialBlockDownload { chain_height, node_height } => write!(f, "IBD at chain_height={chain_height} and node_height={node_height}"),
            Self::CompactBlockFilterDownload { filter_height } => write!(f, "CBF Download at filter_height={filter_height}"),
            Self::Backfill => write!(f, "Backfilling"),
            Self::Operational => write!(f, "Operational"),
            Self::ShuttingDown => write!(f, "Shutting Down"),
        }
    }
}

// TODO(@luisschwab): missing
// - S2 (DNS Bootstrap)
// - S6 (Backfill)
/// Continuously update the [`Node`]'s [`State`].
///
/// Since the [`Node`] can be modelled as a Finite State Machine,
/// we compute the next [`State`] from the current [`State`] and
/// external inputs.
///
/// See `doc/FSM.md` for FSM modelling and next-state logic.
pub fn compute_next_state(
    wire_ready: bool,
    current_state: &State,
    node_height: u32,
    chain_height: u32,
    filter_height: u32,
) -> State {
    // The filter tip should not be greater than node's tip
    //
    // Something went wrong if this is the case, so
    // fallback to the minimum between the two values.
    let filter_height = if filter_height > node_height {
        warn!("The filter height={filter_height} is greater than the node height={node_height}");
        warn!("Falling back to the node height={node_height}");
        node_height
    } else {
        filter_height
    };

    match current_state {
        // Skip these variants since their
        // next-state logic is handled externally:
        // - S0 (Inactive): handled by the shutdown task
        // - S8 (ShuttingDown): handled by the shutdown task
        //
        // Skip these variants since their next
        // state logic still needs figuring out
        // - S2 (DnsBootstrapping)
        // - S6 (Backfill)
        s @ State::ShuttingDown | s @ State::Inactive | s @ State::DnsBootstrapping | s @ State::Backfill => s.clone(),

        _ => {
            // Wait for the inner node to be ready
            // before transitioning to the next state.
            if !wire_ready {
                State::Active
            } else if node_height == 0 && chain_height > 0 {
                State::HeaderSync { height: chain_height }
            } else if node_height + OPERATIONAL_TOLERANCE < chain_height {
                State::InitialBlockDownload {
                    chain_height,
                    node_height,
                }
            } else if filter_height < chain_height {
                State::CompactBlockFilterDownload { filter_height }
            } else if node_height > 0 {
                State::Operational
            } else {
                State::Active
            }
        }
    }
}
