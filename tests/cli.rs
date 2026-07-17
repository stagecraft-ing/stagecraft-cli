//! End-to-end checks that drive the built `statecraft` binary (spec 002 §3).
//!
//! Cargo sets `CARGO_BIN_EXE_statecraft`, so no extra test crates are needed.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_statecraft"))
        .args(args)
        // Neutralize ambient config so tests are deterministic.
        .env_remove("STATECRAFT_BASE_URL")
        .env_remove("STATECRAFT_OUTPUT")
        .output()
        .expect("failed to run statecraft binary")
}

/// Run the binary with its working directory set to `dir`: the `template`
/// verb operates on the stamped app in the current directory (spec 006).
fn run_in(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_statecraft"))
        .args(args)
        .current_dir(dir)
        .env_remove("STATECRAFT_BASE_URL")
        .env_remove("STATECRAFT_OUTPUT")
        .output()
        .expect("failed to run statecraft binary")
}

/// Create a unique temp dir seeded with a `template.toml` and `package.json`,
/// for the offline `template` refusals and the dry-run plan (no git/npm needed).
fn stamped_fixture(template_toml: &str, package_json: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("statecraft-cli-tmpl-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("template.toml"), template_toml).expect("write template.toml");
    std::fs::write(dir.join("package.json"), package_json).expect("write package.json");
    dir
}

/// Drive the binary with `input` piped to stdin, then close it. The `mcp` stdio
/// server reads newline-delimited requests and shuts down on the resulting EOF.
fn run_with_stdin(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_statecraft"))
        .args(args)
        .env_remove("STATECRAFT_BASE_URL")
        .env_remove("STATECRAFT_OUTPUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn statecraft binary");
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input.as_bytes())
        .expect("failed to write stdin");
    // The taken stdin drops here, closing the pipe: EOF the server treats as
    // shutdown, so the wait below returns rather than blocking.
    child
        .wait_with_output()
        .expect("failed to wait on statecraft binary")
}

#[test]
fn mcp_print_config_emits_an_installable_snippet() {
    // `mcp --print-config` is the install helper (spec 005 §1): a `.mcp.json`
    // snippet on stdout, exit 0. No stubs remain in the command tree.
    let out = run(&["mcp", "--print-config"]);
    assert_eq!(out.status.code(), Some(0), "print-config should exit 0");
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("print-config emits valid JSON");
    assert_eq!(value["mcpServers"]["statecraft"]["args"][0], "mcp");
    assert!(
        value["mcpServers"]["statecraft"]["command"].is_string(),
        "the snippet names a launch command, got: {value}"
    );
}

#[test]
fn mcp_server_answers_initialize_over_stdio() {
    // The real binary wires stdin -> the JSON-RPC loop -> stdout. Feed one
    // initialize request; the server replies, then shuts down on stdin EOF.
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let out = run_with_stdin(&["mcp"], &format!("{request}\n"));
    assert_eq!(out.status.code(), Some(0), "clean shutdown on stdin EOF");
    let line = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value =
        serde_json::from_str(line.trim()).expect("one JSON-RPC response line on stdout");
    assert_eq!(value["id"], 1);
    assert_eq!(value["result"]["serverInfo"]["name"], "statecraft");
}

#[test]
fn governance_verbs_without_base_url_are_usage_errors() {
    // The spec 004 verbs reach the base-URL guard before any network call;
    // omitting it is misuse (exit 2), same contract as login/whoami. Each is
    // given its required flags/args so clap does not reject it first.
    for args in [
        ["tenants", "list"].as_slice(),
        ["fleet", "list", "--tenant", "t_1"].as_slice(),
        ["stamp", "status", "j_1"].as_slice(),
    ] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("base URL"),
            "args {args:?}: stderr should name the missing base URL, got: {stderr}"
        );
        assert!(
            out.stdout.is_empty(),
            "args {args:?}: errors must not print to stdout"
        );
    }
}

#[test]
fn governance_verb_without_a_credential_exits_1_with_login_hint() {
    // With a base URL but no stored token, a spec 004 verb short-circuits to the
    // unauthenticated error before any network call (hermetic), like whoami.
    let out = run(&[
        "tenants",
        "list",
        "--base-url",
        "https://unconfigured.plane.invalid",
    ]);
    assert_eq!(out.status.code(), Some(1), "no credential is exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("statecraft login"),
        "stderr should hint at login, got: {stderr}"
    );
    assert!(out.stdout.is_empty(), "errors must not print to stdout");
}

#[test]
fn stamp_new_requires_a_posture() {
    // Posture is a required flag with no default (spec 004 §5.1): omitting it is
    // a clap usage error (exit 2), so the CLI never invents a posture.
    let out = run(&[
        "stamp", "new", "--tenant", "t_1", "--app", "x", "--org", "acme",
    ]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing --posture should exit 2"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("posture"),
        "stderr should name the missing posture, got: {stderr}"
    );
}

#[test]
fn auth_verbs_without_base_url_are_usage_errors() {
    // login and whoami need a control plane to talk to; omitting it is misuse
    // (exit 2), distinct from being logged out (exit 1, below).
    for args in [["login"].as_slice(), ["whoami"].as_slice()] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("base URL"),
            "args {args:?}: stderr should name the missing base URL, got: {stderr}"
        );
        assert!(
            out.stdout.is_empty(),
            "args {args:?}: stdout must stay clean"
        );
    }
}

#[test]
fn whoami_without_a_stored_credential_exits_1_with_login_hint() {
    // A base URL that no credentials file could hold: whoami short-circuits to
    // the unauthenticated error before any network call, so this is hermetic.
    let out = run(&["whoami", "--base-url", "https://unconfigured.plane.invalid"]);
    assert_eq!(out.status.code(), Some(1), "unauthenticated is exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("statecraft login"),
        "stderr should hint at login, got: {stderr}"
    );
    assert!(out.stdout.is_empty(), "errors must not print to stdout");
}

#[test]
fn help_lists_the_full_command_tree() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for verb in [
        "login",
        "whoami",
        "tenants",
        "stamp",
        "fleet",
        "template",
        "mcp",
        "version",
        "config",
        "completions",
    ] {
        assert!(stdout.contains(verb), "help missing `{verb}`: {stdout}");
    }
}

#[test]
fn version_json_shape_is_stable() {
    let out = run(&["--output", "json", "version"]);
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["name"], "statecraft");
    assert!(value["version"].is_string());
}

#[test]
fn completions_zsh_emits_a_script() {
    let out = run(&["completions", "zsh"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        !out.stdout.is_empty(),
        "completion script should not be empty"
    );
}

#[test]
fn config_show_reports_flag_as_the_base_url_source() {
    let out = run(&[
        "--output",
        "json",
        "--base-url",
        "https://flag.example",
        "config",
        "show",
    ]);
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["base_url"]["value"], "https://flag.example");
    assert_eq!(value["base_url"]["source"], "flag");
}

#[test]
fn invalid_env_output_value_is_a_usage_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_statecraft"))
        .args(["version"])
        .env_remove("STATECRAFT_BASE_URL")
        .env("STATECRAFT_OUTPUT", "yaml")
        .output()
        .expect("failed to run statecraft binary");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "usage errors must not print to stdout"
    );
}

#[test]
fn config_show_reports_env_as_the_base_url_source() {
    let out = Command::new(env!("CARGO_BIN_EXE_statecraft"))
        .args(["--output", "json", "config", "show"])
        .env_remove("STATECRAFT_OUTPUT")
        .env("STATECRAFT_BASE_URL", "https://env.example")
        .output()
        .expect("failed to run statecraft binary");
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["base_url"]["value"], "https://env.example");
    assert_eq!(value["base_url"]["source"], "env");
}

// --- template upgrade (spec 006), offline paths only -----------------------

const TOOLCHAIN_TOML: &str =
    "[template]\nname = \"enrahitu\"\nversion = \"0.1.0\"\n\n[requires]\ntoolchain = \"^0.1\"\n\n[verbs]\nverify = \"npm test\"\n";

#[test]
fn template_upgrade_not_stamped_is_a_refusal() {
    // An empty directory: no template.toml, so it is not a stamped app. The
    // JSON error envelope lands on stdout (spec 004 §5.2), exit 1.
    let dir = std::env::temp_dir().join(format!("statecraft-cli-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = run_in(
        &dir,
        &["--output", "json", "template", "upgrade", "--to", "0.1.5"],
    );
    assert_eq!(out.status.code(), Some(1), "not-a-stamped-app is exit 1");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["kind"], "not_stamped");
}

#[test]
fn template_upgrade_pre_018_app_is_refused() {
    // A stamped app whose package.json carries no chassis packages: pre-018.
    let dir = stamped_fixture(
        TOOLCHAIN_TOML,
        "{\n  \"name\": \"legacy\",\n  \"dependencies\": { \"left-pad\": \"^1.0.0\" }\n}\n",
    );
    let out = run_in(
        &dir,
        &["--output", "json", "template", "upgrade", "--to", "0.1.5"],
    );
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["error"]["kind"], "pre_chassis");
}

#[test]
fn template_upgrade_dry_run_plans_without_mutating() {
    // Dry run reaches no git/npm: it reads, plans, and reports. It must change
    // nothing on disk and exit 0 with the machine result.
    let package = "{\n  \"name\": \"app\",\n  \"devDependencies\": {\n    \"@enrahitu/toolchain\": \"0.1.0\"\n  }\n}\n";
    let dir = stamped_fixture(TOOLCHAIN_TOML, package);
    let out = run_in(
        &dir,
        &[
            "--output",
            "json",
            "template",
            "upgrade",
            "--to",
            "0.1.5",
            "--dry-run",
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["to"], "0.1.5");
    assert_eq!(value["data"]["dryRun"], true);
    // Byte-for-byte unchanged.
    assert_eq!(
        std::fs::read_to_string(dir.join("package.json")).unwrap(),
        package
    );
}
