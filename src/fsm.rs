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

use crate::node::Action;
#[allow(unused)]
use crate::node::Node;

/// The set of [`State`]s the [`Node`] can possibly be in.
///
/// The [`Node`] can be modelled as a Finite State Machine.
/// As such, we can enumerate the set of possible states it
/// can be in. This allows restricting certain interactions
/// with the [`Node`] to when it's in a well-defined set of
/// [`State`]s. Upstream applications can also use this to
/// display the [`Node`]'s [`State`] to their users.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// The [`Node`] is not running (S0).
    Inactive,
    /// The [`Node`] is active, but not in a well-defined state (S1).
    Active,
    // TODO(@luisschwab): how do we figure out when we are in this state?
    /// The [`Node`] is bootstrapping its [address manager](AddressMan) from DNS seeders (S2).
    DnsBootstrapping,
    /// The [`Node`] is synchronizing headers from its peers (S3).
    HeaderSync(u32),
    /// The [`Node`] is performing Initial Block Download (S4).
    InitialBlockDownload((u32, u32)),
    /// The [`Node`] is downloading Compact Block Filters from its peers (S5).
    CompactBlockFilterDownload(u32),
    /// The [`Node`] is performing backfill (S6).
    Backfill,
    /// The [`Node`] is fully operational (S7).
    Operational,
    /// The [`Node`] is performing an [`Action`] (S8).
    PerformingAction(Action),
    /// The [`Node`] is in the process of shutting down (S9).
    ShuttingDown,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::DnsBootstrapping => {
                write!(f, "Bootstrapping the Address Manager from DNS seeders")
            }
            Self::HeaderSync(height) => write!(f, "Synchronizing Headers at height={}", height),
            Self::InitialBlockDownload((node_tip, chain_tip)) => {
                write!(f, "Performing IBD [{}/{}] ", node_tip, chain_tip)
            }
            Self::CompactBlockFilterDownload(height) => {
                write!(f, "Downloading Compact Block Filters at height={}", height)
            }
            Self::Backfill => write!(f, "Performing Backfill"),
            Self::Operational => write!(f, "Operational"),
            Self::PerformingAction(action) => write!(f, "Performing {}", action),
            Self::ShuttingDown => write!(f, "Shuting Down"),
            Self::Inactive => write!(f, "Inactive"),
        }
    }
}

// TODO(@luisschwab): missing
// - S2 (DNS Bootstrap)
// - S6 (Backfill)
/// Continuously update the [`Node`]'s [`State`].
///
/// Since the [`Node`] is an Finite State Machine,
/// we compute the next [`State`] from the current
/// [`State`] and external inputs. See `doc/FSM.md`
/// for the FSM model and next-state logic.
pub fn compute_next_state(
    current_state: State,
    node_tip: u32,
    chain_tip: u32,
    filter_tip: u32,
) -> State {
    match current_state {
        // S1: Active
        State::Active => {
            if node_tip == 0 && chain_tip > 0 {
                State::HeaderSync(chain_tip)
            } else {
                State::Active
            }
        }
        // S3: HeaderSync
        State::HeaderSync(_) => {
            if node_tip == 0 && chain_tip > 0 {
                State::HeaderSync(chain_tip)
            } else {
                State::InitialBlockDownload((node_tip, chain_tip))
            }
        }
        // S4: InitialBlockDownload
        State::InitialBlockDownload(_) => {
            if node_tip != chain_tip {
                State::InitialBlockDownload((node_tip, chain_tip))
            } else {
                State::CompactBlockFilterDownload(filter_tip)
            }
        }
        // S5: Compact Block Filter Download
        State::CompactBlockFilterDownload(_) => {
            if filter_tip != chain_tip && filter_tip != node_tip {
                State::CompactBlockFilterDownload(filter_tip)
            } else {
                State::Operational
            }
        }
        // S7: Operational
        State::Operational => State::Operational,

        // Skip these variants since their
        // next-state logic is handled externally:
        // - S0 (Inactive): handled by the shutdown task
        // - S8 (PerformingAction): handled by `Node` methods
        // - S9 (ShuttingDown): handled by the shutdown task
        //
        // Skip these variants since their next
        // state logic still needs figuring out
        // - S2 (DnsBootstrapping)
        // - S6 (Backfill)
        s @ State::PerformingAction(_)
        | s @ State::ShuttingDown
        | s @ State::Inactive
        | s @ State::DnsBootstrapping
        | s @ State::Backfill => s,
    }
}
