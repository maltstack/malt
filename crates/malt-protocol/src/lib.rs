//! VNP message types, wire framing, and protocol primitives for MALT.
//!
//! This crate is the L0 foundation — every other MALT crate depends on it.
//! Message types are generated from `.vexil` schemas by `vexilc` at build time.

pub mod framing;
pub mod priority;
pub mod envelope;

// Generated code — uncomment after build.rs is implemented (Task 2)
// #[allow(clippy::all, unused_qualifications)]
// mod generated;
// pub use generated::malt::*;
