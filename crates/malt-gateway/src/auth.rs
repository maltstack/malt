// Authentication middleware — token validation, capability extraction.

use std::collections::HashMap;
use std::path::Path;
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
        // Entropy failure is not a recoverable condition for a credential.
        // Callers that need to handle it use `try_generate_token`.
        self.try_generate_token(scope)
            .expect("OS entropy unavailable; cannot mint a credential")
    }

    /// Fallible token minting, for paths that must report entropy failure
    /// rather than abort.
    pub fn try_generate_token(&self, scope: AuthScope) -> Result<String, AuthError> {
        let token = generate_random_token()?;
        let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        tokens.insert(token.clone(), scope);
        Ok(token)
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
    pub fn load_or_generate_default(&self) -> Result<String, AuthError> {
        self.load_or_generate_default_at(&dirs_token_path())
    }

    /// As [`TokenStore::load_or_generate_default`], against an explicit path.
    /// Exists so tests can exercise the real persistence logic without
    /// touching the developer's own token file.
    pub fn load_or_generate_default_at(&self, token_path: &Path) -> Result<String, AuthError> {
        if let Ok(token) = std::fs::read_to_string(token_path) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
                tokens.insert(token.clone(), AuthScope::Admin);
                return Ok(token);
            }
        }

        let token = generate_random_token()?;
        write_token_file(token_path, &token)?;
        let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
        tokens.insert(token.clone(), AuthScope::Admin);
        Ok(token)
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a bearer token from OS entropy.
///
/// This previously derived both halves from `SystemTime::now().as_nanos()`
/// and a fixed multiplier, which made a token recomputable by anyone who
/// could approximate daemon start time — the credential was effectively
/// public. It is now 32 bytes of CSPRNG output rendered as hex.
///
/// A failure to obtain OS entropy is fatal rather than silently degraded:
/// there is no weaker fallback that would be honest to hand out as a
/// credential.
/// Errors from credential minting and persistence.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    #[error("OS entropy unavailable: {0}")]
    Entropy(String),
    #[error("could not persist the API token to {path}: {source}")]
    Persist {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Write the token durably and readable only by its owner.
///
/// Errors are returned rather than ignored: a daemon that silently fails to
/// persist its token comes up with a credential no client can ever read,
/// which presents as "auth is broken" long after the cause.
fn write_token_file(path: &Path, token: &str) -> Result<(), AuthError> {
    let persist = |source: std::io::Error| AuthError::Persist {
        path: path.to_path_buf(),
        source,
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(persist)?;
    }

    // Temp + rename, so a reader never observes a half-written token.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token).map_err(persist)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(persist)?;
    }

    std::fs::rename(&tmp, path).map_err(persist)?;
    Ok(())
}

fn generate_random_token() -> Result<String, AuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| AuthError::Entropy(e.to_string()))?;
    let mut token = String::with_capacity(5 + bytes.len() * 2);
    token.push_str("malt_");
    for byte in bytes {
        use std::fmt::Write as _;
        // Writing to a String cannot fail; the Result is discarded knowingly.
        let _ = write!(token, "{byte:02x}");
    }
    Ok(token)
}

/// Where `TokenStore::load_or_generate_default` reads/writes the default
/// API token. Public so first-party clients (`malt-bin`, `malt-mcp`) can
/// read the same file the daemon wrote, rather than duplicating this path
/// logic and risking drift.
pub fn dirs_token_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("malt")
        .join("api-token")
}
