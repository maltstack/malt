//! Request dispatch — routes `ElevateRequest` variants to handlers.
//!
//! Phase 2: most operations are stubs returning success. Real implementations
//! arrive with `malt-platform` isolation tier support.

/// Identifies a privileged operation requested by the daemon.
///
/// These variants mirror the `ElevateRequest` union from `elevate.vexil`.
/// We define them here as plain Rust types so the dispatch logic is testable
/// independently of the vexilc codegen pipeline.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ElevateRequest {
    // Linux
    CreateNamespace {
        pid: u32,
        tier: u8,
    },
    MountOverlay {
        lower: String,
        upper: String,
        merged: String,
    },
    SetCgroup {
        pid: u32,
        memory_mb: u32,
        cpu_pct: u16,
    },
    SetupNetns {
        pid: u32,
        bridge: String,
        veth_host: String,
        veth_ns: String,
    },
    ApplySeccomp {
        pid: u32,
        policy: Vec<u8>,
    },
    // Windows
    CreateSymlink {
        target: String,
        link: String,
    },
    CreateRestrictedToken {
        pid: u32,
        tier: u8,
    },
    ManageHcsContainer {
        operation: String,
        config: Vec<u8>,
    },
    // macOS
    ApplySeatbelt {
        pid: u32,
        profile: String,
    },
    // Cross-platform
    BindPort {
        port: u16,
        socket_path: String,
    },
}

/// Result of dispatching an `ElevateRequest`.
#[derive(Debug, Clone, PartialEq)]
pub struct ElevateResponse {
    /// Correlation ID from the request envelope.
    pub request_id: u32,
    /// Operation result: `Ok(payload)` on success, `Err(message)` on failure.
    pub result: Result<Vec<u8>, String>,
}

/// Dispatch a request to the appropriate handler.
///
/// Most operations are stubs in Phase 2 — they return success without
/// performing any actual privileged work. `CreateSymlink` is implemented
/// on Windows since symlink creation requires elevation.
pub fn dispatch_request(request_id: u32, request: &ElevateRequest) -> ElevateResponse {
    let result = match request {
        ElevateRequest::CreateNamespace { .. } => stub_success("CreateNamespace"),
        ElevateRequest::MountOverlay { .. } => stub_success("MountOverlay"),
        ElevateRequest::SetCgroup { .. } => stub_success("SetCgroup"),
        ElevateRequest::SetupNetns { .. } => stub_success("SetupNetns"),
        ElevateRequest::ApplySeccomp { .. } => stub_success("ApplySeccomp"),
        ElevateRequest::CreateSymlink { target, link } => dispatch_create_symlink(target, link),
        ElevateRequest::CreateRestrictedToken { .. } => stub_success("CreateRestrictedToken"),
        ElevateRequest::ManageHcsContainer { .. } => stub_success("ManageHcsContainer"),
        ElevateRequest::ApplySeatbelt { .. } => stub_success("ApplySeatbelt"),
        ElevateRequest::BindPort { .. } => stub_success("BindPort"),
    };

    ElevateResponse { request_id, result }
}

/// Stub handler: logs the operation and returns success with an empty payload.
fn stub_success(op: &str) -> Result<Vec<u8>, String> {
    tracing::info!(
        operation = op,
        "stub: operation not yet implemented, returning success"
    );
    Ok(Vec::new())
}

/// Create a symlink. On Windows this requires elevation; on Unix it does not
/// but we handle it here for consistency.
fn dispatch_create_symlink(target: &str, link: &str) -> Result<Vec<u8>, String> {
    tracing::info!(target = target, link = link, "creating symlink");

    #[cfg(windows)]
    {
        create_symlink_windows(target, link)
    }

    #[cfg(unix)]
    {
        create_symlink_unix(target, link)
    }
}

#[cfg(windows)]
fn create_symlink_windows(target: &str, link: &str) -> Result<Vec<u8>, String> {
    use std::path::Path;

    let target_path = Path::new(target);

    // Try file symlink first, fall back to directory symlink.
    // On Windows, symlink type must match the target type.
    let result = if target_path.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };

    match result {
        Ok(()) => Ok(Vec::new()),
        Err(e) => Err(format!("symlink creation failed: {e}")),
    }
}

#[cfg(unix)]
fn create_symlink_unix(target: &str, link: &str) -> Result<Vec<u8>, String> {
    match std::os::unix::fs::symlink(target, link) {
        Ok(()) => Ok(Vec::new()),
        Err(e) => Err(format!("symlink creation failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_operations_return_success() {
        let stubs = vec![
            ElevateRequest::CreateNamespace { pid: 1, tier: 0 },
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
            ElevateRequest::CreateRestrictedToken { pid: 1, tier: 1 },
            ElevateRequest::ManageHcsContainer {
                operation: "create".into(),
                config: vec![],
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

        for (i, req) in stubs.iter().enumerate() {
            let resp = dispatch_request(i as u32, req);
            assert_eq!(resp.request_id, i as u32);
            assert!(
                resp.result.is_ok(),
                "expected success for {:?}, got {:?}",
                req,
                resp.result
            );
        }
    }

    #[test]
    fn response_carries_request_id() {
        let req = ElevateRequest::BindPort {
            port: 443,
            socket_path: "/s".into(),
        };
        let resp = dispatch_request(42, &req);
        assert_eq!(resp.request_id, 42);
    }
}
