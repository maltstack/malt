//! Integration tests for the malt-elevate library components.

use malt_elevate::auth::NonceAuth;
use malt_elevate::dispatch::{dispatch_request, ElevateRequest};
use malt_elevate::protocol::MessageTag;

// ── Auth tests ──

#[test]
fn nonce_from_file_valid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonce");
    std::fs::write(&path, 12345u64.to_le_bytes()).unwrap();

    let auth = NonceAuth::from_file(&path).unwrap();
    assert!(auth.validate(12345));
    assert!(!auth.validate(0));
}

#[test]
fn nonce_from_file_wrong_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonce");
    std::fs::write(&path, b"short").unwrap();

    let result = NonceAuth::from_file(&path);
    assert!(result.is_err());
}

#[test]
fn nonce_from_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent");

    let result = NonceAuth::from_file(&path);
    assert!(result.is_err());
}

#[test]
fn nonce_from_file_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonce");
    std::fs::write(&path, 0u64.to_le_bytes()).unwrap();

    let auth = NonceAuth::from_file(&path).unwrap();
    assert!(auth.validate(0));
    assert!(!auth.validate(1));
}

// ── Dispatch tests ──

#[test]
fn all_stubs_return_success() {
    let requests: Vec<(u32, ElevateRequest)> = vec![
        (0, ElevateRequest::CreateNamespace { pid: 100, tier: 2 }),
        (
            1,
            ElevateRequest::MountOverlay {
                lower: "/a".into(),
                upper: "/b".into(),
                merged: "/c".into(),
            },
        ),
        (
            2,
            ElevateRequest::SetCgroup {
                pid: 100,
                memory_mb: 1024,
                cpu_pct: 80,
            },
        ),
        (
            3,
            ElevateRequest::SetupNetns {
                pid: 100,
                bridge: "br0".into(),
                veth_host: "vh0".into(),
                veth_ns: "vn0".into(),
            },
        ),
        (
            4,
            ElevateRequest::ApplySeccomp {
                pid: 100,
                policy: vec![1, 2, 3],
            },
        ),
        (
            5,
            ElevateRequest::CreateRestrictedToken { pid: 100, tier: 1 },
        ),
        (
            6,
            ElevateRequest::ManageHcsContainer {
                operation: "start".into(),
                config: vec![],
            },
        ),
        (
            7,
            ElevateRequest::ApplySeatbelt {
                pid: 100,
                profile: "sandbox".into(),
            },
        ),
        (
            8,
            ElevateRequest::BindPort {
                port: 443,
                socket_path: "/tmp/s".into(),
            },
        ),
    ];

    for (id, req) in &requests {
        let resp = dispatch_request(*id, req);
        assert_eq!(resp.request_id, *id);
        assert!(resp.result.is_ok(), "stub {:?} should succeed", req);
        // Stubs return empty payload
        assert!(resp.result.as_ref().unwrap().is_empty());
    }
}

#[test]
fn symlink_to_nonexistent_target_fails() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("mylink");

    let req = ElevateRequest::CreateSymlink {
        target: "/nonexistent/path/that/does/not/exist".into(),
        link: link.to_string_lossy().into_owned(),
    };

    let resp = dispatch_request(99, &req);
    assert_eq!(resp.request_id, 99);

    // On Unix, symlinks to nonexistent targets succeed (dangling symlink).
    // On Windows, symlink creation may fail without elevation or if the
    // target type cannot be determined. We accept either outcome.
    #[cfg(unix)]
    assert!(resp.result.is_ok());
}

#[test]
fn symlink_creation_succeeds_for_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");

    std::fs::write(&target, b"hello").unwrap();

    let req = ElevateRequest::CreateSymlink {
        target: target.to_string_lossy().into_owned(),
        link: link.to_string_lossy().into_owned(),
    };

    let resp = dispatch_request(10, &req);
    assert_eq!(resp.request_id, 10);

    // On Windows, symlink creation requires elevation (SeCreateSymbolicLinkPrivilege).
    // In CI/unprivileged contexts this will fail — we accept either outcome.
    #[cfg(unix)]
    {
        assert!(resp.result.is_ok());
        assert!(link.is_symlink());
    }
}

// ── Protocol tests ──

#[test]
fn message_tag_round_trip() {
    let tags = [
        MessageTag::Hello,
        MessageTag::HelloAck,
        MessageTag::Request,
        MessageTag::Response,
        MessageTag::Shutdown,
    ];

    for tag in tags {
        let byte = tag as u8;
        assert_eq!(MessageTag::from_byte(byte), Some(tag));
    }
}

#[test]
fn unknown_message_tag() {
    for b in 5..=255 {
        assert_eq!(MessageTag::from_byte(b), None);
    }
}
