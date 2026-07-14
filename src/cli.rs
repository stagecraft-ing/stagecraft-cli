//! The `stagecraft` command tree (spec 002 §2).
//!
//! Stub verbs are present from day one so `--help` is honest: each carries
//! about-text naming the spec that implements it, and its handler exits 2.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    /// List tenants (spec 004).
    List,
    /// Show one tenant (spec 004).
    Show,
}

#[derive(Debug, Subcommand)]
pub enum StampCommand {
    /// Request a new governance stamp (spec 004).
    New,
    /// Check stamp status (spec 004).
    Status,
}

#[derive(Debug, Subcommand)]
pub enum FleetCommand {
    /// List fleet members (spec 004).
    List,
    /// Deploy to the fleet (spec 004).
    Deploy,
    /// Update fleet members (spec 004).
    Update,
    /// Back up fleet state (spec 004).
    Backup,
    /// Remove a fleet member (spec 004).
    Remove,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the effective, merged configuration with sources annotated.
    Show,
}
