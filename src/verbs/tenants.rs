//! The `tenants` verb family (spec 004 §5.1): list, show, install-url.

use std::fmt::Write;

use serde_json::Value;

use super::{as_array, client_for, emit_err, emit_ok, field, render, table};
use crate::api::{block_on, ApiClient, ApiError};
use crate::config::ResolvedConfig;
use crate::error::AppResult;
use crate::output::OutputFormat;

/// GET /api/v1/tenants: the `tenants list` request, shared by both faces (the
/// CLI renders it, the MCP `tenants_list` tool returns its envelope, spec 005).
pub(crate) async fn list_request(client: &ApiClient) -> Result<Value, ApiError> {
    client.get_value("/api/v1/tenants").await
}

/// GET /api/v1/tenants/:id (includes installations): the `tenants show` request.
pub(crate) async fn show_request(client: &ApiClient, id: &str) -> Result<Value, ApiError> {
    client.get_value(&format!("/api/v1/tenants/{id}")).await
}

/// `tenants list` -> GET /api/v1/tenants.
pub fn list(resolved: &ResolvedConfig, format: OutputFormat, debug: bool) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(list_request(&client))?;
    render(format, result, render_list)
}

/// `tenants show <id>` -> GET /api/v1/tenants/:id (includes installations).
pub fn show(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    id: &str,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(show_request(&client, id))?;
    render(format, result, render_detail)
}

/// `tenants install-url <id>` -> GET /api/v1/tenants/:id/github/install-url.
/// With `open`, the returned URL is launched in the default browser as well as
/// printed. The passthrough envelope is emitted either way.
pub fn install_url(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    id: &str,
    open: bool,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let path = format!("/api/v1/tenants/{id}/github/install-url");
    match block_on(client.get_value(&path))? {
        Ok(value) => {
            // Print the URL (or envelope) first: it is the actual deliverable, so
            // a best-effort browser launch never costs the operator the URL.
            emit_ok(format, &value, render_install_url)?;
            if open {
                match value.get("url").and_then(Value::as_str) {
                    Some(url) => super::open_in_browser(url),
                    None => eprintln!("warning: the response has no install URL to open"),
                }
            }
            Ok(())
        }
        Err(err) => Err(emit_err(format, err)),
    }
}

fn render_list(v: &Value) -> AppResult<String> {
    let tenants = as_array(v)?;
    if tenants.is_empty() {
        return Ok("no tenants".to_string());
    }
    let rows: Vec<Vec<String>> = tenants
        .iter()
        .map(|t| vec![field(t, "id"), field(t, "name"), field(t, "createdAt")])
        .collect();
    Ok(table(&["ID", "NAME", "CREATED"], &rows))
}

fn render_detail(v: &Value) -> AppResult<String> {
    let mut out = String::new();
    let _ = write!(out, "id:      {}", field(v, "id"));
    let _ = write!(out, "\nname:    {}", field(v, "name"));
    let owner = field(v, "ownerUserId");
    if !owner.is_empty() {
        let _ = write!(out, "\nowner:   {owner}");
    }
    let created = field(v, "createdAt");
    if !created.is_empty() {
        let _ = write!(out, "\ncreated: {created}");
    }

    match v.get("installations").and_then(Value::as_array) {
        Some(installs) if !installs.is_empty() => {
            let rows: Vec<Vec<String>> = installs
                .iter()
                .map(|i| {
                    vec![
                        field(i, "id"),
                        field(i, "githubOrg"),
                        field(i, "installationId"),
                        field(i, "status"),
                    ]
                })
                .collect();
            let _ = write!(
                out,
                "\n\ninstallations:\n{}",
                table(&["ID", "ORG", "INSTALLATION", "STATUS"], &rows)
            );
        }
        _ => {
            let _ = write!(out, "\n\ninstallations: none");
        }
    }
    Ok(out)
}

fn render_install_url(v: &Value) -> AppResult<String> {
    match v.get("url").and_then(Value::as_str) {
        Some(url) => Ok(url.to_string()),
        None => Ok(v.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    use crate::api::ApiClient;

    #[test]
    fn list_renders_a_table() {
        let value = json!([
            {"id": "t_1", "name": "Acme", "createdAt": "2026-07-01"},
            {"id": "t_2", "name": "Beta", "createdAt": "2026-07-02"}
        ]);
        let out = render_list(&value).unwrap();
        assert!(out.contains("ID"));
        assert!(out.contains("t_1"));
        assert!(out.contains("Acme"));
    }

    #[test]
    fn list_empty_states_when_no_tenants() {
        assert_eq!(render_list(&json!([])).unwrap(), "no tenants");
    }

    #[test]
    fn detail_shows_installations() {
        let value = json!({
            "id": "t_1",
            "name": "Acme",
            "ownerUserId": "u_1",
            "createdAt": "2026-07-01",
            "installations": [
                {"id": "i_1", "githubOrg": "acme-inc", "installationId": 125344051, "status": "active"}
            ]
        });
        let out = render_detail(&value).unwrap();
        assert!(out.contains("name:    Acme"));
        assert!(out.contains("acme-inc"));
        assert!(out.contains("125344051"));
    }

    #[test]
    fn install_url_renders_bare_url_for_humans() {
        let value = json!({"url": "https://github.com/apps/x/installations/new?state=abc"});
        assert_eq!(
            render_install_url(&value).unwrap(),
            "https://github.com/apps/x/installations/new?state=abc"
        );
    }

    #[test]
    fn list_get_hits_the_tenants_endpoint() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants");
            then.status(200)
                .json_body(json!([{"id": "t_1", "name": "Acme"}]));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let value = block_on(client.get_value("/api/v1/tenants"))
            .unwrap()
            .unwrap();

        mock.assert();
        assert_eq!(value[0]["id"], "t_1");
    }

    #[test]
    fn list_surfaces_an_api_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants");
            then.status(403)
                .json_body(json!({"message": "tenant suspended"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let err = block_on(client.get_value("/api/v1/tenants"))
            .unwrap()
            .unwrap_err();
        match err {
            crate::api::ApiError::Api { status, message } => {
                assert_eq!(status, 403);
                assert_eq!(message, "tenant suspended");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn show_maps_a_missing_service_404() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants/t_9");
            then.status(404);
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let err = block_on(client.get_value("/api/v1/tenants/t_9"))
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, crate::api::ApiError::Api { status: 404, .. }));
    }

    #[test]
    fn list_envelope_snapshot() {
        let data = json!([{"id": "t_1", "name": "Acme"}]);
        let env = crate::verbs::success_envelope_value(&data);
        assert_eq!(env, json!({ "ok": true, "data": data }));
    }
}
