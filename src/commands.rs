//! Command dispatch and handlers (spec 002 §2).
//!
//! Stub verbs return [`AppError::NotImplemented`] (exit 2, spec named). The
//! three working verbs (`version`, `config show`, `completions`) render through
//! the shared output layer so `--output json` is honored uniformly.

use std::io;
use std::path::PathBuf;

use clap::CommandFactory;
use serde::Serialize;

use crate::cli::{Cli, Command, ConfigCommand, FleetCommand, StampCommand, TenantsCommand};
use crate::config::{self, FlagConfig, ResolvedConfig, Sourced};
use crate::error::{AppError, AppResult};
use crate::output::{self, OutputFormat};

/// Resolve config, then run the selected command.
pub fn dispatch(cli: Cli) -> AppResult<()> {
    let resolved = load_config(&cli)?;
    let format = resolved.output_format();

    match &cli.command {
        Command::Login => Err(AppError::not_implemented("login", "003-auth-api-client")),
        Command::Whoami => Err(AppError::not_implemented("whoami", "003-auth-api-client")),
        Command::Tenants { command } => tenants(command),
        Command::Stamp { command } => stamp(command),
        Command::Fleet { command } => fleet(command),
        Command::Mcp => Err(AppError::not_implemented("mcp", "005-mcp-server")),
        Command::Version => {
            version(format);
            Ok(())
        }
        Command::Config { command } => config_cmd(command, &cli, &resolved),
        Command::Completions { shell } => {
            completions(*shell);
            Ok(())
        }
    }
}

/// Where the config file lives: explicit `--config`, else the platform default.
fn config_path(cli: &Cli) -> Option<PathBuf> {
    cli.config.clone().or_else(config::default_config_path)
}

/// Merge file + env + flag layers into the effective config.
fn load_config(cli: &Cli) -> AppResult<ResolvedConfig> {
    let file = match config_path(cli) {
        Some(path) => config::load_file(&path)?,
        None => config::FileConfig::default(),
    };
    let env = config::load_env()?;
    let flags = FlagConfig {
        base_url: cli.base_url.clone(),
        output: cli.output,
    };
    Ok(config::resolve(file, env, flags))
}

// --- stub verbs (spec 004) -------------------------------------------------

fn tenants(command: &TenantsCommand) -> AppResult<()> {
    let verb = match command {
        TenantsCommand::List => "list",
        TenantsCommand::Show => "show",
    };
    Err(AppError::not_implemented(
        format!("tenants {verb}"),
        "004-governance-verbs",
    ))
}

fn stamp(command: &StampCommand) -> AppResult<()> {
    let verb = match command {
        StampCommand::New => "new",
        StampCommand::Status => "status",
    };
    Err(AppError::not_implemented(
        format!("stamp {verb}"),
        "004-governance-verbs",
    ))
}

fn fleet(command: &FleetCommand) -> AppResult<()> {
    let verb = match command {
        FleetCommand::List => "list",
        FleetCommand::Deploy => "deploy",
        FleetCommand::Update => "update",
        FleetCommand::Backup => "backup",
        FleetCommand::Remove => "remove",
    };
    Err(AppError::not_implemented(
        format!("fleet {verb}"),
        "004-governance-verbs",
    ))
}

// --- working verbs ---------------------------------------------------------

/// Version payload; stable JSON shape (API from the first verb).
#[derive(Serialize)]
struct VersionInfo {
    name: &'static str,
    version: &'static str,
}

fn version(format: OutputFormat) {
    let info = VersionInfo {
        name: "stagecraft",
        version: env!("CARGO_PKG_VERSION"),
    };
    output::emit(format, &info, || format!("stagecraft {}", info.version));
}

/// Config-file location and existence, for `config show`.
#[derive(Serialize)]
struct ConfigFileInfo {
    path: Option<String>,
    exists: bool,
}

/// The `config show` payload: file location plus each field with its source.
#[derive(Serialize)]
struct ConfigShow<'a> {
    config_file: ConfigFileInfo,
    base_url: &'a Sourced<Option<String>>,
    output: &'a Sourced<OutputFormat>,
}

fn config_cmd(command: &ConfigCommand, cli: &Cli, resolved: &ResolvedConfig) -> AppResult<()> {
    match command {
        ConfigCommand::Show => {
            let path = config_path(cli);
            let payload = ConfigShow {
                config_file: ConfigFileInfo {
                    exists: path.as_ref().is_some_and(|p| p.exists()),
                    path: path.as_ref().map(|p| p.display().to_string()),
                },
                base_url: &resolved.base_url,
                output: &resolved.output,
            };
            output::emit(resolved.output_format(), &payload, || {
                render_config_human(&payload)
            });
            Ok(())
        }
    }
}

fn render_config_human(payload: &ConfigShow) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let base = payload
        .base_url
        .value
        .clone()
        .unwrap_or_else(|| "(unset)".to_string());
    let _ = writeln!(
        out,
        "base_url = {base}  # source: {}",
        payload.base_url.source.label()
    );
    let _ = writeln!(
        out,
        "output   = {}  # source: {}",
        payload.output.value.as_str(),
        payload.output.source.label()
    );
    match &payload.config_file.path {
        Some(path) => {
            let state = if payload.config_file.exists {
                "found"
            } else {
                "not found"
            };
            let _ = write!(out, "config file: {path} ({state})");
        }
        None => {
            let _ = write!(out, "config file: (no home directory)");
        }
    }
    out
}

fn completions(shell: clap_complete::Shell) {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut io::stdout());
}
