//! VNP message types, wire framing, and protocol primitives for MALT.
//!
//! This crate is the L0 foundation — every other MALT crate depends on it.
//! Message types are generated from `.vexil` schemas by `vexilc` at build time.

pub mod framing;
pub mod priority;

// Envelope: hand-written helpers wrapping the generated type
pub mod envelope;

// VNP codec constants and helpers
pub mod codec;

// Generated code from vexilc — all domain modules.
//
// The generated code uses `crate::malt::*` paths for cross-module imports,
// so we include it directly at the crate root. This creates `crate::malt`
// which makes `crate::malt::common::Foo` etc. resolve correctly.
//
// The include chain is:
//   OUT_DIR/mod.rs  ->  pub mod malt;
//   OUT_DIR/malt/mod.rs  ->  pub mod common; pub mod shell; ...
//   OUT_DIR/malt/common.rs  ->  actual type definitions
include!(concat!(env!("OUT_DIR"), "/mod.rs"));

// Re-export domain modules at crate root for ergonomic access:
//   malt_protocol::common::PaneId instead of malt_protocol::malt::common::PaneId
pub use malt::common;
pub use malt::elevate;
pub use malt::frame_element;
pub use malt::handshake;
pub use malt::input;
pub use malt::mux;
pub use malt::persist;
pub use malt::render;
pub use malt::session;
pub use malt::shell;
pub use malt::system;
pub use malt::task;

// Make the generated envelope module accessible under a different name
// to avoid collision with the hand-written envelope module
pub(crate) use malt::envelope as envelope_generated;
