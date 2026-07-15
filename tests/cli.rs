//! End-to-end checks that drive the built `stagecraft` binary (spec 002 §3).
//!
//! Cargo sets `CARGO_BIN_EXE_stagecraft`, so no extra test crates are needed.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stagecraft"))
        .args(args)
        // Neutralize ambient config so tests are deterministic.
        .env_remove("STAGECRAFT_BASE_URL")
        .env_remove("STAGECRAFT_OUTPUT")
        .output()
        .expect("failed to run stagecraft binary")
}

#[test]
fn stub_verb_exits_2_names_spec_and_keeps_stdout_clean() {
    // One representative stub per still-unimplemented owning spec. login and
    // whoami left this list when spec 003 implemented them.
    for (args, spec) in [
        (["tenants", "list"].as_slice(), "004"),
        (["stamp", "new"].as_slice(), "004"),
        (["fleet", "remove"].as_slice(), "004"),
        (["mcp"].as_slice(), "005"),
    ] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(spec),
            "args {args:?}: stderr should name spec {spec}, got: {stderr}"
        );
        assert!(
            out.stdout.is_empty(),
            "args {args:?}: errors must not print to stdout, got: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
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
        stderr.contains("stagecraft login"),
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
    assert_eq!(value["name"], "stagecraft");
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
    let out = Command::new(env!("CARGO_BIN_EXE_stagecraft"))
        .args(["version"])
        .env_remove("STAGECRAFT_BASE_URL")
        .env("STAGECRAFT_OUTPUT", "yaml")
        .output()
        .expect("failed to run stagecraft binary");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "usage errors must not print to stdout"
    );
}

#[test]
fn config_show_reports_env_as_the_base_url_source() {
    let out = Command::new(env!("CARGO_BIN_EXE_stagecraft"))
        .args(["--output", "json", "config", "show"])
        .env_remove("STAGECRAFT_OUTPUT")
        .env("STAGECRAFT_BASE_URL", "https://env.example")
        .output()
        .expect("failed to run stagecraft binary");
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(value["base_url"]["value"], "https://env.example");
    assert_eq!(value["base_url"]["source"], "env");
}
