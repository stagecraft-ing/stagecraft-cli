//! Command dispatch and handlers (spec 002 §2).
//!
//! `mcp` is the last stub, returning [`AppError::NotImplemented`] (exit 2, spec
//! named) until spec 005. The governance verbs (spec 004) live in
//! [`crate::verbs`]; the local verbs (`version`, `config show`, `completions`)
//! render through the shared output layer so `--output json` is honored
//! uniformly.

use std::io;
use std::path::PathBuf;

use clap::CommandFactory;
use serde::Serialize;

use crate::cli::{
    Cli, Command, ConfigCommand, FleetCommand, StampCommand, TemplateCommand, TenantsCommand,
};
use crate::config::{self, FlagConfig, ResolvedConfig, Sourced};
use crate::error::AppResult;
use crate::output::{self, OutputFormat};

/// Resolve config, then run the selected command.
pub fn dispatch(cli: Cli) -> AppResult<()> {
    let resolved = load_config(&cli)?;
    let format = resolved.output_format();

    match &cli.command {
        Command::Login => crate::auth::run_login(&resolved, format, cli.debug),
        Command::Whoami => crate::api::run_whoami(&resolved, format, cli.debug),
        Command::Tenants { command } => tenants(command, &resolved, format, cli.debug),
        Command::Stamp { command } => stamp(command, &resolved, format, cli.debug),
        Command::Fleet { command } => fleet(command, &resolved, format, cli.debug),
        Command::Template { command } => template(command, format, cli.debug),
        Command::Mcp { print_config } => {
            if *print_config {
                crate::mcp::print_config(&resolved)
            } else {
                crate::mcp::run(&resolved, cli.debug)
            }
        }
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

// --- governance verbs (spec 004) -------------------------------------------

fn tenants(
    command: &TenantsCommand,
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
) -> AppResult<()> {
    use crate::verbs::tenants;
    match command {
        TenantsCommand::List => tenants::list(resolved, format, debug),
        TenantsCommand::Show { id } => tenants::show(resolved, format, debug, id),
        TenantsCommand::InstallUrl { id, open } => {
            tenants::install_url(resolved, format, debug, id, *open)
        }
    }
}

fn stamp(
    command: &StampCommand,
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
) -> AppResult<()> {
    use crate::verbs::stamp;
    match command {
        StampCommand::New {
            tenant,
            app,
            org,
            frontend,
            posture,
        } => stamp::new(
            resolved,
            format,
            debug,
            tenant,
            app,
            org,
            frontend.as_deref(),
            *posture,
        ),
        StampCommand::Status { job_id, watch } => {
            stamp::status(resolved, format, debug, job_id, *watch)
        }
    }
}

fn fleet(
    command: &FleetCommand,
    resolved: &ResolvedConfig,
    format: OutputFormat,
    debug: bool,
) -> AppResult<()> {
    use crate::verbs::fleet;
    match command {
        FleetCommand::List { tenant } => fleet::list(resolved, format, debug, tenant),
        FleetCommand::Deploy { tenant, app, image } => {
            fleet::deploy(resolved, format, debug, tenant, app, image)
        }
        FleetCommand::Update { app_id, image } => {
            fleet::update(resolved, format, debug, app_id, image)
        }
        FleetCommand::Backup { app_id } => fleet::backup(resolved, format, debug, app_id),
        FleetCommand::Remove { app_id, confirm } => {
            fleet::remove(resolved, format, debug, app_id, confirm)
        }
    }
}

// --- local governed verb (spec 006) ----------------------------------------

/// `template upgrade`: the one verb that never touches the control plane. It
/// operates on the stamped app checkout in the current working directory, so it
/// takes no `resolved` config (no base URL, no token); the app dir is cwd.
fn template(command: &TemplateCommand, format: OutputFormat, debug: bool) -> AppResult<()> {
    use crate::verbs::template;
    let dir = std::env::current_dir().map_err(|e| {
        crate::error::AppError::Operational(anyhow::anyhow!(
            "cannot determine the current directory: {e}"
        ))
    })?;
    match command {
        TemplateCommand::Upgrade {
            to,
            dry_run,
            no_branch,
        } => template::upgrade(&dir, to.as_deref(), *dry_run, *no_branch, format, debug),
    }
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
        name: "statecraft",
        version: env!("CARGO_PKG_VERSION"),
    };
    output::emit(format, &info, || format!("statecraft {}", info.version));
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
