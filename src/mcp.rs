//! The MCP face (spec 005): `stagecraft mcp` runs a Model Context Protocol
//! server over stdio so a coding agent operates under Stagecraft governance
//! natively.
//!
//! Transport is hand-rolled JSON-RPC 2.0 over newline-delimited stdio (spec 005
//! §1 Transport decision): one JSON object per line, no embedded newlines. The
//! server speaks `initialize`, `notifications/initialized`, `tools/list`,
//! `tools/call`, `ping`, and `shutdown`, and treats stdin EOF as shutdown.
//!
//! The MCP face is not a privileged side door: every tool calls the identical
//! spec 004 verb request (the endpoint and body knowledge lives once, in
//! [`crate::verbs`]), and the tool result is the spec 004 §5.2 `{ok,data|error}`
//! envelope verbatim, so any attestation/record ids in the plane's payload are
//! carried through. The destructive guards pass through to the agent unchanged:
//! `stamp_new` requires an explicit `posture`, `fleet_remove` a `confirm_name`.
//! Neither `login` nor `install-url --open` is exposed, so an agent can never
//! trigger a browser flow; an unauthenticated server answers every tool call
//! with a structured error naming `stagecraft login`.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::api::{normalize_base_url, ApiClient, ApiError};
use crate::cli::Posture;
use crate::config::ResolvedConfig;
use crate::error::{AppError, AppResult};
use crate::verbs::{envelope_value, error_envelope, fleet, stamp, tenants};

/// Server identity reported in `initialize` and used as the `.mcp.json` key.
const SERVER_NAME: &str = "stagecraft";
/// The MCP protocol version we answer with when a client offers none.
const PROTOCOL_VERSION: &str = "2025-06-18";

// JSON-RPC 2.0 error codes (the standard reserved set).
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the server needs to serve tool calls: the resolved base URL, the
/// credential loaded from the spec 003 store (both optional), and one runtime
/// reused across the sequential request loop. A `None` token is a
/// running-but-unauthenticated server (spec 005 §1): it starts and answers every
/// tool call with a login instruction rather than refusing to boot.
struct ServerContext {
    base_url: Option<String>,
    token: Option<String>,
    debug: bool,
    /// One current-thread runtime, built at startup and reused for every tool
    /// call. Calls are strictly sequential (the stdio loop is single-threaded),
    /// so a per-call runtime would only pay repeated spin-up for no concurrency.
    runtime: tokio::runtime::Runtime,
}

/// `stagecraft mcp`: run the stdio server until stdin closes (or a `shutdown`).
///
/// The credential is loaded once, at startup, from the spec 003 store; a client
/// that logs in afterward reconnects (Claude Code restarts the server on config
/// change), which keeps the request loop free of filesystem access and testable.
/// A missing or unreadable store is not fatal: the server starts unauthenticated
/// and instructs `stagecraft login` per call (spec 005 §1 "if unauthenticated it
/// starts").
pub fn run(resolved: &ResolvedConfig, debug: bool) -> AppResult<()> {
    let base_url = resolved.base_url.value.as_deref().map(normalize_base_url);
    let token = match &base_url {
        Some(url) => {
            match crate::auth::load_token(url) {
                Ok(token) => token,
                // A corrupt/unreadable store is an unauthenticated start, not a
                // refusal to boot. Log to stderr (the MCP server's log channel;
                // stdout is the JSON-RPC stream) and carry on with no credential.
                Err(err) => {
                    eprintln!("warning: could not read stored credentials ({err}); starting unauthenticated");
                    None
                }
            }
        }
        None => None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Operational(e.into()))?;
    let ctx = ServerContext {
        base_url,
        token,
        debug,
        runtime,
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve(stdin.lock(), stdout.lock(), &ctx)
}

/// `stagecraft mcp --print-config`: print the `.mcp.json` snippet that installs
/// this binary as a Claude Code MCP server. The command path is this executable
/// so the snippet works before the binary is on `PATH`; when a base URL is
/// resolved it is pinned as `STAGECRAFT_BASE_URL` so the snippet is turnkey.
pub fn print_config(resolved: &ResolvedConfig) -> AppResult<()> {
    let command = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| SERVER_NAME.to_string());
    let base_url = resolved.base_url.value.as_deref().map(normalize_base_url);
    let snippet = config_snippet(base_url.as_deref(), &command);
    println!(
        "{}",
        serde_json::to_string_pretty(&snippet).expect("serializing an owned snippet cannot fail")
    );
    Ok(())
}

/// Build the `.mcp.json` snippet value (pure, so it is unit-testable).
fn config_snippet(base_url: Option<&str>, command: &str) -> Value {
    let mut server = json!({ "command": command, "args": ["mcp"] });
    if let Some(base) = base_url {
        server["env"] = json!({ "STAGECRAFT_BASE_URL": base });
    }
    json!({ "mcpServers": { SERVER_NAME: server } })
}

// --- the stdio JSON-RPC loop ------------------------------------------------

/// The result of handling one line: an optional response to write and whether
/// the server should stop afterward (a `shutdown`).
struct Outcome {
    response: Option<String>,
    stop: bool,
}

impl Outcome {
    fn respond(response: String) -> Self {
        Outcome {
            response: Some(response),
            stop: false,
        }
    }
    fn silent() -> Self {
        Outcome {
            response: None,
            stop: false,
        }
    }
}

/// Read newline-delimited JSON-RPC messages, dispatch each, and write each
/// response as one line. EOF (the client closed stdin) is a clean shutdown.
///
/// The read is byte-oriented on purpose: one malformed line (invalid UTF-8, a
/// copy-paste artifact, a truncated multi-byte character) must be answered with
/// a parse error and skipped, never kill the whole server, so the next valid
/// request is still served.
fn serve<R: BufRead, W: Write>(mut reader: R, mut writer: W, ctx: &ServerContext) -> AppResult<()> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let read = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| AppError::Operational(e.into()))?;
        if read == 0 {
            return Ok(());
        }
        let outcome = handle_message(&buf, ctx);
        if let Some(response) = outcome.response {
            writeln!(writer, "{response}").map_err(|e| AppError::Operational(e.into()))?;
            writer
                .flush()
                .map_err(|e| AppError::Operational(e.into()))?;
        }
        if outcome.stop {
            return Ok(());
        }
    }
}

/// Parse and dispatch a single message (the raw line bytes). Failures degrade to
/// a JSON-RPC error the caller writes, never an error that aborts the loop:
/// invalid UTF-8 or invalid JSON is a parse error (-32700); a well-formed JSON
/// value that is not a valid request object is an invalid request (-32600).
fn handle_message(bytes: &[u8], ctx: &ServerContext) -> Outcome {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.trim(),
        Err(_) => {
            return Outcome::respond(error_response(
                &Value::Null,
                PARSE_ERROR,
                "parse error: line is not valid UTF-8",
            ))
        }
    };
    if text.is_empty() {
        return Outcome::silent();
    }
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            return Outcome::respond(error_response(
                &Value::Null,
                PARSE_ERROR,
                "parse error: message is not valid JSON",
            ))
        }
    };
    let object = match value.as_object() {
        Some(object) => object,
        None => {
            return Outcome::respond(error_response(
                &Value::Null,
                INVALID_REQUEST,
                "invalid request: message is not a JSON object",
            ))
        }
    };

    // An absent `id` marks a notification (no reply): the only one we expect,
    // `notifications/initialized`, needs no action, so all notifications are
    // answered with silence. An explicit `id` (including `null`) is a request.
    let id = match object.get("id") {
        Some(id) => id.clone(),
        None => return Outcome::silent(),
    };
    let method = match object.get("method").and_then(Value::as_str) {
        Some(method) => method,
        None => {
            return Outcome::respond(error_response(
                &id,
                INVALID_REQUEST,
                "invalid request: `method` is missing or not a string",
            ))
        }
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => Outcome::respond(initialize_response(&id, &params)),
        "ping" => Outcome::respond(success_response(&id, json!({}))),
        "tools/list" => Outcome::respond(success_response(
            &id,
            json!({ "tools": tool_definitions() }),
        )),
        "tools/call" => Outcome::respond(tools_call_response(&id, &params, ctx)),
        "shutdown" => Outcome {
            response: Some(success_response(&id, json!({}))),
            stop: true,
        },
        other => Outcome::respond(error_response(
            &id,
            METHOD_NOT_FOUND,
            &format!("method not found: {other}"),
        )),
    }
}

/// The `initialize` result: capabilities, server identity, and the negotiated
/// protocol version (echo the client's when offered, else our default).
fn initialize_response(id: &Value, params: &Value) -> String {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    success_response(
        id,
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") }
        }),
    )
}

/// Handle `tools/call`: validate `name`, dispatch to the verb, and shape the
/// outcome. An invalid parameter (a missing guard, an unknown tool) is a
/// JSON-RPC invalid-params error; a completed call (success or an operational
/// failure) is a tool result carrying the envelope.
fn tools_call_response(id: &Value, params: &Value, ctx: &ServerContext) -> String {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => return error_response(id, INVALID_PARAMS, "tools/call: missing `name`"),
    };
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);
    match call_tool(ctx, name, args) {
        Dispatch::Invalid(message) => error_response(id, INVALID_PARAMS, &message),
        Dispatch::Envelope(envelope) => success_response(id, tool_result(envelope)),
    }
}

/// Wrap a verb envelope as an MCP tool result: the pretty envelope as text (so a
/// text-only client still sees the whole governed record), the envelope as
/// `structuredContent` for typed clients, and `isError` set from `ok`.
fn tool_result(envelope: Value) -> Value {
    let is_error = envelope.get("ok") == Some(&Value::Bool(false));
    let text =
        serde_json::to_string_pretty(&envelope).expect("serializing an owned envelope cannot fail");
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": envelope,
        "isError": is_error,
    })
}

// --- tool registry ----------------------------------------------------------

/// The advertised tools (spec 005 §1): each name maps to one spec 004 verb, with
/// a precise input schema so agents get typed parameters. The guard fields carry
/// loud descriptions: the agent must never guess `posture` or `confirm_name`.
fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "tenants_list",
            "description": "List the tenants the authenticated identity owns.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "tenants_show",
            "description": "Show one tenant, including its GitHub installations.",
            "inputSchema": {
                "type": "object",
                "properties": { "tenant_id": { "type": "string", "description": "Tenant id." } },
                "required": ["tenant_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "stamp_new",
            "description": "Request a new governance stamp (a born-green repo) for a tenant.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tenant_id": { "type": "string", "description": "Tenant the stamp is charged to." },
                    "app_name": { "type": "string", "description": "Application name for the stamped repo." },
                    "org": { "type": "string", "description": "Target GitHub org the repo is created in." },
                    "posture": {
                        "type": "string",
                        "enum": ["none", "assisted", "autonomous"],
                        "description": "Governance posture. REQUIRED. Must reflect the human operator's declared intent; the agent must not guess or default it."
                    },
                    "frontend": { "type": "string", "description": "Optional frontend flavor slot (for example `vue`)." }
                },
                "required": ["tenant_id", "app_name", "org", "posture"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "stamp_status",
            "description": "Poll a stamp job's status once. Terminal states are green and failed; loop yourself to watch.",
            "inputSchema": {
                "type": "object",
                "properties": { "job_id": { "type": "string", "description": "Stamp job id." } },
                "required": ["job_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "fleet_list",
            "description": "List a tenant's fleet apps.",
            "inputSchema": {
                "type": "object",
                "properties": { "tenant_id": { "type": "string", "description": "Tenant whose fleet is listed." } },
                "required": ["tenant_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "fleet_deploy",
            "description": "Deploy an image as a new fleet app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tenant_id": { "type": "string", "description": "Tenant the app belongs to." },
                    "app": { "type": "string", "description": "Fleet app name." },
                    "image": { "type": "string", "description": "Container image reference to place." }
                },
                "required": ["tenant_id", "app", "image"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "fleet_update",
            "description": "Roll a fleet app to a new image.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app_id": { "type": "string", "description": "Fleet app id." },
                    "image": { "type": "string", "description": "New container image reference." }
                },
                "required": ["app_id", "image"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "fleet_backup",
            "description": "Back up a fleet app's volume.",
            "inputSchema": {
                "type": "object",
                "properties": { "app_id": { "type": "string", "description": "Fleet app id." } },
                "required": ["app_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "fleet_remove",
            "description": "Remove a fleet app. Destructive and irreversible.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app_id": { "type": "string", "description": "Fleet app id." },
                    "confirm_name": {
                        "type": "string",
                        "description": "The fleet app's exact name, typed by the human to authorize teardown. A safety guard; the agent must not fabricate or guess it."
                    }
                },
                "required": ["app_id", "confirm_name"],
                "additionalProperties": false
            }
        }),
    ]
}

/// A tool dispatch outcome: an invalid-params rejection, or a completed envelope.
enum Dispatch {
    /// Reject as JSON-RPC invalid-params (-32602). The two guarded tools land
    /// here when `posture` / `confirm_name` is missing, empty, or malformed.
    Invalid(String),
    /// A completed tool result: the `{ok,data|error}` envelope to wrap.
    Envelope(Value),
}

/// Unwrap a validated parameter or short-circuit `call_tool` with an
/// invalid-params rejection. Each arm owns its parameter as a `String` for the
/// move into the async request.
macro_rules! param {
    ($result:expr) => {
        match $result {
            Ok(value) => value.to_string(),
            Err(message) => return Dispatch::Invalid(message),
        }
    };
}

/// Validate parameters, then run the matching spec 004 verb request. Parameter
/// validation stays in the MCP layer (it is transport concern); the endpoint and
/// body construction stay in the verb layer (they are the shared contract).
fn call_tool(ctx: &ServerContext, name: &str, args: &Value) -> Dispatch {
    match name {
        "tenants_list" => Dispatch::Envelope(execute(ctx, |client| async move {
            tenants::list_request(&client).await
        })),
        "tenants_show" => {
            let id = param!(req(args, "tenant_id", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                tenants::show_request(&client, &id).await
            }))
        }
        "stamp_new" => {
            let tenant = param!(req(args, "tenant_id", name));
            let app = param!(req(args, "app_name", name));
            let org = param!(req(args, "org", name));
            let posture =
                match req(args, "posture", name).and_then(|token| parse_posture(token, name)) {
                    Ok(posture) => posture,
                    Err(message) => return Dispatch::Invalid(message),
                };
            let frontend = opt(args, "frontend").map(str::to_string);
            Dispatch::Envelope(execute(ctx, move |client| async move {
                stamp::new_request(&client, &tenant, &app, &org, frontend.as_deref(), posture).await
            }))
        }
        "stamp_status" => {
            let job_id = param!(req(args, "job_id", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                stamp::status_request(&client, &job_id).await
            }))
        }
        "fleet_list" => {
            let tenant = param!(req(args, "tenant_id", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                fleet::list_request(&client, &tenant).await
            }))
        }
        "fleet_deploy" => {
            let tenant = param!(req(args, "tenant_id", name));
            let app = param!(req(args, "app", name));
            let image = param!(req(args, "image", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                fleet::deploy_request(&client, &tenant, &app, &image).await
            }))
        }
        "fleet_update" => {
            let app_id = param!(req(args, "app_id", name));
            let image = param!(req(args, "image", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                fleet::update_request(&client, &app_id, &image).await
            }))
        }
        "fleet_backup" => {
            let app_id = param!(req(args, "app_id", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                fleet::backup_request(&client, &app_id).await
            }))
        }
        "fleet_remove" => {
            let app_id = param!(req(args, "app_id", name));
            let confirm = param!(req(args, "confirm_name", name));
            Dispatch::Envelope(execute(ctx, move |client| async move {
                fleet::remove_request(&client, &app_id, &confirm).await
            }))
        }
        other => Dispatch::Invalid(format!("unknown tool `{other}`")),
    }
}

/// Build the client (or the structured error envelope for an unauthenticated /
/// unconfigured server), run `request` against it, and wrap the outcome in the
/// shared envelope. `request` owns the endpoint and body (it is the spec 004
/// verb); the MCP face only chooses which to call.
fn execute<F, Fut>(ctx: &ServerContext, request: F) -> Value
where
    F: FnOnce(ApiClient) -> Fut,
    Fut: std::future::Future<Output = Result<Value, ApiError>>,
{
    let client = match client_or_envelope(ctx) {
        Ok(client) => client,
        Err(envelope) => return envelope,
    };
    // The runtime is the one built at startup and reused across the loop; the
    // request runs to completion on it and the outcome becomes the envelope.
    envelope_value(ctx.runtime.block_on(request(client)))
}

/// Build an authenticated client, or the structured error envelope that the tool
/// result carries when the server cannot make a call: `config` when no base URL
/// was set, `unauthenticated` (naming `stagecraft login`) when no credential is
/// stored. An agent can never resolve the latter itself: `login` is not a tool.
fn client_or_envelope(ctx: &ServerContext) -> Result<ApiClient, Value> {
    let base_url = match &ctx.base_url {
        Some(base_url) => base_url,
        None => {
            return Err(error_envelope(
                "config",
                "no control-plane base URL; launch `stagecraft mcp` with STAGECRAFT_BASE_URL set (or --base-url)".to_string(),
                None,
            ))
        }
    };
    let token = match &ctx.token {
        Some(token) => token,
        None => {
            return Err(error_envelope(
                "unauthenticated",
                ApiError::Unauthenticated.to_string(),
                None,
            ))
        }
    };
    ApiClient::new(base_url.clone(), Some(token.clone()), ctx.debug)
        .map_err(|err| error_envelope(err.kind(), err.to_string(), err.status()))
}

// --- parameter extraction ---------------------------------------------------

/// A required string parameter: present, a string, and not blank. Absent, empty,
/// or the wrong type is an invalid-params message naming the tool and field.
fn req<'a>(args: &'a Value, key: &str, tool: &str) -> Result<&'a str, String> {
    match args.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.as_str()),
        Some(Value::String(_)) => Err(format!("{tool}: `{key}` must not be empty")),
        Some(_) => Err(format!("{tool}: `{key}` must be a string")),
        None => Err(format!("{tool}: missing required parameter `{key}`")),
    }
}

/// An optional string parameter; blank or non-string is treated as absent.
fn opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    match args.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.as_str()),
        _ => None,
    }
}

/// Parse the `stamp_new` posture enum by hand (the schema advertises it, but a
/// hand-rolled server still validates). Unknown tokens are rejected, never
/// defaulted: neither face invents a posture (spec 004 §5.1, spec 005 §1).
fn parse_posture(token: &str, tool: &str) -> Result<Posture, String> {
    Posture::from_wire(token)
        .ok_or_else(|| format!("{tool}: `posture` must be one of none, assisted, autonomous"))
}

/// Build the JSON-RPC success response line for `id`.
fn success_response(id: &Value, result: Value) -> String {
    line(json!({ "jsonrpc": "2.0", "id": id.clone(), "result": result }))
}

/// Build the JSON-RPC error response line for `id`.
fn error_response(id: &Value, code: i64, message: &str) -> String {
    line(
        json!({ "jsonrpc": "2.0", "id": id.clone(), "error": { "code": code, "message": message } }),
    )
}

/// Serialize one message to a single compact line (the stdio framing).
fn line(value: Value) -> String {
    serde_json::to_string(&value).expect("serializing an owned response cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    /// A test context with the fields under test and a fresh reused runtime.
    fn ctx(base_url: Option<&str>, token: Option<&str>) -> ServerContext {
        ServerContext {
            base_url: base_url.map(str::to_string),
            token: token.map(str::to_string),
            debug: false,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        }
    }

    fn bare_ctx() -> ServerContext {
        ctx(None, None)
    }

    fn authed_ctx(base_url: &str) -> ServerContext {
        ctx(Some(&normalize_base_url(base_url)), Some("tok"))
    }

    /// Drive `serve` over scripted request lines, returning the parsed response
    /// lines (one per request that carried an id).
    fn drive(ctx: &ServerContext, requests: &[Value]) -> Vec<Value> {
        let input = requests
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut output: Vec<u8> = Vec::new();
        serve(io::Cursor::new(input), &mut output, ctx).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect()
    }

    fn required(tool: &Value) -> Vec<String> {
        tool["inputSchema"]["required"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn initialize_reports_server_info_and_echoes_protocol_version() {
        let out = drive(
            &bare_ctx(),
            &[json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-03-26", "capabilities": {} }
            })],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], 1);
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "stagecraft");
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-03-26");
        assert!(out[0]["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialize_defaults_protocol_version_when_client_offers_none() {
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })],
        );
        assert_eq!(out[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn tools_list_advertises_the_nine_verbs_with_guard_schemas() {
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })],
        );
        let tools = out[0]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in [
            "tenants_list",
            "tenants_show",
            "stamp_new",
            "stamp_status",
            "fleet_list",
            "fleet_deploy",
            "fleet_update",
            "fleet_backup",
            "fleet_remove",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
        assert_eq!(names.len(), 9, "exactly the nine spec 005 tools");

        let stamp_new = tools.iter().find(|t| t["name"] == "stamp_new").unwrap();
        assert!(required(stamp_new).contains(&"posture".to_string()));
        let fleet_remove = tools.iter().find(|t| t["name"] == "fleet_remove").unwrap();
        assert!(required(fleet_remove).contains(&"confirm_name".to_string()));
    }

    #[test]
    fn every_advertised_tool_is_dispatchable() {
        // The tools/list registry and the call_tool match must not drift apart:
        // no advertised name may fall through to the unknown-tool arm.
        let ctx = bare_ctx();
        for def in tool_definitions() {
            let name = def["name"].as_str().unwrap();
            if let Dispatch::Invalid(message) = call_tool(&ctx, name, &json!({})) {
                assert!(
                    !message.contains("unknown tool"),
                    "{name} is advertised but not dispatchable"
                );
            }
        }
    }

    #[test]
    fn stamp_new_without_posture_is_rejected() {
        match call_tool(
            &bare_ctx(),
            "stamp_new",
            &json!({ "tenant_id": "t_1", "app_name": "smoke", "org": "acme" }),
        ) {
            Dispatch::Invalid(message) => assert!(message.contains("posture"), "{message}"),
            Dispatch::Envelope(_) => panic!("a missing posture must be rejected, not run"),
        }
    }

    #[test]
    fn stamp_new_with_unknown_posture_is_rejected() {
        match call_tool(
            &bare_ctx(),
            "stamp_new",
            &json!({ "tenant_id": "t_1", "app_name": "x", "org": "o", "posture": "yolo" }),
        ) {
            Dispatch::Invalid(message) => assert!(message.contains("posture")),
            Dispatch::Envelope(_) => panic!("an unknown posture must be rejected"),
        }
    }

    #[test]
    fn fleet_remove_without_confirm_name_is_rejected() {
        match call_tool(&bare_ctx(), "fleet_remove", &json!({ "app_id": "a_1" })) {
            Dispatch::Invalid(message) => assert!(message.contains("confirm_name")),
            Dispatch::Envelope(_) => panic!("a missing confirm_name must be rejected"),
        }
    }

    #[test]
    fn fleet_remove_with_empty_confirm_name_is_rejected() {
        match call_tool(
            &bare_ctx(),
            "fleet_remove",
            &json!({ "app_id": "a_1", "confirm_name": "  " }),
        ) {
            Dispatch::Invalid(message) => assert!(message.contains("confirm_name")),
            Dispatch::Envelope(_) => panic!("a blank confirm_name must be rejected"),
        }
    }

    #[test]
    fn unknown_tool_is_rejected() {
        match call_tool(&bare_ctx(), "definitely_not_a_tool", &json!({})) {
            Dispatch::Invalid(message) => assert!(message.contains("unknown tool")),
            Dispatch::Envelope(_) => panic!("an unknown tool must be rejected"),
        }
    }

    #[test]
    fn unauthenticated_tool_call_instructs_login() {
        // base_url present, no stored token: the server started, and every tool
        // call returns a structured login instruction (spec 005 §1).
        let ctx = ctx(Some("http://localhost:4000"), None);
        match call_tool(&ctx, "tenants_list", &json!({})) {
            Dispatch::Envelope(envelope) => {
                assert_eq!(envelope["ok"], false);
                assert_eq!(envelope["error"]["kind"], "unauthenticated");
                assert!(envelope["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("stagecraft login"));
            }
            Dispatch::Invalid(_) => {
                panic!("an unauthenticated call is a tool result, not invalid params")
            }
        }
    }

    #[test]
    fn missing_base_url_is_a_config_tool_error() {
        let ctx = ctx(None, Some("tok"));
        match call_tool(&ctx, "tenants_list", &json!({})) {
            Dispatch::Envelope(envelope) => {
                assert_eq!(envelope["ok"], false);
                assert_eq!(envelope["error"]["kind"], "config");
            }
            Dispatch::Invalid(_) => {
                panic!("a missing base URL is a tool result, not invalid params")
            }
        }
    }

    #[test]
    fn round_trip_initialize_list_call_lists_tenants() {
        // The spec 005 §2 acceptance: initialize -> tools/list -> tools/call.
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants");
            then.status(200)
                .json_body(json!({ "tenants": [{ "id": "t_1", "name": "Acme" }] }));
        });
        let out = drive(
            &authed_ctx(&server.base_url()),
            &[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
                json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
                json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                        "params": { "name": "tenants_list", "arguments": {} } }),
            ],
        );
        mock.assert();
        // The notification produced no response: three ids in, three out.
        assert_eq!(out.len(), 3);
        let call = out.iter().find(|r| r["id"] == 3).unwrap();
        assert_eq!(call["result"]["isError"], false);
        assert_eq!(call["result"]["structuredContent"]["ok"], true);
        assert_eq!(
            call["result"]["structuredContent"]["data"]["tenants"][0]["id"],
            "t_1"
        );
        let text = call["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("t_1"), "the text content carries the record");
    }

    #[test]
    fn stamp_new_forwards_posture_and_passes_through_record_ids() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/tenants/t_1/stamps")
                .json_body(json!({
                    "appName": "smoke", "targetOrg": "acme-inc", "posture": "assisted"
                }));
            then.status(200).json_body(json!({
                "id": "j_1", "status": "queued", "attestationId": "att_9"
            }));
        });
        let out = drive(
            &authed_ctx(&server.base_url()),
            &[json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {
                    "name": "stamp_new",
                    "arguments": {
                        "tenant_id": "t_1", "app_name": "smoke",
                        "org": "acme-inc", "posture": "assisted"
                    }
                }
            })],
        );
        mock.assert();
        let result = &out[0]["result"];
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["data"]["status"], "queued");
        // Every tool result carries the plane's record ids when present (§1).
        assert_eq!(
            result["structuredContent"]["data"]["attestationId"],
            "att_9"
        );
    }

    #[test]
    fn stamp_new_missing_posture_is_invalid_params_over_the_wire() {
        let out = drive(
            &bare_ctx(),
            &[json!({
                "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": "stamp_new",
                            "arguments": { "tenant_id": "t_1", "app_name": "x", "org": "o" } }
            })],
        );
        assert_eq!(out[0]["id"], 7);
        assert_eq!(out[0]["error"]["code"], INVALID_PARAMS);
        assert!(out[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("posture"));
    }

    #[test]
    fn fleet_remove_forwards_confirm_name_in_the_body() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(DELETE)
                .path("/api/v1/fleet/a_1")
                .json_body(json!({ "confirm": "smoke" }));
            then.status(200)
                .json_body(json!({ "id": "a_1", "name": "smoke", "status": "removed" }));
        });
        let out = drive(
            &authed_ctx(&server.base_url()),
            &[json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "fleet_remove",
                            "arguments": { "app_id": "a_1", "confirm_name": "smoke" } }
            })],
        );
        mock.assert();
        assert_eq!(
            out[0]["result"]["structuredContent"]["data"]["status"],
            "removed"
        );
    }

    #[test]
    fn api_error_becomes_an_iserror_tool_result() {
        // A missing service (404) is an operational tool error, not a protocol
        // error: isError true, the envelope explaining it (spec 004 §1).
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/tenants");
            then.status(404);
        });
        let out = drive(
            &authed_ctx(&server.base_url()),
            &[json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "tenants_list", "arguments": {} }
            })],
        );
        let result = &out[0]["result"];
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["ok"], false);
        assert_eq!(result["structuredContent"]["error"]["kind"], "api");
        assert_eq!(result["structuredContent"]["error"]["status"], 404);
    }

    #[test]
    fn ping_returns_an_empty_result() {
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "id": 5, "method": "ping" })],
        );
        assert_eq!(out[0]["id"], 5);
        assert!(out[0]["result"].is_object());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "id": 9, "method": "frobnicate" })],
        );
        assert_eq!(out[0]["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn notifications_get_no_response() {
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })],
        );
        assert!(out.is_empty(), "a notification is answered with silence");
    }

    #[test]
    fn malformed_json_is_a_parse_error_with_null_id() {
        let mut output: Vec<u8> = Vec::new();
        serve(io::Cursor::new("{not json}\n"), &mut output, &bare_ctx()).unwrap();
        let resp: Value = serde_json::from_str(String::from_utf8(output).unwrap().trim()).unwrap();
        assert_eq!(resp["error"]["code"], PARSE_ERROR);
        assert!(resp["id"].is_null());
    }

    #[test]
    fn invalid_utf8_line_is_a_parse_error_and_the_server_survives() {
        // A bad byte on one line must not kill the server: it answers a parse
        // error and still serves the next, valid request (the trailing ping).
        let mut input: Vec<u8> = vec![0xff, 0xfe, b' ', b'x', b'\n'];
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        input.push(b'\n');
        let mut output: Vec<u8> = Vec::new();
        serve(io::Cursor::new(input), &mut output, &bare_ctx()).unwrap();
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            lines.len(),
            2,
            "the bad line is answered, the ping still served"
        );
        assert_eq!(lines[0]["error"]["code"], PARSE_ERROR);
        assert!(lines[0]["id"].is_null());
        assert_eq!(lines[1]["id"], 1);
        assert!(lines[1]["result"].is_object());
    }

    #[test]
    fn non_object_and_non_string_method_are_invalid_requests() {
        // Valid JSON that is not a request object: -32600, not -32700.
        let out = drive(&bare_ctx(), &[json!([1, 2, 3])]);
        assert_eq!(out[0]["error"]["code"], INVALID_REQUEST);
        // A request whose `method` is the wrong type is also an invalid request.
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "id": 3, "method": 123 })],
        );
        assert_eq!(out[0]["error"]["code"], INVALID_REQUEST);
        assert_eq!(out[0]["id"], 3);
    }

    #[test]
    fn an_explicit_null_id_is_a_request_not_a_notification() {
        // Strict JSON-RPC 2.0: an explicit `id: null` is a request expecting a
        // null-id response, distinct from a notification (an absent id).
        let out = drive(
            &bare_ctx(),
            &[json!({ "jsonrpc": "2.0", "id": null, "method": "ping" })],
        );
        assert_eq!(out.len(), 1, "an explicit null id gets a response");
        assert!(out[0]["id"].is_null());
        assert!(out[0]["result"].is_object());
    }

    #[test]
    fn shutdown_answers_then_stops_the_loop() {
        // The ping after shutdown must never be answered: the loop has stopped.
        let input = format!(
            "{}\n{}\n",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "shutdown" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" })
        );
        let mut output: Vec<u8> = Vec::new();
        serve(io::Cursor::new(input), &mut output, &bare_ctx()).unwrap();
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 1, "only shutdown is answered");
        assert_eq!(lines[0]["id"], 1);
    }

    #[test]
    fn print_config_snippet_carries_mcp_args_and_optional_env() {
        let with = config_snippet(Some("http://localhost:4000"), "/usr/local/bin/stagecraft");
        let entry = &with["mcpServers"]["stagecraft"];
        assert_eq!(entry["command"], "/usr/local/bin/stagecraft");
        assert_eq!(entry["args"][0], "mcp");
        assert_eq!(entry["env"]["STAGECRAFT_BASE_URL"], "http://localhost:4000");

        // No base URL resolved: no env block to hardcode.
        let without = config_snippet(None, "stagecraft");
        assert!(without["mcpServers"]["stagecraft"].get("env").is_none());
    }
}
