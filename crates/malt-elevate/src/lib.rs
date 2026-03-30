//! Elevated helper library for MALT — nonce auth and request dispatch.
//!
//! `malt-elevate` is a standalone binary that runs with admin/root privileges.
//! It exposes a fixed set of privileged operations, authenticated via a
//! single-use nonce. The daemon communicates with it over a local IPC channel
//! (named pipe on Windows, Unix socket on Linux/macOS).
//!
//! This module provides the testable library components. The binary entry
//! point is in `main.rs`.

pub mod auth;
pub mod dispatch;
pub mod error;
pub mod protocol;
