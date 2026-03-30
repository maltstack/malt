// Authentication middleware — token validation, capability extraction.

/// Authorization scope levels, ordered from least to most privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthScope {
    Monitor = 0,
    Read = 1,
    Interact = 2,
    Admin = 3,
}

/// Authentication context carried through request processing.
#[derive(Debug, Clone)]
pub struct AuthContext {
    scope: AuthScope,
    client_id: String,
}

impl AuthContext {
    /// Create a new auth context with the given scope and `"local"` as client id.
    pub fn new(scope: AuthScope) -> Self {
        Self {
            scope,
            client_id: "local".to_owned(),
        }
    }

    /// Create a new auth context with explicit client id.
    pub fn with_client(scope: AuthScope, client_id: String) -> Self {
        Self { scope, client_id }
    }

    /// Default context for local (same-machine) connections: full admin.
    pub fn local_default() -> Self {
        Self::new(AuthScope::Admin)
    }

    /// Returns `true` if this context's scope is sufficient for `required`.
    pub fn has_scope(&self, required: AuthScope) -> bool {
        self.scope >= required
    }

    /// The scope level of this context.
    pub fn scope(&self) -> AuthScope {
        self.scope
    }

    /// The client identifier.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
}
