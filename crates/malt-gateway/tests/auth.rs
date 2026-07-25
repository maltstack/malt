use malt_gateway::auth::{AuthContext, AuthScope, TokenStore};

#[test]
fn scope_hierarchy_admin_includes_all() {
    let ctx = AuthContext::new(AuthScope::Admin);
    assert!(ctx.has_scope(AuthScope::Monitor));
    assert!(ctx.has_scope(AuthScope::Read));
    assert!(ctx.has_scope(AuthScope::Interact));
    assert!(ctx.has_scope(AuthScope::Admin));
}

#[test]
fn scope_hierarchy_read_excludes_interact() {
    let ctx = AuthContext::new(AuthScope::Read);
    assert!(ctx.has_scope(AuthScope::Monitor));
    assert!(ctx.has_scope(AuthScope::Read));
    assert!(!ctx.has_scope(AuthScope::Interact));
    assert!(!ctx.has_scope(AuthScope::Admin));
}

#[test]
fn scope_hierarchy_monitor_minimal() {
    let ctx = AuthContext::new(AuthScope::Monitor);
    assert!(ctx.has_scope(AuthScope::Monitor));
    assert!(!ctx.has_scope(AuthScope::Read));
    assert!(!ctx.has_scope(AuthScope::Interact));
    assert!(!ctx.has_scope(AuthScope::Admin));
}

#[test]
fn default_local_scope_is_admin() {
    let ctx = AuthContext::local_default();
    assert_eq!(ctx.scope(), AuthScope::Admin);
    assert_eq!(ctx.client_id(), "local");
}

#[test]
fn token_generate_and_validate() {
    let store = TokenStore::new();
    let token = store.generate_token(AuthScope::Admin);
    assert!(store.validate(&token).is_some());
    assert_eq!(store.validate(&token), Some(AuthScope::Admin));
}

#[test]
fn token_revoke() {
    let store = TokenStore::new();
    let token = store.generate_token(AuthScope::Read);
    store.revoke(&token);
    assert!(store.validate(&token).is_none());
}

#[test]
fn invalid_token_rejected() {
    let store = TokenStore::new();
    assert!(store.validate("fake_token").is_none());
}

#[test]
fn token_scope_preserved() {
    let store = TokenStore::new();
    let read_token = store.generate_token(AuthScope::Read);
    let admin_token = store.generate_token(AuthScope::Admin);
    assert_eq!(store.validate(&read_token), Some(AuthScope::Read));
    assert_eq!(store.validate(&admin_token), Some(AuthScope::Admin));
}

// --- Credential quality and persistence (audit A-03) ---------------------
//
// The tests above all pass against the epoch-derived generator this
// replaced: they only ask that a token round-trips. These ask whether the
// credential is actually unguessable and actually stored.

#[test]
fn two_tokens_generated_in_the_same_process_differ() {
    let store = TokenStore::new();
    let a = store.generate_token(AuthScope::Admin);
    let b = store.generate_token(AuthScope::Admin);
    assert_ne!(a, b, "each minted credential must be distinct");
    assert!(a.starts_with("malt_") && b.starts_with("malt_"));
}

#[test]
fn a_token_is_not_derivable_from_the_clock() {
    // Regression pin for A-03. The old generator was:
    //
    //   seed = SystemTime::now().as_nanos()
    //   format!("malt_{:016x}{:016x}", seed, seed.wrapping_mul(6364136223846793005))
    //
    // so anyone who could approximate daemon start time could recompute the
    // credential. Reconstruct that derivation across the window in which the
    // real token was minted and assert it matches none of the candidates.
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    };

    let before = nanos();
    let token = TokenStore::new().generate_token(AuthScope::Admin);
    let after = nanos();

    // A guesser would not need the exact nanosecond, only a nearby one, so
    // sample the whole window rather than a single point.
    let span = after.saturating_sub(before).max(1);
    let step = (span / 5_000).max(1);
    let mut candidate = before;
    while candidate <= after {
        let guess = format!(
            "malt_{:016x}{:016x}",
            candidate,
            candidate.wrapping_mul(6_364_136_223_846_793_005)
        );
        assert_ne!(
            token, guess,
            "the token was reproducible from the clock -- the CSPRNG fix regressed"
        );
        candidate = candidate.saturating_add(step);
    }
}

#[test]
fn the_default_token_is_persisted_and_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("api-token");

    let token = TokenStore::new()
        .load_or_generate_default_at(&path)
        .expect("minting and persisting should succeed");
    assert!(path.exists(), "the token file must actually be written");

    // A fresh store over the same path must adopt the same credential, or a
    // daemon restart would invalidate every client.
    let second = TokenStore::new();
    let reloaded = second.load_or_generate_default_at(&path).unwrap();
    assert_eq!(token, reloaded);
    assert_eq!(second.validate(&reloaded), Some(AuthScope::Admin));
}

#[test]
fn a_persistence_failure_is_reported_rather_than_swallowed() {
    // Use an existing *file* as the parent directory, so creation must fail.
    // The old code ignored both the create_dir_all and the write results, so
    // the daemon could come up holding a credential no client could read and
    // say nothing about it.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"i am a file").unwrap();

    let result = TokenStore::new().load_or_generate_default_at(&blocker.join("api-token"));
    assert!(
        result.is_err(),
        "a token that could not be persisted must surface as an error, not a \
         success the daemon then acts on"
    );
}

#[test]
fn a_persisted_token_leaves_no_temp_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api-token");
    TokenStore::new()
        .load_or_generate_default_at(&path)
        .unwrap();

    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name != "api-token")
        .collect();
    assert!(
        leftovers.is_empty(),
        "the atomic write must rename its temp file, not leave it: {leftovers:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_token_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api-token");
    TokenStore::new()
        .load_or_generate_default_at(&path)
        .unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o077,
        0,
        "a credential readable by group or other is a credential leak"
    );
}
