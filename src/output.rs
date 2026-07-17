//! Output discipline (spec 002 §2): one layer every command renders through.
//!
//! Human-readable text on a TTY; stable machine JSON with `--output json`.
//! The JSON shapes are the contract the MCP face (spec 005) and scripts
//! consume later, so they are treated as versioned API from the first verb.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// The rendering format, selected by `--output` / `STATECRAFT_OUTPUT` / config.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable text.
    #[default]
    Human,
    /// Stable machine JSON.
    Json,
}

impl OutputFormat {
    /// The canonical lowercase token for this format.
    pub fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Human => "human",
            OutputFormat::Json => "json",
        }
    }

    /// Parse a format from an env / string value; `None` if unrecognized.
    pub fn parse_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "human" => Some(OutputFormat::Human),
            "json" => Some(OutputFormat::Json),
            _ => None,
        }
    }
}

/// Render `value` to stdout: pretty JSON, or the `human` closure's text.
///
/// Serialization of our own owned payloads cannot fail in practice; a failure
/// here is a programming error, not an operational one, so it panics rather
/// than muddying the exit-code contract.
pub fn emit<T: Serialize>(format: OutputFormat, value: &T, human: impl FnOnce() -> String) {
    match format {
        OutputFormat::Json => {
            let rendered = serde_json::to_string_pretty(value)
                .expect("serializing an owned output payload cannot fail");
            println!("{rendered}");
        }
        OutputFormat::Human => println!("{}", human()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_is_case_insensitive() {
        assert_eq!(OutputFormat::parse_token("JSON"), Some(OutputFormat::Json));
        assert_eq!(
            OutputFormat::parse_token("  human "),
            Some(OutputFormat::Human)
        );
        assert_eq!(OutputFormat::parse_token("yaml"), None);
    }

    #[test]
    fn as_str_round_trips_through_parse() {
        for f in [OutputFormat::Human, OutputFormat::Json] {
            assert_eq!(OutputFormat::parse_token(f.as_str()), Some(f));
        }
    }
}
