//! What the topology aggregate emits.

use netwatch_shared::{Identity, IdentityEpoch, Role};

use crate::ResyncGeneration;

/// One output of [`Topology::observe`](crate::Topology::observe). The caller must
/// handle outputs in the order they are returned. For example, an `EpochEnded` always
/// precedes the `EpochStarted` that replaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A fact to persist and publish.
    Event(TopologyEvent),
    /// Something the shell must do.
    Command(Command),
}

/// A *topology event*: a *domain event* recording a change in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyEvent {
    /// `role` is now bound to `identity`, for the new `epoch`.
    EpochStarted {
        /// The newly allocated epoch.
        epoch: IdentityEpoch,
        /// The role that was bound.
        role: Role,
        /// The identity it is now bound to.
        identity: Identity,
    },
    /// `role`'s binding for `epoch` has ended. The role is now either unbound or
    /// about to start a new epoch.
    EpochEnded {
        /// The epoch that ended.
        epoch: IdentityEpoch,
        /// The role whose binding ended.
        role: Role,
    },
    /// Notifications were lost, so the view can no longer be trusted until
    /// `Resynchronized` arrives for the same generation.
    Desynchronized {
        /// The generation started by this loss.
        generation: ResyncGeneration,
    },
    /// The view has been reconciled with a snapshot and can be trusted again.
    Resynchronized {
        /// The generation the snapshot answered.
        generation: ResyncGeneration,
    },
}

/// A *command*: something the shell must do for the aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Dump the kernel's full topology and return it as a
    /// [`Snapshot`](crate::Snapshot) carrying this generation.
    Resync {
        /// The generation the snapshot must carry.
        generation: ResyncGeneration,
    },
}
