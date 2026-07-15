//! The governance verbs (spec 004): the CLI face over the control-plane API.
//!
//! Each verb resolves the base URL and stored token, calls one control-plane
//! endpoint through the spec 003 client, and renders the result two ways: the
//! stable `{ok, data|error}` JSON envelope for `--output json` (spec 004 §5.2,
//! the shared contract the MCP face reuses in spec 005), or an aligned human
//! table on a TTY. The JSON envelope is passthrough: the plane's payload is
//! wrapped, never reshaped, so the contract stays stable as fields grow.
//!
//! Submodules ([`tenants`], [`stamp`], [`fleet`]) hold one verb family each and
//! call the private helpers here directly (a child module sees its parent's
//! private items). [`template`] (spec 006) is the exception: a *local* verb that
//! never calls the control plane (it operates on a stamped app checkout), yet
//! still renders through the same `{ok,data|error}` envelope
//! ([`success_envelope_value`], [`error_envelope`]) so both faces stay uniform.

pub mod fleet;
pub mod stamp;
pub mod template;
pub mod tenants;

use serde::Serialize;
use serde_json::Value;

use crate::api::{self, ApiClient, ApiError};
use crate::config::ResolvedConfig;
use crate::error::{AppError, AppResult, EXIT_OPERATIONAL};
use crate::output::OutputFormat;

/// The success/failure envelope both faces consume (spec 004 §5.2). `data`
/// borrows the passthrough value; exactly one of `data`/`error` is present.
#[derive(Serialize)]
struct Envelope<'a> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorBody>,
}

/// The `error` arm: the spec 003 taxonomy projected onto stable JSON fields.
#[derive(Serialize)]
struct ErrorBody {
    kind: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
}

/// Resolve the base URL and stored token, then build an API client: the shared
/// preamble every verb runs (it mirrors `whoami`). A missing token is an
/// operational failure (exit 1) with the login hint, not a usage error: the
/// command was invoked correctly, the plane precondition is just unmet.
fn client_for(resolved: &ResolvedConfig, debug: bool) -> AppResult<ApiClient> {
    let base_url = api::require_base_url(resolved)?;
    let token = crate::auth::load_token(&base_url)?.ok_or_else(|| {
        AppError::Operational(anyhow::anyhow!(
            "not authenticated for {base_url}; run `stagecraft login`"
        ))
    })?;
    Ok(ApiClient::new(base_url, Some(token), debug)?)
}

/// Render a completed call: the passthrough envelope for JSON, a table for
/// human output. Delegates success to [`emit_ok`] and failure to [`emit_err`].
fn render(
    format: OutputFormat,
    result: Result<Value, ApiError>,
    human: impl FnOnce(&Value) -> AppResult<String>,
) -> AppResult<()> {
    match result {
        Ok(value) => emit_ok(format, &value, human),
        Err(err) => Err(emit_err(format, err)),
    }
}

/// Emit a success: the `{ok:true,data}` envelope in JSON mode, the `human`
/// closure's table on a TTY. The closure is never called in JSON mode, so the
/// passthrough value is serialized untouched.
fn emit_ok(
    format: OutputFormat,
    value: &Value,
    human: impl FnOnce(&Value) -> AppResult<String>,
) -> AppResult<()> {
    match format {
        OutputFormat::Json => {
            let env = Envelope {
                ok: true,
                data: Some(value),
                error: None,
            };
            println!("{}", to_pretty(&env));
            Ok(())
        }
        OutputFormat::Human => {
            println!("{}", human(value)?);
            Ok(())
        }
    }
}

/// Turn an API failure into the error the caller returns to `main`. In JSON
/// mode it writes the `{ok:false,error}` envelope to stdout and returns
/// [`AppError::Rendered`] so `main` exits 1 without a second stderr line; in
/// human mode it returns the taxonomy error for `main` to print to stderr.
fn emit_err(format: OutputFormat, err: ApiError) -> AppError {
    match format {
        OutputFormat::Json => {
            let env = Envelope {
                ok: false,
                data: None,
                error: Some(ErrorBody {
                    kind: err.kind(),
                    message: err.to_string(),
                    status: err.status(),
                }),
            };
            println!("{}", to_pretty(&env));
            AppError::Rendered {
                code: EXIT_OPERATIONAL,
            }
        }
        OutputFormat::Human => AppError::from(err),
    }
}

/// Pretty-print an owned envelope. Serializing our own payload cannot fail in
/// practice, so a failure is a programming error, not an operational one.
fn to_pretty<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serializing an owned envelope cannot fail")
}

/// Interpret a passthrough value as a JSON array (list endpoints), or a decode
/// error naming what came back instead. Human path only; JSON passthrough never
/// calls this.
fn as_array(v: &Value) -> AppResult<&Vec<Value>> {
    v.as_array().ok_or_else(|| {
        AppError::Operational(anyhow::anyhow!(
            "expected a JSON array from the control plane, got {}",
            kind_of(v)
        ))
    })
}

/// A human name for a JSON value's shape, for decode-error messages.
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// One optional field of a JSON object rendered for a table cell: strings
/// verbatim, numbers and booleans stringified, anything absent or null as empty.
fn field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// A required scalar field for human rendering. Absent or non-scalar means the
/// plane returned a record the CLI cannot read (spec 004 §5.3: stamp and fleet
/// records carry at least `id` and `status`): a decode error (exit 1) that
/// signals drift rather than rendering a silent blank.
fn require_field(v: &Value, key: &str) -> AppResult<String> {
    match v.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        _ => Err(AppError::Operational(anyhow::anyhow!(
            "unexpected response from the control plane: missing `{key}`"
        ))),
    }
}

/// Render an aligned text table: a header row over body rows, each column
/// padded to its widest cell (the last column is not padded). Callers handle
/// the empty case, so this is only given non-empty `rows`.
fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.chars().count());
            }
        }
    }
    let render_row = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let last = i + 1 == cells.len();
                let width = widths.get(i).copied().unwrap_or(0);
                if last {
                    cell.clone()
                } else {
                    format!("{cell:<width$}")
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    };

    let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    let mut lines = vec![render_row(&header_cells)];
    for row in rows {
        lines.push(render_row(row));
    }
    lines.join("\n")
}

/// Open `url` in the operator's default browser (the `install-url --open`
/// path). Best-effort and platform-specific; no new dependency is pulled in for
/// it (CLAUDE.md keeps the tree lean). The URL is already printed by the time
/// this runs, so a failed or non-zero launch warns on stderr rather than losing
/// the operator the URL or flipping the exit code.
fn open_in_browser(url: &str) {
    match browser_command(url).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("warning: browser launcher exited with status {status}"),
        Err(e) => eprintln!("warning: failed to launch a browser: {e}"),
    }
}

#[cfg(target_os = "macos")]
fn browser_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(url);
    command
}

#[cfg(target_os = "windows")]
fn browser_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    // The empty "" is `start`'s title argument; without it a quoted URL would be
    // taken as the window title.
    command.args(["/C", "start", "", url]);
    command
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn browser_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    command
}

/// Build the success envelope (`{ok:true,data}`) as an owned JSON value: the
/// same shape [`emit_ok`] prints for `--output json`, produced without touching
/// stdout. The MCP face (spec 005) returns it as the tool result; per-verb
/// snapshot tests use it to lock the shape.
pub(crate) fn success_envelope_value(data: &Value) -> Value {
    serde_json::to_value(Envelope {
        ok: true,
        data: Some(data),
        error: None,
    })
    .expect("serializing an owned envelope cannot fail")
}

/// Build the failure envelope (`{ok:false,error}`) as an owned JSON value from
/// an explicit taxonomy `kind`, message, and optional HTTP status. The MCP face
/// uses this for pre-request failures (no base URL, no stored credential) so its
/// tool-result errors carry the same `error` shape the API path emits.
pub(crate) fn error_envelope(kind: &'static str, message: String, status: Option<u16>) -> Value {
    serde_json::to_value(Envelope {
        ok: false,
        data: None,
        error: Some(ErrorBody {
            kind,
            message,
            status,
        }),
    })
    .expect("serializing an owned envelope cannot fail")
}

/// Wrap a completed verb request in the passthrough envelope: `{ok:true,data}`
/// on success, `{ok:false,error}` mapped from the taxonomy on failure. This is
/// the value the MCP tool result carries (spec 005 §1), byte-for-byte what
/// `--output json` prints for the CLI face (spec 004 §5.2).
pub(crate) fn envelope_value(result: Result<Value, ApiError>) -> Value {
    match result {
        Ok(value) => success_envelope_value(&value),
        Err(err) => error_envelope(err.kind(), err.to_string(), err.status()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn success_envelope_wraps_data_verbatim() {
        let value = json!({"id": "t_1", "name": "Acme", "extra": [1, 2, 3]});
        let env = Envelope {
            ok: true,
            data: Some(&value),
            error: None,
        };
        let rendered = serde_json::to_value(&env).unwrap();
        assert_eq!(rendered["ok"], true);
        assert_eq!(rendered["data"], value);
        assert!(rendered.get("error").is_none(), "no error arm on success");
    }

    #[test]
    fn error_envelope_carries_kind_and_status() {
        let err = ApiError::Api {
            status: 404,
            message: "tenants not enabled on this control plane".to_string(),
        };
        let env = Envelope {
            ok: false,
            data: None,
            error: Some(ErrorBody {
                kind: err.kind(),
                message: err.to_string(),
                status: err.status(),
            }),
        };
        let rendered = serde_json::to_value(&env).unwrap();
        assert_eq!(rendered["ok"], false);
        assert!(rendered.get("data").is_none(), "no data arm on failure");
        assert_eq!(rendered["error"]["kind"], "api");
        assert_eq!(rendered["error"]["status"], 404);
    }

    #[test]
    fn table_pads_columns_and_leaves_last_ragged() {
        let rows = vec![
            vec!["t_1".to_string(), "Acme".to_string()],
            vec!["t_22".to_string(), "Beta".to_string()],
        ];
        let out = table(&["ID", "NAME"], &rows);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "ID    NAME");
        assert_eq!(lines[1], "t_1   Acme");
        assert_eq!(lines[2], "t_22  Beta");
    }

    #[test]
    fn field_stringifies_scalars_and_blanks_the_rest() {
        let v = json!({"s": "x", "n": 42, "b": true, "nil": null, "arr": [1]});
        assert_eq!(field(&v, "s"), "x");
        assert_eq!(field(&v, "n"), "42");
        assert_eq!(field(&v, "b"), "true");
        assert_eq!(field(&v, "nil"), "");
        assert_eq!(field(&v, "arr"), "");
        assert_eq!(field(&v, "absent"), "");
    }
}
