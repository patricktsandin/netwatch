//! Anti-corruption layer for ModemManager over D-Bus.
//!
//! This crate translates the modem state into two kinds of observation. The *serving
//! cell* goes to Topology as an identity. Signal quality and access technology go to
//! Host health as gauges (see `docs/context-map.md`).
