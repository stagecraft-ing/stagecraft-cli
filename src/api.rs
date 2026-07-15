//! The control-plane API client (spec 003 §2).
//!
//! `base_url` + a stored credential become an authenticated request path with
//! a uniform error taxonomy (network, auth, api-4xx, api-5xx), retry with
//! jitter for idempotent GETs only, and a `--debug` metadata dump that never
//! prints credential material. The `whoami` verb is the first consumer; every
//! spec-004 verb hangs off the same client. JSON output shapes are API: the
//! MCP face (spec 005) reuses them.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::ResolvedConfig;
use crate::error::{AppError, AppResult};
use crate::output::{self, OutputFormat};

/// Sent on every request so control-plane logs can attribute CLI traffic.
const USER_AGENT: &str = concat!("stagecraft/", env!("CARGO_PKG_VERSION"));
/// Total attempts for an idempotent GET: one initial call plus retries.
const MAX_ATTEMPTS: u32 = 3;
/// Backoff base; the nth retry waits `RETRY_BASE * n` plus jitter.
const RETRY_BASE: Duration = Duration::from_millis(100);
/// The chassis auth identity endpoint (spec 003 §2).
const AUTH_ME_PATH: &str = "/api/v1/auth/me";

/// The failure taxonomy every API call maps onto (spec 003 §2). All variants
/// are operational (exit 1): a failed request is never a usage error.
#[derive(Debug)]
pub enum ApiError {
    /// Transport failure: DNS, connect, TLS, timeout, a dropped connection.
    Network(String),
    /// HTTP 401: no valid credential. Rendered with the "run login" hint.
    Unauthenticated,
    /// A 4xx other than 401, carrying the server's own message when present.
    Api { status: u16, message: String },
    /// A 5xx that survived retries.
    Server { status: u16 },
    /// A 2xx body that was not the JSON shape we expected.
    Decode(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(e) => write!(f, "network error talking to the control plane: {e}"),
            ApiError::Unauthenticated => {
                write!(f, "not authenticated; run `stagecraft login`")
            }
            ApiError::Api { status, message } => {
                write!(f, "control plane returned {status}: {message}")
            }
            ApiError::Server { status } => {
                write!(f, "control plane returned {status} (server error)")
            }
            ApiError::Decode(e) => write!(f, "unexpected response from the control plane: {e}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// Every API failure is an operational failure (exit 1), including 401: an
/// unauthenticated call is a runtime condition the operator resolves by
/// logging in, not a misuse of the command line.
impl From<ApiError> for AppError {
    fn from(err: ApiError) -> Self {
        AppError::Operational(anyhow::anyhow!(err.to_string()))
    }
}

/// The identity subset of the chassis `/auth/me` response we depend on. Extra
/// fields (roles, timestamps) are ignored so the shape can grow server-side.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Identity {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The stable `whoami` JSON shape (API; reused by the MCP face, spec 005).
#[derive(Debug, Serialize)]
pub struct Whoami {
    pub base_url: String,
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A base-URL + credential bound into an authenticated, retrying HTTP client.
pub struct ApiClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
    debug: bool,
}

impl ApiClient {
    /// Build a client for `base_url`, optionally bearing `token`. rustls only
    /// (the crate never links native-tls); the reqwest client is reused across
    /// requests so connection pooling and retries share one pool.
    pub fn new(
        base_url: impl Into<String>,
        token: Option<String>,
        debug: bool,
    ) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(Self {
            base_url: normalize_base_url(&base_url.into()),
            token,
            http,
            debug,
        })
    }

    /// GET the chassis identity endpoint and deserialize the caller identity.
    pub async fn fetch_identity(&self) -> Result<Identity, ApiError> {
        self.get_json(AUTH_ME_PATH).await
    }

    /// Join the base URL and a path into a request URL.
    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// Send a retrying request, then map status + body onto the taxonomy.
    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let (status, body) = self.send_retrying(Method::GET, path).await?;
        if status.as_u16() == 401 {
            return Err(ApiError::Unauthenticated);
        }
        if status.is_client_error() {
            return Err(ApiError::Api {
                status: status.as_u16(),
                message: server_message(&body),
            });
        }
        if status.is_server_error() {
            return Err(ApiError::Server {
                status: status.as_u16(),
            });
        }
        serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// One HTTP attempt: send, optionally emit `--debug` metadata, return the
    /// status and body text. The Authorization header is never logged.
    async fn attempt(&self, method: Method, path: &str) -> Result<(StatusCode, String), ApiError> {
        let url = self.url(path);
        let mut req = self.http.request(method.clone(), url.as_str());
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        let start = std::time::Instant::now();
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let status = resp.status();
        if self.debug {
            eprintln!(
                "[debug] {method} {url} -> {} ({} ms)",
                status.as_u16(),
                start.elapsed().as_millis()
            );
        }
        let body = resp
            .text()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok((status, body))
    }

    /// Retry idempotent GETs on transient failure (network error or 5xx) with
    /// jittered backoff. Non-idempotent methods and 4xx are never retried.
    async fn send_retrying(
        &self,
        method: Method,
        path: &str,
    ) -> Result<(StatusCode, String), ApiError> {
        let retryable = is_retryable_method(&method);
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.attempt(method.clone(), path).await {
                Ok((status, body)) => {
                    if status.is_server_error() && retryable && attempt < MAX_ATTEMPTS {
                        backoff(attempt).await;
                        continue;
                    }
                    return Ok((status, body));
                }
                Err(err) => {
                    if retryable && attempt < MAX_ATTEMPTS {
                        backoff(attempt).await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }
}

/// The `whoami` verb: fetch and render the authenticated identity, or exit 1
/// when unauthenticated (spec 003 §2).
pub fn run_whoami(resolved: &ResolvedConfig, format: OutputFormat, debug: bool) -> AppResult<()> {
    let base_url = require_base_url(resolved)?;
    let token = crate::auth::load_token(&base_url)?.ok_or_else(|| {
        AppError::Operational(anyhow::anyhow!(
            "not authenticated for {base_url}; run `stagecraft login`"
        ))
    })?;
    let client = ApiClient::new(base_url.clone(), Some(token), debug)?;
    let identity = block_on(client.fetch_identity())??;
    let payload = Whoami {
        base_url,
        id: identity.id,
        email: identity.email,
        name: identity.name,
    };
    output::emit(format, &payload, || render_whoami(&payload));
    Ok(())
}

fn render_whoami(w: &Whoami) -> String {
    match &w.name {
        Some(name) => format!("{name} <{}>  (id {}, {})", w.email, w.id, w.base_url),
        None => format!("{}  (id {}, {})", w.email, w.id, w.base_url),
    }
}

/// Resolve the effective control-plane base URL, normalized, or a usage error
/// (exit 2) naming the flag and env var that set it.
pub(crate) fn require_base_url(resolved: &ResolvedConfig) -> AppResult<String> {
    resolved
        .base_url
        .value
        .as_deref()
        .map(normalize_base_url)
        .ok_or_else(|| {
            AppError::Usage(
                "no control-plane base URL; pass --base-url or set STAGECRAFT_BASE_URL".to_string(),
            )
        })
}

/// Drop trailing slashes so a URL is one canonical credentials key and one
/// stable request prefix (`http://x:4000/` and `http://x:4000` are the same).
pub(crate) fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

/// Run an async operation to completion on a fresh multi-thread runtime. The
/// CLI is otherwise synchronous; only the API verbs need async, so the runtime
/// is built per invocation rather than wrapping `main`. A build failure (thread
/// or resource exhaustion) is an operational error (exit 1), never a panic that
/// would escape the exit-code taxonomy.
pub(crate) fn block_on<F: std::future::Future>(future: F) -> AppResult<F::Output> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            AppError::Operational(anyhow::anyhow!("failed to start async runtime: {e}"))
        })?;
    Ok(runtime.block_on(future))
}

/// Only idempotent methods are safe to retry (spec 003 §2: GETs only).
fn is_retryable_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
}

/// Jittered backoff before the nth retry: `RETRY_BASE * n` plus 0-49ms of
/// jitter derived from the wall clock (no `rand` dependency).
async fn backoff(attempt: u32) {
    let jitter = Duration::from_millis(jitter_ms());
    tokio::time::sleep(RETRY_BASE * attempt + jitter).await;
}

fn jitter_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos() % 50))
        .unwrap_or(0)
}

/// Extract a human-facing message from an error body: the chassis `message`
/// field when present, else the trimmed body, else a placeholder.
fn server_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct ErrBody {
        message: Option<String>,
    }
    if let Ok(ErrBody {
        message: Some(message),
    }) = serde_json::from_str::<ErrBody>(body)
    {
        return message;
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "(no message)".to_string()
    } else {
        trimmed.chars().take(200).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    #[test]
    fn normalize_trims_trailing_slashes() {
        assert_eq!(normalize_base_url("http://x:4000/"), "http://x:4000");
        assert_eq!(normalize_base_url("http://x:4000"), "http://x:4000");
        assert_eq!(normalize_base_url("http://x:4000///"), "http://x:4000");
    }

    #[test]
    fn only_idempotent_methods_retry() {
        assert!(is_retryable_method(&Method::GET));
        assert!(is_retryable_method(&Method::HEAD));
        assert!(!is_retryable_method(&Method::POST));
        assert!(!is_retryable_method(&Method::DELETE));
    }

    #[test]
    fn server_message_prefers_json_message_field() {
        assert_eq!(server_message(r#"{"code":"x","message":"boom"}"#), "boom");
        assert_eq!(server_message("plain text"), "plain text");
        assert_eq!(server_message("   "), "(no message)");
    }

    #[test]
    fn unauthenticated_message_hints_login() {
        assert!(ApiError::Unauthenticated
            .to_string()
            .contains("stagecraft login"));
    }

    #[test]
    fn api_errors_map_to_operational_exit_code() {
        let err: AppError = ApiError::Unauthenticated.into();
        assert_eq!(err.code(), crate::error::EXIT_OPERATIONAL);
    }

    #[test]
    fn whoami_json_shape_is_stable() {
        // The emitted envelope is API the MCP face reuses (spec 003 §2).
        let payload = Whoami {
            base_url: "http://localhost:4000".to_string(),
            id: "u_1".to_string(),
            email: "dev@example.com".to_string(),
            name: Some("Dev".to_string()),
        };
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["base_url"], "http://localhost:4000");
        assert_eq!(v["id"], "u_1");
        assert_eq!(v["email"], "dev@example.com");
        assert_eq!(v["name"], "Dev");

        // An absent name is omitted, never serialized as null.
        let anon = Whoami {
            base_url: "b".to_string(),
            id: "i".to_string(),
            email: "e".to_string(),
            name: None,
        };
        let v = serde_json::to_value(&anon).unwrap();
        assert!(v.get("name").is_none(), "name must be omitted when None");
    }

    #[test]
    fn whoami_happy_path_parses_identity() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path(AUTH_ME_PATH)
                .header("authorization", "Bearer good-token");
            then.status(200).json_body(json!({
                "id": "u_1",
                "email": "dev@example.com",
                "name": "Dev",
                "roles": ["admin"]
            }));
        });

        let client = ApiClient::new(server.base_url(), Some("good-token".into()), false).unwrap();
        let identity = block_on(client.fetch_identity()).unwrap().unwrap();

        mock.assert();
        assert_eq!(identity.id, "u_1");
        assert_eq!(identity.email, "dev@example.com");
        assert_eq!(identity.name.as_deref(), Some("Dev"));
    }

    #[test]
    fn whoami_401_maps_to_unauthenticated() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path(AUTH_ME_PATH);
            then.status(401).json_body(json!({
                "code": "unauthenticated",
                "message": "missing authentication credentials"
            }));
        });

        let client = ApiClient::new(server.base_url(), Some("bad".into()), false).unwrap();
        let err = block_on(client.fetch_identity()).unwrap().unwrap_err();
        assert!(matches!(err, ApiError::Unauthenticated));
    }

    #[test]
    fn whoami_4xx_carries_server_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path(AUTH_ME_PATH);
            then.status(403)
                .json_body(json!({ "code": "forbidden", "message": "tenant suspended" }));
        });

        let client = ApiClient::new(server.base_url(), Some("t".into()), false).unwrap();
        let err = block_on(client.fetch_identity()).unwrap().unwrap_err();
        match err {
            ApiError::Api { status, message } => {
                assert_eq!(status, 403);
                assert_eq!(message, "tenant suspended");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn get_retries_5xx_up_to_max_attempts() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/boom");
            then.status(503);
        });

        let client = ApiClient::new(server.base_url(), None, false).unwrap();
        let (status, _) = block_on(client.send_retrying(Method::GET, "/boom"))
            .unwrap()
            .unwrap();

        assert_eq!(status.as_u16(), 503);
        assert_eq!(mock.hits(), MAX_ATTEMPTS as usize);
    }

    #[test]
    fn non_idempotent_request_is_not_retried() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/boom");
            then.status(503);
        });

        let client = ApiClient::new(server.base_url(), None, false).unwrap();
        let (status, _) = block_on(client.send_retrying(Method::POST, "/boom"))
            .unwrap()
            .unwrap();

        assert_eq!(status.as_u16(), 503);
        assert_eq!(mock.hits(), 1);
    }
}
