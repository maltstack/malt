//! Outcome and capability cross-checks over every schema-generated operation.

use malt_elevate::capability::capabilities;
use malt_elevate::dispatch::dispatch_request;
use malt_elevate::protocol::{ContainerOperation, ElevateRequest, OutcomeKind, ReasonCode};
use malt_protocol::common::IsolationTier;

fn schema_requests() -> Vec<(&'static str, ElevateRequest)> {
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
                lower: "/lower".into(),
                upper: "/upper".into(),
                merged: "/merged".into(),
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
                target: "/not-a-real-target".into(),
                link: "/not-a-real-link".into(),
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
                socket_path: "/tmp/helper.sock".into(),
            },
        ),
    ]
}

#[test]
fn every_generated_operation_has_an_explicit_outcome() {
    for (request_id, (operation, request)) in schema_requests().iter().enumerate() {
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
    let requests = schema_requests();
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
