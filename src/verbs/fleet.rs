//! The `fleet` verb family (spec 004 §5.1): list, deploy, update, backup,
//! remove. `remove` carries the confirm-name guard verbatim (stagecraft spec
//! 006 §3); there is deliberately no `--force`/`--yes` shortcut.

use std::fmt::Write;

use serde_json::{json, Map, Value};

use super::{as_array, client_for, field, render, require_field, table};
use crate::api::{block_on, ApiClient, ApiError};
use crate::config::ResolvedConfig;
use crate::error::AppResult;
use crate::output::OutputFormat;

/// GET /api/v1/tenants/:id/fleet: the `fleet list` request, shared by both faces
/// (spec 005 reuses each of these for the matching MCP tool).
pub(crate) async fn list_request(client: &ApiClient, tenant: &str) -> Result<Value, ApiError> {
    client
        .get_value(&format!("/api/v1/tenants/{tenant}/fleet"))
        .await
}

/// POST /api/v1/tenants/:id/fleet {name, image}: the `fleet deploy` request.
pub(crate) async fn deploy_request(
    client: &ApiClient,
    tenant: &str,
    app: &str,
    image: &str,
) -> Result<Value, ApiError> {
    let path = format!("/api/v1/tenants/{tenant}/fleet");
    let mut body = Map::new();
    body.insert("name".to_string(), Value::String(app.to_string()));
    body.insert("image".to_string(), Value::String(image.to_string()));
    client.post_value(&path, Value::Object(body)).await
}

/// POST /api/v1/fleet/:appId/update {image}: the `fleet update` request.
pub(crate) async fn update_request(
    client: &ApiClient,
    app_id: &str,
    image: &str,
) -> Result<Value, ApiError> {
    let path = format!("/api/v1/fleet/{app_id}/update");
    client.post_value(&path, json!({ "image": image })).await
}

/// POST /api/v1/fleet/:appId/backup: the `fleet backup` request.
pub(crate) async fn backup_request(client: &ApiClient, app_id: &str) -> Result<Value, ApiError> {
    let path = format!("/api/v1/fleet/{app_id}/backup");
    client.post_value(&path, json!({})).await
}

/// DELETE /api/v1/fleet/:appId {confirm}: the `fleet remove` request. The
/// confirm name is echoed in the body (stagecraft spec 006 §3); the platform
/// rejects a mismatch, so both faces forward it as-is rather than pre-judging.
pub(crate) async fn remove_request(
    client: &ApiClient,
    app_id: &str,
    confirm: &str,
) -> Result<Value, ApiError> {
    let path = format!("/api/v1/fleet/{app_id}");
    client
        .delete_value(&path, json!({ "confirm": confirm }))
        .await
}

/// `fleet list --tenant <id>` -> GET /api/v1/tenants/:id/fleet.
pub fn list(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    tenant: &str,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(list_request(&client, tenant))?;
    render(format, result, render_list)
}

/// `fleet deploy --tenant <id> --app <name> --image <ref>`
/// -> POST /api/v1/tenants/:id/fleet.
pub fn deploy(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    tenant: &str,
    app: &str,
    image: &str,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(deploy_request(&client, tenant, app, image))?;
    render(format, result, render_app)
}

/// `fleet update <appId> --image <ref>` -> POST /api/v1/fleet/:appId/update.
pub fn update(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    app_id: &str,
    image: &str,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(update_request(&client, app_id, image))?;
    render(format, result, render_app)
}

/// `fleet backup <appId>` -> POST /api/v1/fleet/:appId/backup.
pub fn backup(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    app_id: &str,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(backup_request(&client, app_id))?;
    render(format, result, render_op)
}

/// `fleet remove <appId> --confirm <name>` -> DELETE /api/v1/fleet/:appId.
pub fn remove(
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
    app_id: &str,
    confirm: &str,
) -> AppResult<()> {
    let client = client_for(resolved, debug)?;
    let result = block_on(remove_request(&client, app_id, confirm))?;
    render(format, result, render_app)
}

fn render_list(v: &Value) -> AppResult<String> {
    let apps = as_array(v)?;
    if apps.is_empty() {
        return Ok("no fleet apps".to_string());
    }
    let rows: Vec<Vec<String>> = apps
        .iter()
        .map(|a| {
            vec![
                field(a, "id"),
                field(a, "name"),
                field(a, "status"),
                field(a, "image"),
                field(a, "createdAt"),
            ]
        })
        .collect();
    Ok(table(&["ID", "NAME", "STATUS", "IMAGE", "CREATED"], &rows))
}

/// A single fleet app (deploy/update/remove responses). A 204-style empty body
/// decodes to null; render it as a plain acknowledgement.
fn render_app(v: &Value) -> AppResult<String> {
    if v.is_null() {
        return Ok("done".to_string());
    }
    let mut out = String::new();
    let _ = write!(out, "app {}", require_field(v, "id")?);
    let name = field(v, "name");
    if !name.is_empty() {
        let _ = write!(out, "  {name}");
    }
    let _ = write!(out, "  status: {}", require_field(v, "status")?);
    let image = field(v, "image");
    if !image.is_empty() {
        let _ = write!(out, "  image: {image}");
    }
    let host = field(v, "host");
    if !host.is_empty() {
        let _ = write!(out, "  host: {host}");
    }
    Ok(out)
}

/// A fleet operation record (backup responses): id, kind, status, artifact.
fn render_op(v: &Value) -> AppResult<String> {
    if v.is_null() {
        return Ok("done".to_string());
    }
    let mut out = String::new();
    let _ = write!(
        out,
        "op {}  {}  status: {}",
        require_field(v, "id")?,
        field(v, "kind"),
        require_field(v, "status")?
    );
    let log = field(v, "log");
    if !log.is_empty() {
        let _ = write!(out, "\n  {log}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    use crate::api::ApiClient;

    #[test]
    fn list_renders_a_table() {
        let v = json!([
            {"id": "a_1", "name": "smoke", "status": "running", "image": "ghcr.io/x:1", "createdAt": "2026-07-01"}
        ]);
        let out = render_list(&v).unwrap();
        assert!(out.contains("STATUS"));
        assert!(out.contains("running"));
        assert!(out.contains("ghcr.io/x:1"));
    }

    #[test]
    fn app_renders_a_null_body_as_done() {
        assert_eq!(render_app(&Value::Null).unwrap(), "done");
    }

    #[test]
    fn remove_sends_confirm_in_the_body() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(DELETE)
                .path("/api/v1/fleet/a_1")
                .json_body(json!({ "confirm": "smoke" }));
            then.status(200)
                .json_body(json!({"id": "a_1", "name": "smoke", "status": "removed"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let value =
            block_on(client.delete_value("/api/v1/fleet/a_1", json!({ "confirm": "smoke" })))
                .unwrap()
                .unwrap();

        mock.assert();
        assert_eq!(value["status"], "removed");
    }

    #[test]
    fn deploy_posts_name_and_image() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/tenants/t_1/fleet")
                .json_body(json!({ "name": "smoke", "image": "ghcr.io/x:1" }));
            then.status(200)
                .json_body(json!({"id": "a_1", "name": "smoke", "status": "placing"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let value = block_on(client.post_value(
            "/api/v1/tenants/t_1/fleet",
            json!({ "name": "smoke", "image": "ghcr.io/x:1" }),
        ))
        .unwrap()
        .unwrap();

        mock.assert();
        assert_eq!(value["status"], "placing");
    }

    #[test]
    fn list_gets_the_tenant_fleet() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants/t_1/fleet");
            then.status(200)
                .json_body(json!([{"id": "a_1", "name": "smoke", "status": "running"}]));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let value = block_on(client.get_value("/api/v1/tenants/t_1/fleet"))
            .unwrap()
            .unwrap();

        mock.assert();
        assert_eq!(value[0]["status"], "running");
    }

    #[test]
    fn backup_posts_to_the_backup_endpoint() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/api/v1/fleet/a_1/backup");
            then.status(200)
                .json_body(json!({"id": "op_1", "kind": "backup", "status": "done"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let value = block_on(client.post_value("/api/v1/fleet/a_1/backup", json!({})))
            .unwrap()
            .unwrap();

        mock.assert();
        assert_eq!(value["kind"], "backup");
    }

    #[test]
    fn list_surfaces_an_api_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants/t_1/fleet");
            then.status(403).json_body(json!({"message": "forbidden"}));
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let err = block_on(client.get_value("/api/v1/tenants/t_1/fleet"))
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, crate::api::ApiError::Api { status: 403, .. }));
    }

    #[test]
    fn list_maps_a_missing_service_404() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants/t_1/fleet");
            then.status(404);
        });

        let client = ApiClient::new(server.base_url(), Some("tok".into()), false).unwrap();
        let err = block_on(client.get_value("/api/v1/tenants/t_1/fleet"))
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, crate::api::ApiError::Api { status: 404, .. }));
    }

    #[test]
    fn fleet_envelope_snapshot() {
        let data = json!({"id": "a_1", "name": "smoke", "status": "placing"});
        let env = crate::verbs::success_envelope_value(&data);
        assert_eq!(env, json!({ "ok": true, "data": data }));
    }
}
