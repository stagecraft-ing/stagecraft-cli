//! Process error taxonomy and exit-code contract (spec 002 §2).
//!
//! Exit codes are product surface: `0` ok, `1` operational failure,
//! `2` usage / not-implemented. Errors print to stderr, never stdout.

use thiserror::Error;

/// Command succeeded.
pub const EXIT_OK: u8 = 0;
/// A governed operation failed at runtime (I/O, parse, later: network).
pub const EXIT_OPERATIONAL: u8 = 1;
/// Misuse: bad flags, or a verb whose implementation lands in a later spec.
pub const EXIT_USAGE: u8 = 2;

/// The failure modes a command handler can surface to `main`.
#[derive(Debug, Error)]
pub enum AppError {
    /// A stub verb whose real implementation arrives in a later spec. This is
    /// the "not-implemented" arm of the spec 002 §2 exit-2 taxonomy. With spec
    /// 005 the last stub (`mcp`) is implemented, so no verb currently constructs
    /// it; it is retained because the spec mandates the taxonomy and a later
    /// spec scaffolds its command as a stub before implementing it.
    #[allow(dead_code)]
    #[error("`stagecraft {command}` is not implemented until spec {spec}")]
    NotImplemented { command: String, spec: &'static str },

    /// Invalid usage we detect ourselves (as opposed to clap's own parse errors).
    #[error("{0}")]
    Usage(String),

    /// A failure already written to stdout as the JSON `{ok:false,error}`
    /// envelope (spec 004 §5.2). `main` exits with `code` and prints nothing
    /// more, so the envelope is not doubled by a stderr line.
    #[error("")]
    Rendered { code: u8 },

    /// Any operational failure; carries anyhow context up to `main`.
    #[error(transparent)]
    Operational(#[from] anyhow::Error),
}

impl AppError {
    /// The process exit code this error maps to.
    pub fn code(&self) -> u8 {
        match self {
            AppError::NotImplemented { .. } | AppError::Usage(_) => EXIT_USAGE,
            AppError::Rendered { code } => *code,
            AppError::Operational(_) => EXIT_OPERATIONAL,
        }
    }

    /// Construct the stub error for a verb owned by a later spec (see
    /// [`AppError::NotImplemented`]; currently unconstructed by any verb).
    #[allow(dead_code)]
    pub fn not_implemented(command: impl Into<String>, spec: &'static str) -> Self {
        AppError::NotImplemented {
            command: command.into(),
            spec,
        }
    }
}

/// Result alias for command handlers.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_implemented_maps_to_usage_exit_code() {
        let err = AppError::not_implemented("login", "003-auth-api-client");
        assert_eq!(err.code(), EXIT_USAGE);
    }

    #[test]
    fn not_implemented_message_names_the_owning_spec() {
        let err = AppError::not_implemented("tenants list", "004-governance-verbs");
        assert!(err.to_string().contains("004-governance-verbs"));
        assert!(err.to_string().contains("tenants list"));
    }

    #[test]
    fn operational_maps_to_operational_exit_code() {
        let err: AppError = anyhow::anyhow!("disk gone").into();
        assert_eq!(err.code(), EXIT_OPERATIONAL);
    }
}
