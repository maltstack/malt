// Auth + rate-limit enforcement — wires TokenStore/AuthContext/RateLimiter
// (built and unit-tested in isolation, but never previously attached to
// any router) into real axum middleware.
//
// Must be applied as the *last* step, after every route (including any
// added outside `build_router`, e.g. malt-bin's `/shutdown`) has been
// registered — axum's `.layer()` only wraps routes that existed at the
// point it was called, not ones added afterward. See `with_auth`.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::MatchedPath;
use axum::http::{Method, Request};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;

use crate::auth::{AuthContext, AuthScope, TokenStore};
use crate::error::GatewayError;
use crate::rate_limit::RateLimiter;

/// Required scope per (method, route template) pair, matching
/// `docs/design/architecture.md`'s scope table. Route templates are
/// axum's `MatchedPath` form (`{id}`, not a real path segment).
/// Unrecognized (method, path) pairs fail closed to `Admin` — a route
/// this table doesn't know about should never default to open access.
fn required_scope(method: &Method, path: &str) -> AuthScope {
    match (method, path) {
        (&Method::GET, "/health") => AuthScope::Monitor,
        (&Method::GET, "/sessions") => AuthScope::Monitor,
        (&Method::POST, "/sessions") => AuthScope::Interact,
        (&Method::GET, "/sessions/{id}") => AuthScope::Read,
        (&Method::DELETE, "/sessions/{id}") => AuthScope::Admin,
        (&Method::POST, "/sessions/{id}/exec") => AuthScope::Interact,
        (&Method::POST, "/sessions/{id}/send") => AuthScope::Interact,
        (&Method::GET, "/sessions/{id}/output") => AuthScope::Read,
        (&Method::GET, "/sessions/{id}/output/text") => AuthScope::Read,
        // Same sensitivity class as output: command text can contain paths,
        // arguments, and secrets typed at the prompt. Not Monitor, which is
        // for liveness/inventory only.
        (&Method::GET, "/sessions/{id}/history") => AuthScope::Read,
        (&Method::GET, "/sessions/{id}/panes") => AuthScope::Read,
        (&Method::POST, "/sessions/{id}/panes/split") => AuthScope::Interact,
        (&Method::DELETE, "/sessions/{id}/panes/{pane_id}") => AuthScope::Interact,
        (&Method::POST, "/shutdown") => AuthScope::Admin,
        _ => AuthScope::Admin,
    }
}

async fn auth_middleware(
    token_store: Arc<TokenStore>,
    rate_limiter: Arc<RateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let matched_path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string());

    let Some(path) = matched_path else {
        // No route matched this request at all -- shouldn't normally
        // reach here since the layer wraps an already-routed request,
        // but fail closed rather than assume an open default.
        return GatewayError::Unauthorized("no matching route".to_string()).into_response();
    };
    let required = required_scope(&method, &path);

    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let Some(token) = token else {
        return GatewayError::Unauthorized("missing bearer token".to_string()).into_response();
    };

    let Some(scope) = token_store.validate(token) else {
        return GatewayError::Unauthorized("invalid or expired token".to_string()).into_response();
    };
    let ctx = AuthContext::with_client(scope, token.to_string());

    if !rate_limiter.check(ctx.client_id()) {
        return GatewayError::RateLimited.into_response();
    }

    if !ctx.has_scope(required) {
        return GatewayError::Forbidden {
            required: format!("{required:?}"),
        }
        .into_response();
    }

    next.run(req).await
}

/// Attach real auth + rate-limit enforcement to a fully-assembled router.
///
/// Must be the last call in router construction — see the module doc
/// comment for why. `malt-bin`'s ad hoc `/shutdown` route must be added
/// to `router` *before* calling this, or it will be served unauthenticated.
pub fn with_auth(
    router: Router,
    token_store: Arc<TokenStore>,
    rate_limiter: Arc<RateLimiter>,
) -> Router {
    router.layer(middleware::from_fn(move |req: Request<Body>, next: Next| {
        let token_store = token_store.clone();
        let rate_limiter = rate_limiter.clone();
        async move { auth_middleware(token_store, rate_limiter, req, next).await }
    }))
}
