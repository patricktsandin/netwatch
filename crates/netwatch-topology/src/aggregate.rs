//! The topology aggregate: the only allocator of identity epochs.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::num::NonZeroU64;

use netwatch_shared::{Identity, IdentityEpoch, Role};
use netwatch_verified::MonotonicCounter;

use crate::{Command, Observation, Output, ResyncGeneration, Snapshot, TopologyEvent};

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
/// - how epochs are allocated (the verified crate's [`MonotonicCounter`])
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
pub struct Topology {
    bindings: BTreeMap<Role, Binding>,
    epochs: MonotonicCounter,
    generations: MonotonicCounter,
    /// The generation of the most recent loss, or 0 if there has been none.
    generation: ResyncGeneration,
    sync: Sync,
}

#[derive(Debug)]
enum Sync {
    Synchronized,
    /// Waiting for the snapshot of the current generation. The notifications
    /// received in the meantime are held, in order, to be replayed onto it.
    Desynchronized {
        buffered: Vec<RoleChange>,
    },
}

/// The part of an [`Observation`] that describes a single role. Only these can be
/// buffered; buffering a loss or a snapshot would be meaningless.
#[derive(Debug)]
enum RoleChange {
    Bound { role: Role, identity: Identity },
    Vanished { role: Role },
}

impl Topology {
    /// A view with no bound roles, synchronized, with no epochs allocated yet.
    pub fn new() -> Self {
        Self {
            bindings: BTreeMap::new(),
            epochs: MonotonicCounter::new(),
            generations: MonotonicCounter::new(),
            generation: ResyncGeneration::new(0),
            sync: Sync::Synchronized,
        }
    }

    /// Applies one observation, returning the events and commands it caused, in
    /// order.
    pub fn observe(&mut self, observation: Observation) -> Vec<Output> {
        let mut outputs = Vec::new();
        match observation {
            Observation::IdentityObserved { role, identity } => {
                self.change(RoleChange::Bound { role, identity }, &mut outputs);
            }
            Observation::RoleVanished { role } => {
                self.change(RoleChange::Vanished { role }, &mut outputs);
            }
            Observation::NotificationsLost => self.lose(&mut outputs),
            Observation::SnapshotReceived(snapshot) => self.resync(snapshot, &mut outputs),
        }
        outputs
    }

    /// The role's current binding, or `None` if it is unbound. While
    /// desynchronized, this is the last binding known before the loss: still a
    /// truthful pin, though possibly out of date (ADR-0020).
    pub fn binding(&self, role: &Role) -> Option<&Binding> {
        self.bindings.get(role)
    }

    /// Whether the view can currently be trusted to match the kernel.
    pub fn is_synchronized(&self) -> bool {
        matches!(self.sync, Sync::Synchronized)
    }

    /// Applies a notification now, or holds it for the snapshot. Applying it while
    /// desynchronized would let a snapshot taken earlier undo it (ADR-0021).
    fn change(&mut self, change: RoleChange, outputs: &mut Vec<Output>) {
        match &mut self.sync {
            Sync::Desynchronized { buffered } => buffered.push(change),
            Sync::Synchronized => match change {
                RoleChange::Bound { role, identity } => self.bind(role, identity, outputs),
                RoleChange::Vanished { role } => self.end(&role, outputs),
            },
        }
    }

    /// Starts a new generation and drops anything buffered: it all predates the
    /// snapshot this generation will request (ADR-0021).
    fn lose(&mut self, outputs: &mut Vec<Output>) {
        let generation = ResyncGeneration::new(
            self.generations
                .allocate()
                .expect("u64 generations cannot be exhausted: one loss per ns lasts 584 years"),
        );
        self.generation = generation;
        self.sync = Sync::Desynchronized {
            buffered: Vec::new(),
        };
        outputs.push(Output::Event(TopologyEvent::Desynchronized { generation }));
        outputs.push(Output::Command(Command::Resync { generation }));
    }

    /// Replays the buffered notifications onto the snapshot's state, then
    /// reconciles every affected role with the result, in ascending role order so
    /// that epoch allocation is deterministic. The replay acts on plain state, not
    /// on bindings, so states that are immediately overwritten never allocate an
    /// epoch.
    fn resync(&mut self, snapshot: Snapshot, outputs: &mut Vec<Output>) {
        let Sync::Desynchronized { buffered } = &mut self.sync else {
            return;
        };
        if snapshot.generation != self.generation {
            return;
        }
        let mut target = snapshot.bindings;
        for change in mem::take(buffered) {
            match change {
                RoleChange::Bound { role, identity } => {
                    target.insert(role, identity);
                }
                RoleChange::Vanished { role } => {
                    target.remove(&role);
                }
            }
        }
        let affected: BTreeSet<Role> = self.bindings.keys().chain(target.keys()).cloned().collect();
        for role in affected {
            match target.remove(&role) {
                Some(identity) => self.bind(role, identity, outputs),
                None => self.end(&role, outputs),
            }
        }
        self.sync = Sync::Synchronized;
        outputs.push(Output::Event(TopologyEvent::Resynchronized {
            generation: self.generation,
        }));
    }

    /// Binds `role` to `identity`, unless it already holds exactly that identity, in
    /// which case nothing happens. Otherwise the old epoch (if any) ends before the
    /// new one starts.
    fn bind(&mut self, role: Role, identity: Identity, outputs: &mut Vec<Output>) {
        if self
            .bindings
            .get(&role)
            .is_some_and(|binding| binding.identity == identity)
        {
            return;
        }
        self.end(&role, outputs);
        let epoch = self.allocate_epoch();
        outputs.push(Output::Event(TopologyEvent::EpochStarted {
            epoch,
            role: role.clone(),
            identity: identity.clone(),
        }));
        self.bindings.insert(role, Binding { identity, epoch });
    }

    fn end(&mut self, role: &Role, outputs: &mut Vec<Output>) {
        if let Some(binding) = self.bindings.remove(role) {
            outputs.push(Output::Event(TopologyEvent::EpochEnded {
                epoch: binding.epoch,
                role: role.clone(),
            }));
        }
    }

    fn allocate_epoch(&mut self) -> IdentityEpoch {
        let raw = self
            .epochs
            .allocate()
            .expect("u64 epochs cannot be exhausted: one change per ns lasts 584 years");
        IdentityEpoch::new(NonZeroU64::new(raw).expect("verified: MonotonicCounter never yields 0"))
    }
}

impl Default for Topology {
    fn default() -> Self {
        Self::new()
    }
}
