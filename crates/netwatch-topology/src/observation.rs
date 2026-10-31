//! What the adapters tell the topology aggregate.

use std::collections::BTreeMap;

use netwatch_shared::{Identity, Role};

/// An input to [`Topology::observe`](crate::Topology::observe). Adapters build these
/// from kernel notifications, once they have been translated into domain terms.
///
/// Each notification carries the role's complete new state, never a delta. That is
/// what makes it harmless for a notification to be replayed late (ADR-0021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The kernel now binds `role` to `identity`.
    IdentityObserved {
        /// The role whose identity was observed.
        role: Role,
        /// The identity the kernel now binds it to.
        identity: Identity,
    },
    /// The kernel no longer binds `role` to anything.
    RoleVanished {
        /// The role that is now unbound.
        role: Role,
    },
    /// Notifications were lost (`ENOBUFS`), and the adapter has **already drained**
    /// its socket, discarding everything that was queued on it. Reporting this
    /// before the drain finishes breaks convergence (ADR-0021).
    NotificationsLost,
    /// The full kernel state, dumped in response to [`Command::Resync`](crate::Command::Resync).
    SnapshotReceived(Snapshot),
}

/// The kernel's complete topology at one moment, as dumped for a resync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The generation of the [`Command::Resync`](crate::Command::Resync) this answers.
    pub generation: ResyncGeneration,
    /// Every bound role. A role that is missing from the map is unbound.
    pub bindings: BTreeMap<Role, Identity>,
}

/// Counts notification losses. Only a snapshot of the current generation can
/// restore synchronization. One from an earlier generation was requested before a
/// later loss, so it can't account for what that loss dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResyncGeneration(u64);

impl ResyncGeneration {
    /// Wraps a generation number. Only adapters echoing a `Resync` command, and
    /// tests, construct these.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The generation number.
    pub const fn get(self) -> u64 {
        self.0
    }
}
