use malt_gateway::auth::{AuthContext, AuthScope};

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
