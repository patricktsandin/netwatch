# 51. A radio that is gone ends its epoch

Date: 2026-11-02

## Status

Accepted. Extends ADR-0025's probe-identified roles and ADR-0036's radios.

## Context

The radio roles (ADR-0036) are bound by `ProbeIdentified`, as hops are (ADR-0025):
the kernel's route notifications say nothing about them. Nothing unbinds them. When
a USB Wi-Fi adapter is unplugged or the modem goes away, the role keeps its binding
and its epoch stays open: the header still names the device, no `epoch-ended` event
is recorded, and a device that comes back with the same identity starts no new
epoch, so the gap is never marked.

## Decision

- **`ProbeVanished { role }`** is the counterpart of `ProbeIdentified`: the source
  that identifies a role reports that it is gone. Like `ProbeIdentified`, it takes
  effect at once, even while the view is desynchronized, and ends the role's epoch.
- The radio reader sends it when a radio it read before is no longer there. Its last
  grade is dropped with it.
- `specs/topology.qnt` gains the `probeVanish` action and the invariant
  `probeRoleVanishesWithProbes`: a role reported gone stays unbound until it is
  identified again.

## Consequences

- An unplugged radio ends its epoch with an event on its row, and its row's status
  becomes unknown.
- Hops don't use it: a hop that stops answering keeps its identity, because silence
  is loss, not a change of router (ADR-0025).
