//! Elevated helper library for MALT — authenticated request dispatch.
//!
//! `malt-elevate` is a standalone binary that runs with admin/root privileges.
//! It exposes a fixed set of privileged operations, authenticated via a
//! transport once the helper service is installed. The daemon communicates
//! with it over a local IPC channel (named pipe on Windows, Unix socket on
//! Linux/macOS).
//!
//! This module provides the testable library components. The binary entry
//! point is in `main.rs`.

pub mod auth;
pub mod capability;
pub mod dispatch;
pub mod error;
pub mod protocol;
pub mod server;
