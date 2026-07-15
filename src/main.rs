//! `stagecraft`: one binary, two faces (spec 001). This crate is the scaffold
//! (spec 002); auth (003), governance verbs (004), and the MCP server (005)
//! hang off the command tree established here.

mod api;
mod auth;
mod cli;
mod commands;
mod config;
mod error;
mod output;
mod verbs;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;
use crate::error::{AppError, EXIT_OK};

fn main() -> ExitCode {
    // clap handles `--help`/`--version` and its own usage errors (exit 2)
    // by terminating the process before returning here.
    let cli = Cli::parse();

    match commands::dispatch(cli) {
        Ok(()) => ExitCode::from(EXIT_OK),
        // A `Rendered` failure already wrote the JSON error envelope to stdout
        // (spec 004 §5.2); exit with its code without a second, stderr report.
        Err(AppError::Rendered { code }) => ExitCode::from(code),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.code())
        }
    }
}
