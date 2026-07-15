//! The `stagecraft` command tree (spec 002 §2).
//!
//! Stub verbs are present from day one so `--help` is honest: each carries
//! about-text naming the spec that implements it, and its handler exits 2.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::output::OutputFormat;

/// Stagecraft: governance verbs for humans (CLI) and agents (MCP).
#[derive(Debug, Parser)]
#[command(
    name = "stagecraft",
    version,
    about = "Stagecraft governance verbs: CLI subcommands for humans, MCP server for agents.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Output format: human-readable text, or stable machine JSON.
    #[arg(long, global = true, value_enum, value_name = "FORMAT")]
    pub output: Option<OutputFormat>,

    /// Stagecraft control-plane base URL (overrides config / env).
    #[arg(long, global = true, value_name = "URL")]
    pub base_url: Option<String>,

    /// Path to the config file (defaults to the platform config directory).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Dump request/response metadata to stderr (never credential material).
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Top-level verbs. Stubs name their owning spec in about-text.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate against a Stagecraft control plane (spec 003).
    Login,
    /// Show the currently authenticated identity (spec 003).
    Whoami,
    /// Inspect tenants (spec 004).
    Tenants {
        #[command(subcommand)]
        command: TenantsCommand,
    },
    /// Governance stamps (spec 004).
    Stamp {
        #[command(subcommand)]
        command: StampCommand,
    },
    /// Fleet operations (spec 004).
    Fleet {
        #[command(subcommand)]
        command: FleetCommand,
    },
    /// Run the MCP server over stdio (spec 005).
    Mcp,
    /// Print version information.
    Version,
    /// Inspect configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Generate a shell completion script.
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum TenantsCommand {
    /// List the tenants you own (spec 004).
    List,
    /// Show one tenant, including its installations (spec 004).
    Show {
        /// Tenant id.
        id: String,
    },
    /// Print the GitHub App install URL for a tenant (spec 004).
    InstallUrl {
        /// Tenant id.
        id: String,
        /// Open the URL in the default browser instead of only printing it.
        #[arg(long)]
        open: bool,
    },
}

/// Governance posture for a stamp. Required with no default: the platform
/// rejects a defaulted posture, so the CLI never invents one (spec 004 §5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Posture {
    /// No autonomous action; every step is operator-driven.
    None,
    /// Agent may act with a human in the loop.
    Assisted,
    /// Agent may act autonomously within its guardrails.
    Autonomous,
}

impl Posture {
    /// The wire token sent to the control plane (`posture` field).
    pub fn as_wire(self) -> &'static str {
        match self {
            Posture::None => "none",
            Posture::Assisted => "assisted",
            Posture::Autonomous => "autonomous",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum StampCommand {
    /// Request a new governance stamp: born-green repo in a customer org (spec 004).
    New {
        /// Tenant id the stamp is charged to.
        #[arg(long)]
        tenant: String,
        /// Application name for the stamped repo.
        #[arg(long)]
        app: String,
        /// Target GitHub org the repo is created in.
        #[arg(long)]
        org: String,
        /// Optional frontend flavor slot (e.g. `vue`).
        #[arg(long)]
        frontend: Option<String>,
        /// Governance posture (required; the platform never defaults it).
        #[arg(long, value_enum)]
        posture: Posture,
    },
    /// Check stamp status; `--watch` streams until the job settles (spec 004).
    Status {
        /// Stamp job id.
        job_id: String,
        /// Poll until the job reaches green or failed, streaming state changes.
        #[arg(long)]
        watch: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum FleetCommand {
    /// List a tenant's fleet apps (spec 004).
    List {
        /// Tenant id whose fleet is listed.
        #[arg(long)]
        tenant: String,
    },
    /// Deploy an image as a new fleet app (spec 004).
    Deploy {
        /// Tenant id the app belongs to.
        #[arg(long)]
        tenant: String,
        /// Fleet app name.
        #[arg(long)]
        app: String,
        /// Container image reference to place.
        #[arg(long)]
        image: String,
    },
    /// Roll a fleet app to a new image (spec 004).
    Update {
        /// Fleet app id.
        app_id: String,
        /// New container image reference.
        #[arg(long)]
        image: String,
    },
    /// Back up a fleet app's volume (spec 004).
    Backup {
        /// Fleet app id.
        app_id: String,
    },
    /// Remove a fleet app; `--confirm <name>` must echo the app name (spec 004).
    Remove {
        /// Fleet app id.
        app_id: String,
        /// The app's literal name, echoed to authorize the teardown.
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the effective, merged configuration with sources annotated.
    Show,
}
