//! The open host service: the REST and WebSocket API, plus the embedded SPA
//! (ADR-0005, ADR-0019).
//!
//! The API types defined here are the published language for the browser. The
//! TypeScript versions are generated from them with `ts-rs`. Changing one of these
//! types changes the contract, so update the frontend's boundary parser in the
//! same commit.
