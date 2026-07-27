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
pub mod error;
pub mod protocol;

// The privileged helper is Windows-only by design -- spec 008's Assumptions
// say so explicitly: "Non-Windows hosts report the helper as unavailable,
// honestly." These modules bind to the Windows service, named-pipe and HCS
// primitives that `malt-platform` only provides there.
//
// They are compiled out rather than given stub implementations. A stub helper
// that appears to exist, accepts requests and reports success is precisely the
// fail-open spec 008 was written to remove; the daemon already refuses these
// operations on non-Windows with a stated reason
// (`elevate_client::manage_image`), which is where that answer belongs.
#[cfg(windows)]
pub mod capability;
#[cfg(windows)]
pub mod dispatch;
#[cfg(windows)]
pub mod entitlement;
#[cfg(windows)]
pub mod server;
