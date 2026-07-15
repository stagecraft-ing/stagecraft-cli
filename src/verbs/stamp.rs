//! The `stamp` verb family (spec 004 §5.1): new, status (with `--watch`).

use std::fmt::Write;
use std::time::Duration;

use serde_json::{Map, Value};

use super::{client_for, emit_err, emit_ok, field, render, require_field};
use crate::api::{block_on, ApiClient, ApiError};
use crate::cli::Posture;
use crate::config::ResolvedConfig;
use crate::error::{AppError, AppResult, EXIT_OPERATIONAL};
use crate::output::OutputFormat;

/// First poll interval for `--watch`; it backs off toward [`WATCH_MAX`].
const WATCH_INITIAL: Duration = Duration::from_secs(2);
/// The poll-interval ceiling for `--watch` (spec 004 §5.4).
const WATCH_MAX: Duration = Duration::from_secs(10);

/// `stamp new` -> POST /api/v1/tenants/:id/stamps. `posture` is always sent
/// (the flag is required); `frontend` only when supplied.
#[allow(clippy::too_many_arguments)]
pub fn new(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    tenant: &str,
    app: &str,
    org: &str,
    frontend: Option<&str>,
    posture: Posture,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let path = format!("/api/v1/tenants/{tenant}/stamps");

    let mut body = Map::new();
    body.insert("appName".to_string(), Value::String(app.to_string()));
    body.insert("targetOrg".to_string(), Value::String(org.to_string()));
    body.insert(
        "posture".to_string(),
        Value::String(posture.as_wire().to_string()),
    );
    if let Some(frontend) = frontend {
        body.insert("frontend".to_string(), Value::String(frontend.to_string()));
    }

    let result = block_on(client.post_value(&path, Value::Object(body)))?;
    render(format, result, render_stamp)
}

/// `stamp status <jobId> [--watch]` -> GET /api/v1/stamps/:jobId.
pub fn status(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    job_id: &str,
    watch: bool,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let path = format!("/api/v1/stamps/{job_id}");
    if watch {
        block_on(watch_loop(&client, &path, format))?
    } else {
        let result = block_on(client.get_value(&path))?;
        render(format, result, render_stamp)
    }
}

/// A stamp job's terminal states (spec 005: queued|stamping|pushing|verifying
/// are transient; green|failed are terminal).
enum Terminal {
    Green,
    Failed,
}

/// Classify a job status: `Some` at a terminal state, `None` while it is still
/// running (keep polling).
fn classify(status: &str) -> Option<Terminal> {
    match status {
        "green" => Some(Terminal::Green),
        "failed" => Some(Terminal::Failed),
        _ => None,
    }
}

/// Poll the status endpoint until the job settles, emitting each *changed*
/// state (a line for humans, an envelope for JSON) and backing the interval off
/// toward the cap. Green exits 0; failed returns [`AppError::Rendered`] (exit 1,
/// the failed state already on stdout). A transport error aborts through the
/// taxonomy after the client's GET retries are exhausted.
async fn watch_loop(client: &ApiClient, path: &str, format: OutputFormat) -> AppResult<()> {
    let mut interval = WATCH_INITIAL;
    let mut last_status: Option<String> = None;
    loop {
        let value = match client.get_value(path).await {
            Ok(value) => value,
            Err(err) => return Err(emit_err(format, err)),
        };
        // A response with no readable `status` is a shape the CLI cannot follow
        // (spec 004 §5.3); abort as a decode error rather than poll forever.
        let status = match value.get("status").and_then(Value::as_str) {
            Some(status) => status.to_string(),
            None => {
                return Err(emit_err(
                    format,
                    ApiError::Decode("stamp response has no readable `status`".to_string()),
                ))
            }
        };
        if last_status.as_deref() != Some(status.as_str()) {
            emit_ok(format, &value, render_stamp)?;
            last_status = Some(status.clone());
        }
        match classify(&status) {
            Some(Terminal::Green) => return Ok(()),
            Some(Terminal::Failed) => {
                return Err(AppError::Rendered {
                    code: EXIT_OPERATIONAL,
                })
            }
            None => {}
        }
        tokio::time::sleep(interval).await;
        interval = (interval + interval / 2).min(WATCH_MAX);
    }
}

fn render_stamp(v: &Value) -> AppResult<String> {
    let mut out = String::new();
    let _ = write!(
        out,
        "job {}  status: {}",
        require_field(v, "id")?,
        require_field(v, "status")?
    );
    let app = field(v, "appName");
    if !app.is_empty() {
        let org = field(v, "org");
        let _ = write!(out, "  ({app} -> {org})");
    }
    let cert = field(v, "certHash");
    if !cert.is_empty() {
        let _ = write!(out, "  cert: {cert}");
    }
    let error = field(v, "error");
    if !error.is_empty() {
        let _ = write!(out, "\n  error: {error}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    #[test]
    fn classify_marks_only_terminal_states() {
        assert!(matches!(classify("green"), Some(Terminal::Green)));
        assert!(matches!(classify("failed"), Some(Terminal::Failed)));
        assert!(classify("stamping").is_none());
        assert!(classify("queued").is_none());
        assert!(classify("verifying").is_none());
    }

    #[test]
    fn render_stamp_summarizes_the_job() {
        let v = json!({"id": "j_1", "status": "green", "appName": "smoke", "org": "acme-inc"});
        let out = render_stamp(&v).unwrap();
        assert!(out.contains("job j_1"));
        assert!(out.contains("status: green"));
        assert!(out.contains("smoke -> acme-inc"));
    }

    #[test]
    fn watch_exits_ok_on_a_terminal_green() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/stamps/j_1");
            then.status(200)
                .json_body(json!({"id": "j_1", "status": "green"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let outcome = block_on(watch_loop(
            &client,
            "/api/v1/stamps/j_1",
            OutputFormat::Human,
        ))
        .unwrap();
        assert!(outcome.is_ok(), "green must exit 0");
    }

    #[test]
    fn watch_exits_one_on_a_terminal_failed() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/stamps/j_2");
            then.status(200)
                .json_body(json!({"id": "j_2", "status": "failed", "error": "push rejected"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let outcome = block_on(watch_loop(
            &client,
            "/api/v1/stamps/j_2",
            OutputFormat::Human,
        ))
        .unwrap();
        match outcome {
            Err(AppError::Rendered { code }) => assert_eq!(code, EXIT_OPERATIONAL),
            other => panic!("expected Rendered exit-1, got {other:?}"),
        }
    }

    #[test]
    fn new_posts_appname_org_and_posture() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/tenants/t_1/stamps")
                .json_body(json!({
                    "appName": "smoke",
                    "targetOrg": "acme-inc",
                    "posture": "assisted"
                }));
            then.status(200)
                .json_body(json!({"id": "j_1", "status": "queued"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let value = block_on(client.post_value(
            "/api/v1/tenants/t_1/stamps",
            json!({"appName": "smoke", "targetOrg": "acme-inc", "posture": "assisted"}),
        ))
        .unwrap()
        .unwrap();

        mock.assert();
        assert_eq!(value["status"], "queued");
    }

    #[test]
    fn status_surfaces_an_api_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/stamps/j_1");
            then.status(409).json_body(json!({"message": "job gone"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let err = block_on(client.get_value("/api/v1/stamps/j_1"))
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, crate::api::ApiError::Api { status: 409, .. }));
    }

    #[test]
    fn status_maps_a_missing_service_404() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/stamps/j_1");
            then.status(404);
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let err = block_on(client.get_value("/api/v1/stamps/j_1"))
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, crate::api::ApiError::Api { status: 404, .. }));
    }

    #[test]
    fn render_stamp_errors_on_a_statusless_record() {
        // Spec 004 §5.3: a stamp record must carry `id` and `status`; a shape
        // missing them is a decode error, not a silent blank.
        assert!(render_stamp(&json!({"appName": "smoke"})).is_err());
    }

    #[test]
    fn watch_aborts_on_a_statusless_response() {
        // A response with no `status` must abort (exit 1) rather than poll
        // forever; the mock always returns the same statusless body.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/stamps/j_9");
            then.status(200).json_body(json!({"id": "j_9"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let outcome = block_on(watch_loop(
            &client,
            "/api/v1/stamps/j_9",
            OutputFormat::Human,
        ))
        .unwrap();
        assert!(
            outcome.is_err(),
            "a statusless response must not loop forever"
        );
    }

    #[test]
    fn stamp_envelope_snapshot() {
        let data = json!({"id": "j_1", "status": "queued"});
        let env = crate::verbs::success_envelope(&data);
        assert_eq!(env, json!({ "ok": true, "data": data }));
    }
}
