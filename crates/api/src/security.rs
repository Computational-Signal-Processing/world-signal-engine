//! Deployment hardening: authentication, CORS, and request limits.
//!
//! The MVP served an open, permissive API because it ran on a laptop. A VM with
//! a public address is a different situation: an unauthenticated write surface
//! and a wildcard CORS policy are not acceptable defaults there. This module is
//! where the difference is expressed.
//!
//! ## Defaults are safe
//!
//! With no configuration the API is **open on loopback only** — the CLI refuses
//! to bind a non-loopback address without a key, so a public deployment cannot
//! happen by accident. Setting `WSE_API_KEYS` turns authentication on for every
//! route except `/health`.
//!
//! ## Why `/health` stays open
//!
//! A load balancer or an uptime check has to reach `/health` without
//! credentials. It returns counts, not data: no observation values, no signal
//! titles, no source credentials. `/metrics` is **not** public by default,
//! because it exposes operational detail an attacker can use to time requests.

use axum::{
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::sync::Arc;

/// How the API is protected.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Accepted API keys. Empty means "no authentication", which the CLI only
    /// permits on a loopback bind.
    pub api_keys: Vec<String>,
    /// Allow `/metrics` without a key. Off by default.
    pub public_metrics: bool,
    /// Origins allowed to call the API from a browser. Empty means same-origin
    /// only, which is right for the bundled UI.
    pub cors_origins: Vec<String>,
    /// Maximum request body size, in bytes.
    pub max_body_bytes: usize,
    /// Per-request timeout, in seconds.
    pub request_timeout_secs: u64,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            api_keys: Vec::new(),
            public_metrics: false,
            cors_origins: Vec::new(),
            max_body_bytes: 1024 * 1024,
            request_timeout_secs: 30,
        }
    }
}

impl SecurityConfig {
    /// Read the configuration from the environment.
    ///
    /// * `WSE_API_KEYS` — comma-separated keys. `WSE_API_KEY` is accepted as a
    ///   single-key shorthand.
    /// * `WSE_CORS_ORIGINS` — comma-separated origins.
    /// * `WSE_PUBLIC_METRICS` — `1`/`true` to expose `/metrics` unauthenticated.
    /// * `WSE_MAX_BODY_BYTES`, `WSE_REQUEST_TIMEOUT_SECS` — numeric overrides.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(keys) = std::env::var("WSE_API_KEYS") {
            config.api_keys = split_list(&keys);
        }
        if config.api_keys.is_empty() {
            if let Ok(key) = std::env::var("WSE_API_KEY") {
                config.api_keys = split_list(&key);
            }
        }
        if let Ok(origins) = std::env::var("WSE_CORS_ORIGINS") {
            config.cors_origins = split_list(&origins);
        }
        if let Ok(value) = std::env::var("WSE_PUBLIC_METRICS") {
            config.public_metrics = matches!(value.trim(), "1" | "true" | "yes" | "on");
        }
        if let Ok(value) = std::env::var("WSE_MAX_BODY_BYTES") {
            if let Ok(parsed) = value.trim().parse() {
                config.max_body_bytes = parsed;
            }
        }
        if let Ok(value) = std::env::var("WSE_REQUEST_TIMEOUT_SECS") {
            if let Ok(parsed) = value.trim().parse() {
                config.request_timeout_secs = parsed;
            }
        }
        config
    }

    /// Whether any key is configured, i.e. whether authentication is enforced.
    pub fn is_authenticated(&self) -> bool {
        !self.api_keys.is_empty()
    }

    /// Check a presented key against the configured set.
    ///
    /// The comparison is constant-time in the key length. A short-circuiting
    /// `==` leaks, through timing, how many leading bytes of a guess are
    /// correct, which is enough to recover a key one byte at a time.
    pub fn accepts(&self, presented: &str) -> bool {
        let mut matched = false;
        for key in &self.api_keys {
            matched |= constant_time_eq(key.as_bytes(), presented.as_bytes());
        }
        matched
    }
}

fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Compare two byte strings without leaking their contents through timing.
///
/// The length check is unavoidably early, but the *content* comparison always
/// walks the full length of the longer input.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract the presented key from either `Authorization: Bearer` or `X-API-Key`.
fn presented_key(request: &Request) -> Option<String> {
    if let Some(value) = request.headers().get(header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(token) = text
                .strip_prefix("Bearer ")
                .or_else(|| text.strip_prefix("bearer "))
            {
                return Some(token.trim().to_string());
            }
        }
    }
    request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

#[derive(Debug, Serialize)]
struct AuthError {
    error: &'static str,
}

/// Middleware enforcing the API key on every route except the exemptions.
pub async fn require_api_key(
    State(security): State<Arc<SecurityConfig>>,
    request: Request,
    next: Next,
) -> Response {
    if !security.is_authenticated() {
        return next.run(request).await;
    }

    let path = request.uri().path().to_string();
    // Health must stay reachable for load balancers and uptime checks. It
    // reports counts only.
    let exempt = path == "/health" || (security.public_metrics && path == "/metrics");
    if exempt {
        return next.run(request).await;
    }

    match presented_key(&request) {
        Some(key) if security.accepts(&key) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Bearer realm=\"world-signal-engine\"",
            )],
            Json(AuthError {
                error: "missing or invalid API key",
            }),
        )
            .into_response(),
    }
}

/// Build the CORS layer from the configuration.
///
/// With no configured origins this emits no cross-origin headers at all, which
/// is right for the bundled same-origin UI. The previous wildcard policy let any
/// page on the internet read the API through a visitor's browser.
pub fn cors_layer(security: &SecurityConfig) -> tower_http::cors::CorsLayer {
    use tower_http::cors::CorsLayer;
    if security.cors_origins.is_empty() {
        return CorsLayer::new();
    }
    let origins: Vec<header::HeaderValue> = security
        .cors_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            "x-api-key".parse().expect("static header name is valid"),
        ])
        .max_age(std::time::Duration::from_secs(600))
        .vary([header::ORIGIN])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_keys_means_authentication_is_off() {
        let config = SecurityConfig::default();
        assert!(!config.is_authenticated());
        // With no keys configured, nothing is rejected.
        assert!(!config.accepts("anything"));
    }

    #[test]
    fn a_configured_key_is_required_and_matched_exactly() {
        let config = SecurityConfig {
            api_keys: vec!["secret-one".into(), "secret-two".into()],
            ..Default::default()
        };
        assert!(config.is_authenticated());
        assert!(config.accepts("secret-one"));
        assert!(config.accepts("secret-two"));
        assert!(!config.accepts("secret-thre"));
        assert!(!config.accepts(""));
        assert!(!config.accepts("SECRET-ONE"));
    }

    #[test]
    fn constant_time_compare_agrees_with_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn env_parsing_splits_and_trims_lists() {
        assert_eq!(split_list("a, b ,c"), vec!["a", "b", "c"]);
        assert_eq!(split_list("  "), Vec::<String>::new());
        assert_eq!(split_list("one"), vec!["one"]);
    }
}
