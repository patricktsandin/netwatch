# 14. Type-driven domain modeling

Date: 2026-10-31

## Status

Accepted

## Context

Networking code is full of values that are easy to confuse: TTL vs. hop index,
interface index vs. ifname, microseconds vs. milliseconds, sequence numbers that wrap.
The options considered were: newtypes only where values are easy to confuse; newtypes
for every domain value (the "parse, don't validate" approach); or newtypes plus a units
library that checks physical units at compile time.

## Decision

- **Parse, don't validate:** adapters parse raw data into domain types once. From then
  on, invalid values can't be represented.
- **A newtype for every domain value** (`HopTtl`, `InterfaceIndex`, `ProbeSequence`,
  `IdentityEpoch`, …). No bare integers or `String`s cross a module boundary.
- **Physical quantities use `uom`** (`Time`, `Information`, `InformationRate`), so units
  never appear in names inside the code. Convert to integers only at the persistence and
  wire boundaries, where names carry the unit (`rtt_us`).
- **The verified crate uses plain integer newtypes**, because Verus can't reason about
  `uom`. It converts at its interface.
- **Typestate** wherever a lifecycle exists. For example, a probe goes from scheduled to
  in flight, and then to answered, timed out, or errored. An illegal transition fails to
  compile.
- Use enums instead of booleans, and sum types instead of optional fields that depend on
  each other.

## Consequences

- A whole class of bugs disappears at compile time, and function signatures document
  themselves.
- More boilerplate at boundaries, and `uom`'s generics add compile time and
  sometimes make error messages hard to read.
- Converting at the edge of the verified crate is a deliberate seam, and it needs tests.
