// Authentication middleware — token validation, capability extraction.

use std::collections::HashMap;
use std::sync::Mutex;

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

/// Token store for bearer token authentication.
pub struct TokenStore {
    tokens: Mutex<HashMap<String, AuthScope>>,
}

impl TokenStore {
    /// Create a new empty token store.
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Generate a new token with the given scope.
    pub fn generate_token(&self, scope: AuthScope) -> String {
        let token = generate_random_token();
        let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        tokens.insert(token.clone(), scope);
        token
    }

    /// Validate a bearer token and return the associated scope.
    pub fn validate(&self, token: &str) -> Option<AuthScope> {
        let tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        tokens.get(token).copied()
    }

    /// Revoke a token.
    pub fn revoke(&self, token: &str) {
        let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        tokens.remove(token);
    }

    /// Load a token from file (e.g. `~/.config/malt/api-token`) or generate one.
    ///
    /// If the token file exists and is non-empty, the token is loaded and
    /// registered with `Admin` scope. Otherwise a new `Admin` token is
    /// generated and saved to the file.
    pub fn load_or_generate_default(&self) -> String {
        let token_path = dirs_token_path();
        if let Ok(token) = std::fs::read_to_string(&token_path) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
                tokens.insert(token.clone(), AuthScope::Admin);
                return token;
            }
        }
        // Generate and save
        let token = self.generate_token(AuthScope::Admin);
        if let Some(parent) = token_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&token_path, &token);
        token
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

fn generate_random_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "malt_{:016x}{:016x}",
        seed,
        seed.wrapping_mul(6_364_136_223_846_793_005)
    )
}

fn dirs_token_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("malt")
        .join("api-token")
}
