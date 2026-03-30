//! Session lifecycle, pane runtime, group policy, and input authority for MALT.
//!
//! L1 crate depending only on `malt-protocol` for schema types. Pure logic — no
//! I/O, no platform calls. The daemon (Phase 3) uses these types and state
//! machines to manage sessions. All state transitions are validated; invalid
//! transitions return errors.

pub mod authority;
pub mod group;
pub mod pane;
pub mod session;
