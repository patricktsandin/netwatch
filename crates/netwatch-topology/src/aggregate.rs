//! The topology aggregate: the only allocator of identity epochs.

use netwatch_shared::{Identity, IdentityEpoch, Role};

use crate::{Observation, Output};

/// A role's current identity, and the epoch in which it holds that identity.
/// A probe takes a copy of this as its *pin* (ADR-0020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// What the role is bound to.
    pub identity: Identity,
    /// The epoch during which it holds that identity.
    pub epoch: IdentityEpoch,
}

/// netwatch's view of which identity each role is bound to, kept in step with the
/// kernel despite notification lag and loss.
///
/// This is the pure state machine specified by the "Aggregate" section of
/// `specs/topology.qnt`, and it is held to that spec by trace conformance. It hides:
/// - how epochs are allocated (the verified crate's `EpochCounter`)
/// - the resync protocol: generations, buffering while desynchronized, and
///   reconciling a snapshot with the buffered notifications (ADR-0020, ADR-0021)
///
/// What it guarantees:
/// - Epochs are allocated densely from 1, never reused, and each names exactly one
///   role and identity.
/// - A repeated notification never splits an epoch.
/// - Once synchronized, with nothing left pending, the bindings equal the kernel's
///   actual state.
#[derive(Debug)]
pub struct Topology {}

impl Topology {
    /// A view with no bound roles, synchronized, with no epochs allocated yet.
    pub fn new() -> Self {
        todo!()
    }

    /// Applies one observation, returning the events and commands it caused, in
    /// order.
    pub fn observe(&mut self, observation: Observation) -> Vec<Output> {
        let _ = observation;
        todo!()
    }

    /// The role's current binding, or `None` if it is unbound. While
    /// desynchronized, this is the last binding known before the loss: still a
    /// truthful pin, though possibly out of date (ADR-0020).
    pub fn binding(&self, role: &Role) -> Option<&Binding> {
        let _ = role;
        todo!()
    }

    /// Whether the view can currently be trusted to match the kernel.
    pub fn is_synchronized(&self) -> bool {
        todo!()
    }
}

impl Default for Topology {
    fn default() -> Self {
        Self::new()
    }
}
