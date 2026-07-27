//! Integration tests for helper authentication and real symlink dispatch.

use malt_elevate::auth::NonceAuth;
// `dispatch` is compiled out off Windows (see lib.rs); the nonce tests below
// are portable and keep running everywhere, so only this import and the one
// test that uses it are gated rather than the whole target.
#[cfg(windows)]
use malt_elevate::dispatch::dispatch_request;
use malt_elevate::protocol::{ElevateRequest, OutcomeKind, ReasonCode};

#[test]
fn nonce_from_file_valid() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("nonce");
    std::fs::write(&path, 12345u64.to_le_bytes()).expect("write nonce");

    let auth = NonceAuth::from_file(&path).expect("read nonce");
    assert!(auth.validate(12345));
    assert!(!auth.validate(0));
}

#[test]
fn nonce_from_file_wrong_length_is_refused() {
    let dir = tempfile::tempdir().expect("temp directory");
    let path = dir.path().join("nonce");
    std::fs::write(&path, b"short").expect("write malformed nonce");

    assert!(NonceAuth::from_file(&path).is_err());
}

#[test]
fn nonce_from_file_missing_is_refused() {
    let dir = tempfile::tempdir().expect("temp directory");
    assert!(NonceAuth::from_file(&dir.path().join("missing")).is_err());
}

#[cfg(windows)]
#[test]
fn symlink_creation_is_only_reported_performed_when_created() {
    let dir = tempfile::tempdir().expect("temp directory");
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&target, b"hello").expect("write target");

    let response = dispatch_request(
        10,
        &ElevateRequest::CreateSymlink {
            target: target.to_string_lossy().into_owned(),
            link: link.to_string_lossy().into_owned(),
        },
    );

    assert_eq!(response.request_id, 10);
    match response.kind {
        OutcomeKind::Performed => {
            assert!(link.is_symlink(), "performed requires the link to exist");
            assert!(response.reason.is_none());
        }
        OutcomeKind::Refused => assert_eq!(response.reason, Some(ReasonCode::OsError)),
        OutcomeKind::Indeterminate | OutcomeKind::Unknown(_) => {
            panic!("symlink operation must be performed or explicitly refused")
        }
        _ => panic!("symlink operation returned an unrecognized outcome"),
    }
}
