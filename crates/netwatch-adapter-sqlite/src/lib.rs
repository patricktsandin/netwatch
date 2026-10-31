//! Persistence for the History context: schema, numbered migrations, the single writer
//! task, and the read queries (ADR-0006).
//!
//! Here, and only here, quantities become unit-suffixed integers (`rtt_us`), and
//! *counter readings* are stored raw so that rates can be computed at query time.
//! A migration that has been applied is never edited.
