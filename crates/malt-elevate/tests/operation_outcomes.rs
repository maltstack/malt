//! Outcome and capability cross-checks over every schema-generated operation.
//! Windows-only: this exercises `capability` and `dispatch`, which are compiled out
//! off Windows because the privileged helper binds to Windows service,
//! named-pipe and HCS primitives (see `lib.rs`). Gating the test target
//! keeps it running where the helper exists rather than deleting cover.
#![cfg(windows)]

use malt_elevate::capability::capabilities;
use malt_elevate::dispatch::dispatch_request;
use malt_elevate::protocol::{ContainerOperation, ElevateRequest, OutcomeKind, ReasonCode};
use malt_protocol::common::IsolationTier;

/// Build one concrete request per generated schema variant.
///
/// `scratch` must be a directory the test owns. **CreateSymlink is the only
/// operation here that is actually implemented**, so unlike the stubs it does
/// real filesystem work, and it does not care that its target does not exist.
///
/// These paths used to be `/not-a-real-target` and `/not-a-real-link`. On
/// Windows a leading slash resolves to the root of the current drive, so every
/// run left a real link at the drive root -- twice, since both tests below
/// dispatch the whole set, and nothing ever removed it. Naming a path "not
/// real" does not make it so.
///
/// Every filesystem-ish parameter here points into `scratch`, not just
/// CreateSymlink's. MountOverlay's mount points and BindPort's socket path are
/// inert *only while those operations are stubs*; whoever implements them
/// would otherwise recreate this bug at the drive root, and the test would
/// still pass while doing it.
fn schema_requests(scratch: &std::path::Path) -> Vec<(&'static str, ElevateRequest)> {
    // Each value is a concrete variant of the generated schema type. Keeping
    // the constructors here means a schema variant shape change breaks this
    // test at compile time rather than silently dropping coverage.
    vec![
        (
            "CreateNamespace",
            ElevateRequest::CreateNamespace {
                pid: 1,
                tier: IsolationTier::Bare,
            },
        ),
        (
            "MountOverlay",
            ElevateRequest::MountOverlay {
                lower: scratch.join("lower").to_string_lossy().into_owned(),
                upper: scratch.join("upper").to_string_lossy().into_owned(),
                merged: scratch.join("merged").to_string_lossy().into_owned(),
            },
        ),
        (
            "SetCgroup",
            ElevateRequest::SetCgroup {
                pid: 1,
                memory_mb: 1,
                cpu_pct: 1,
            },
        ),
        (
            "SetupNetns",
            ElevateRequest::SetupNetns {
                pid: 1,
                bridge: "br0".into(),
                veth_host: "veth-host".into(),
                veth_ns: "veth-ns".into(),
            },
        ),
        (
            "ApplySeccomp",
            ElevateRequest::ApplySeccomp {
                pid: 1,
                policy: Vec::new(),
            },
        ),
        (
            "CreateSymlink",
            ElevateRequest::CreateSymlink {
                target: scratch
                    .join("symlink-target")
                    .to_string_lossy()
                    .into_owned(),
                link: scratch.join("symlink-link").to_string_lossy().into_owned(),
            },
        ),
        (
            "CreateRestrictedToken",
            ElevateRequest::CreateRestrictedToken {
                pid: 1,
                tier: IsolationTier::Restricted,
            },
        ),
        (
            "ManageHcsContainer",
            ElevateRequest::ManageHcsContainer {
                operation: ContainerOperation::Create {
                    memory_limit_mb: None,
                    hostname: None,
                    image_id: None,
                },
            },
        ),
        (
            "ApplySeatbelt",
            ElevateRequest::ApplySeatbelt {
                pid: 1,
                profile: "default".into(),
            },
        ),
        (
            "BindPort",
            ElevateRequest::BindPort {
                port: 8080,
                socket_path: scratch.join("helper.sock").to_string_lossy().into_owned(),
            },
        ),
    ]
}

#[test]
fn every_generated_operation_has_an_explicit_outcome() {
    let scratch = tempfile::tempdir().expect("scratch dir for filesystem operations");
    for (request_id, (operation, request)) in schema_requests(scratch.path()).iter().enumerate() {
        let response = dispatch_request(request_id as u32, request);
        assert_eq!(response.request_id, request_id as u32);
        assert!(
            matches!(response.kind, OutcomeKind::Refused | OutcomeKind::Performed),
            "{operation} returned an ambiguous outcome: {response:?}"
        );
        if response.kind == OutcomeKind::Performed {
            assert!(
                response.reason.is_none(),
                "{operation}: performed with a reason"
            );
        } else {
            assert!(
                response.reason.is_some(),
                "{operation}: refusal needs a reason"
            );
            assert!(
                response.payload.is_none(),
                "{operation}: refusal cannot carry success payload"
            );
        }
    }
}

#[test]
fn capability_surface_agrees_with_dispatch_reality() {
    let capabilities = capabilities();
    let scratch = tempfile::tempdir().expect("scratch dir for filesystem operations");
    let requests = schema_requests(scratch.path());
    assert_eq!(capabilities.operations.len(), requests.len());

    for ((operation, request), capability) in requests.iter().zip(&capabilities.operations) {
        assert_eq!(capability.operation, *operation);
        let response = dispatch_request(1, request);
        if capability.available {
            assert!(
                response.reason != Some(ReasonCode::NotImplemented)
                    && response.reason != Some(ReasonCode::UnsupportedPlatform),
                "{operation} is advertised available but dispatch says {response:?}"
            );
        } else {
            assert_eq!(
                response.kind,
                OutcomeKind::Refused,
                "{operation}: {response:?}"
            );
            assert_eq!(
                response.reason, capability.reason,
                "{operation}: {response:?}"
            );
        }
    }
}
