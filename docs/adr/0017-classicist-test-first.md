# 17. Classicist, test-first testing

Date: 2026-10-31

## Status

Accepted

## Context

The schools considered were:
- **Classicist (Chicago/Detroit):** Beck; Khorikov's *Unit Testing*. Real objects,
  verifying behavior and resulting state.
- **London/mockist:** *Growing Object-Oriented Software, Guided by Tests*. Outside-in,
  with collaborators mocked. Drives interface design, but the tests are brittle.
- **Test-after** with a high bar for coverage.

The sans-IO core (ADR-0012) makes the domain testable without fakes of any kind.

## Decision

- Write the failing test before the code.
- Test behavior through public interfaces, with real domain objects.
- Fakes replace only the process boundary (sockets, netlink, the filesystem, the clock).
  Never mock collaborators inside the domain. Domain tests feed in events and assert on
  the commands and facts that come out.
- Test names state the behavior in glossary terms:
  `route_change_starts_new_identity_epoch_for_affected_hops`.
- Integration tests that need privileges build throwaway `ip netns` topologies (veth
  pairs and routers), clean them up even when a test fails, and are `#[ignore]`d by
  default.

## Consequences

- Tests survive refactoring and document the behavior.
- Integration coverage of the adapters needs root, so it runs on demand rather than
  with every `cargo test`.
