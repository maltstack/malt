//! Capability reporting derived from dispatch behaviour, not encodability.

use crate::protocol::{ElevateCapabilities, OperationCapability, ReasonCode, SCHEMA_VERSION};

/// The protocol version exposed by this helper.
pub const PROTOCOL_VERSION: u32 = 2;

/// Return the operations this build and host can actually perform.
pub fn capabilities() -> ElevateCapabilities {
    ElevateCapabilities {
        protocol_version: PROTOCOL_VERSION,
        operations: vec![
            unavailable("CreateNamespace", linux_reason()),
            unavailable("MountOverlay", linux_reason()),
            unavailable("SetCgroup", linux_reason()),
            unavailable("SetupNetns", linux_reason()),
            unavailable("ApplySeccomp", linux_reason()),
            available("CreateSymlink"),
            unavailable("CreateRestrictedToken", windows_reason()),
            hcs_capability(),
            unavailable("ApplySeatbelt", macos_reason()),
            unavailable(
                "BindPort",
                (
                    ReasonCode::NotImplemented,
                    "BindPort is not implemented by this helper build",
                ),
            ),
        ],
        _unknown: Vec::new(),
    }
}

/// Return the code-generated schema version for diagnostics and handshake checks.
pub fn schema_version() -> &'static str {
    SCHEMA_VERSION
}

fn available(operation: &str) -> OperationCapability {
    OperationCapability {
        operation: operation.into(),
        available: true,
        reason: None,
        detail: None,
        _unknown: Vec::new(),
    }
}

fn unavailable(operation: &str, reason: (ReasonCode, &'static str)) -> OperationCapability {
    OperationCapability {
        operation: operation.into(),
        available: false,
        reason: Some(reason.0),
        detail: Some(reason.1.into()),
        _unknown: Vec::new(),
    }
}

fn linux_reason() -> (ReasonCode, &'static str) {
    if cfg!(target_os = "linux") {
        (
            ReasonCode::NotImplemented,
            "not implemented by this helper build",
        )
    } else {
        (
            ReasonCode::UnsupportedPlatform,
            "unsupported on this platform",
        )
    }
}

fn windows_reason() -> (ReasonCode, &'static str) {
    if cfg!(target_os = "windows") {
        (
            ReasonCode::NotImplemented,
            "not implemented by this helper build",
        )
    } else {
        (
            ReasonCode::UnsupportedPlatform,
            "unsupported on this platform",
        )
    }
}

fn hcs_capability() -> OperationCapability {
    if !cfg!(windows) {
        return unavailable(
            "ManageHcsContainer",
            (
                ReasonCode::UnsupportedPlatform,
                "unsupported on this platform",
            ),
        );
    }
    match malt_platform::isolation::hcs::ensure_hcs_runtime() {
        Ok(()) => available("ManageHcsContainer"),
        Err(error) => OperationCapability {
            operation: "ManageHcsContainer".into(),
            available: false,
            reason: Some(ReasonCode::OsError),
            detail: Some(format!("HCS runtime is unavailable: {error}")),
            _unknown: Vec::new(),
        },
    }
}

fn macos_reason() -> (ReasonCode, &'static str) {
    if cfg!(target_os = "macos") {
        (
            ReasonCode::NotImplemented,
            "not implemented by this helper build",
        )
    } else {
        (
            ReasonCode::UnsupportedPlatform,
            "unsupported on this platform",
        )
    }
}
