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
