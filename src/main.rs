//! `stagecraft`: one binary, two faces (spec 001). This crate is the scaffold
//! (spec 002); auth (003), governance verbs (004), and the MCP server (005)
//! hang off the command tree established here.

mod cli;
mod commands;
mod config;
mod error;
mod output;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;
use crate::error::EXIT_OK;

fn main() -> ExitCode {
    // clap handles `--help`/`--version` and its own usage errors (exit 2)
    // by terminating the process before returning here.
    let cli = Cli::parse();

    match commands::dispatch(cli) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.code())
        }
    }
}
