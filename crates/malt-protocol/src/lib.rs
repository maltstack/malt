//! VNP message types, wire framing, and protocol primitives for MALT.
//!
//! This crate is the L0 foundation — every other MALT crate depends on it.
//! Message types are generated from `.vexil` schemas by `vexilc` at build time.

pub mod framing;
pub mod identity;
pub mod priority;

/// Vexil wire primitives used by generated message types. Consumers that need
/// to frame a generated protocol message use this re-export instead of taking
/// a second, potentially divergent runtime dependency.
pub use vexil_runtime;

/// Stable framing discriminants for the elevated-helper VNP channel.
///
/// The payload bodies are generated from `schemas/elevate.vexil`; these values
/// only select which generated message a framed body carries.
pub mod elevate_channel {
    pub const HELLO: u8 = 1;
    pub const HELLO_ACK: u8 = 2;
    pub const REQUEST: u8 = 3;
    pub const RESPONSE: u8 = 4;
    pub const DAEMON_ENROLLMENT_REQUEST: u8 = 5;
    pub const DAEMON_ENROLLMENT_RESPONSE: u8 = 6;
    pub const SESSION_ENTITLEMENT_REQUEST: u8 = 7;
    pub const SESSION_ENTITLEMENT_RESPONSE: u8 = 8;
}

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
//
// Lints are scoped off the generated tree rather than satisfied inside it.
// These files are rewritten by vexilc on every build, so any fix here would
// be erased, and what the linter objects to is codegen style -- a redundant
// borrow, a same-type cast, an unused glob import -- not defects. The point
// of a deny-warnings gate is to hold *hand-written* code to a standard;
// silencing it crate-wide would defeat that, so the exemption covers the
// generated module and nothing else. The codegen warts themselves are
// recorded for upstream report in `docs/BACKLOG.md`.
//
// `#[allow]` cannot attach to an `include!` invocation, so the include lives
// inside a wrapper module that carries the attribute. `malt` is re-exported
// at the crate root immediately below, which is what keeps the generated
// code's own `crate::malt::*` cross-module paths resolving.
#[allow(warnings)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod.rs"));
}
pub use generated::malt;

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
