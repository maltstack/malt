//! Request dispatch for generated `ElevateRequest` values.
//!
//! An outcome is only `Performed` after its handler completes the requested
//! effect. Everything else is an explicit refusal or an indeterminate result.

use std::path::Path;

use crate::protocol::{ElevateRequest, ElevateResponse, OutcomeKind, ReasonCode};

/// Dispatch a request to the operation handler.
pub fn dispatch_request(request_id: u32, request: &ElevateRequest) -> ElevateResponse {
    match request {
        ElevateRequest::CreateNamespace { .. } => {
            unsupported_or_unimplemented(request_id, "CreateNamespace", cfg!(target_os = "linux"))
        }
        ElevateRequest::MountOverlay { .. } => {
            unsupported_or_unimplemented(request_id, "MountOverlay", cfg!(target_os = "linux"))
        }
        ElevateRequest::SetCgroup { .. } => {
            unsupported_or_unimplemented(request_id, "SetCgroup", cfg!(target_os = "linux"))
        }
        ElevateRequest::SetupNetns { .. } => {
            unsupported_or_unimplemented(request_id, "SetupNetns", cfg!(target_os = "linux"))
        }
        ElevateRequest::ApplySeccomp { .. } => {
            unsupported_or_unimplemented(request_id, "ApplySeccomp", cfg!(target_os = "linux"))
        }
        ElevateRequest::CreateSymlink { target, link } => {
            dispatch_create_symlink(request_id, target, link)
        }
        ElevateRequest::CreateRestrictedToken { .. } => unsupported_or_unimplemented(
            request_id,
            "CreateRestrictedToken",
            cfg!(target_os = "windows"),
        ),
        ElevateRequest::ManageHcsContainer { .. } => unsupported_or_unimplemented(
            request_id,
            "ManageHcsContainer",
            cfg!(target_os = "windows"),
        ),
        ElevateRequest::ApplySeatbelt { .. } => {
            unsupported_or_unimplemented(request_id, "ApplySeatbelt", cfg!(target_os = "macos"))
        }
        ElevateRequest::BindPort { .. } => refused(
            request_id,
            ReasonCode::NotImplemented,
            "BindPort is not implemented by this helper build",
        ),
        ElevateRequest::Unknown { .. } => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unknown elevate operation cannot be validated",
        ),
        _ => refused(
            request_id,
            ReasonCode::InvalidParameters,
            "unrecognized elevate operation cannot be validated",
        ),
    }
}

/// Construct a refusal for an operation the current host cannot perform.
pub fn unsupported_or_unimplemented(
    request_id: u32,
    operation: &str,
    supported_platform: bool,
) -> ElevateResponse {
    let (reason, detail) = if supported_platform {
        (
            ReasonCode::NotImplemented,
            format!("{operation} is not implemented by this helper build"),
        )
    } else {
        (
            ReasonCode::UnsupportedPlatform,
            format!("{operation} is unsupported on this platform"),
        )
    };
    refused(request_id, reason, detail)
}

/// Construct a refusal. Refused outcomes never carry an effect payload.
pub fn refused(request_id: u32, reason: ReasonCode, detail: impl Into<String>) -> ElevateResponse {
    ElevateResponse {
        request_id,
        kind: OutcomeKind::Refused,
        reason: Some(reason),
        detail: Some(detail.into()),
        payload: None,
        _unknown: Vec::new(),
    }
}

fn dispatch_create_symlink(request_id: u32, target: &str, link: &str) -> ElevateResponse {
    tracing::info!(target, link, "creating symlink through malt-platform");
    match malt_platform::fs::create_symlink(Path::new(target), Path::new(link)) {
        Ok(()) => ElevateResponse {
            request_id,
            kind: OutcomeKind::Performed,
            reason: None,
            detail: None,
            payload: Some(Vec::new()),
            _unknown: Vec::new(),
        },
        Err(error) => refused(
            request_id,
            ReasonCode::OsError,
            format!("CreateSymlink failed: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malt_protocol::common::IsolationTier;
    use malt_protocol::elevate::ContainerOperation;

    #[test]
    fn unimplemented_operations_are_refused_not_reported_as_success() {
        let requests = [
            ElevateRequest::CreateNamespace {
                pid: 1,
                tier: IsolationTier::Bare,
            },
            ElevateRequest::MountOverlay {
                lower: "/lower".into(),
                upper: "/upper".into(),
                merged: "/merged".into(),
            },
            ElevateRequest::SetCgroup {
                pid: 1,
                memory_mb: 512,
                cpu_pct: 50,
            },
            ElevateRequest::SetupNetns {
                pid: 1,
                bridge: "br0".into(),
                veth_host: "veth0".into(),
                veth_ns: "veth1".into(),
            },
            ElevateRequest::ApplySeccomp {
                pid: 1,
                policy: vec![0, 1, 2],
            },
            ElevateRequest::CreateRestrictedToken {
                pid: 1,
                tier: IsolationTier::Restricted,
            },
            ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::Create {
                    memory_limit_mb: None,
                    hostname: None,
                },
            },
            ElevateRequest::ApplySeatbelt {
                pid: 1,
                profile: "default".into(),
            },
            ElevateRequest::BindPort {
                port: 8080,
                socket_path: "/tmp/sock".into(),
            },
        ];

        for (index, request) in requests.iter().enumerate() {
            let response = dispatch_request(index as u32, request);
            assert_eq!(response.request_id, index as u32);
            assert_eq!(response.kind, OutcomeKind::Refused, "{request:?}");
            assert!(response.reason.is_some(), "{request:?}");
            assert!(response.payload.is_none(), "{request:?}");
        }
    }

    #[test]
    fn response_carries_request_id() {
        let request = ElevateRequest::BindPort {
            port: 443,
            socket_path: "/s".into(),
        };
        assert_eq!(dispatch_request(42, &request).request_id, 42);
    }
}
