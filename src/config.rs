//! Layered configuration (spec 002 §2): flags > env > config file > default.
//!
//! Resolution is a pure function over three raw layers so precedence is
//! unit-testable without touching the filesystem or the process environment.
//! `config show` reports the effective value of each field with its source.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::output::OutputFormat;

/// Environment-variable prefix for config overrides.
pub const ENV_PREFIX: &str = "STATECRAFT_";

/// Where a resolved value came from, highest precedence last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Default,
    File,
    Env,
    Flag,
}

impl Source {
    /// Human-facing label for `config show`.
    pub fn label(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::File => "config file",
            Source::Env => "env",
            Source::Flag => "flag",
        }
    }
}

/// A resolved value together with the layer it came from.
#[derive(Clone, Debug, Serialize)]
pub struct Sourced<T> {
    pub value: T,
    pub source: Source,
}

/// Config as parsed from `config.toml`. Every field optional.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub base_url: Option<String>,
    pub output: Option<OutputFormat>,
}

/// Config drawn from the environment (`STATECRAFT_*`). Every field optional.
#[derive(Clone, Debug, Default)]
pub struct EnvConfig {
    pub base_url: Option<String>,
    pub output: Option<OutputFormat>,
}

/// Config drawn from command-line flags. Every field optional.
#[derive(Clone, Debug, Default)]
pub struct FlagConfig {
    pub base_url: Option<String>,
    pub output: Option<OutputFormat>,
}

/// The merged configuration with per-field provenance for `config show`.
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedConfig {
    pub base_url: Sourced<Option<String>>,
    pub output: Sourced<OutputFormat>,
}

impl ResolvedConfig {
    /// The effective output format.
    pub fn output_format(&self) -> OutputFormat {
        self.output.value
    }
}

/// Merge the three raw layers into the effective config, recording provenance.
///
/// Precedence, highest first: flag, env, file, built-in default.
pub fn resolve(file: FileConfig, env: EnvConfig, flags: FlagConfig) -> ResolvedConfig {
    ResolvedConfig {
        base_url: pick_opt(flags.base_url, env.base_url, file.base_url),
        output: pick(
            flags.output,
            env.output,
            file.output,
            OutputFormat::default(),
        ),
    }
}

/// Pick the highest-precedence present value, falling back to `default`.
fn pick<T>(flag: Option<T>, env: Option<T>, file: Option<T>, default: T) -> Sourced<T> {
    if let Some(value) = flag {
        Sourced {
            value,
            source: Source::Flag,
        }
    } else if let Some(value) = env {
        Sourced {
            value,
            source: Source::Env,
        }
    } else if let Some(value) = file {
        Sourced {
            value,
            source: Source::File,
        }
    } else {
        Sourced {
            value: default,
            source: Source::Default,
        }
    }
}

/// Like [`pick`], but the field is itself optional (absent stays `None`).
fn pick_opt(
    flag: Option<String>,
    env: Option<String>,
    file: Option<String>,
) -> Sourced<Option<String>> {
    if let Some(value) = flag {
        Sourced {
            value: Some(value),
            source: Source::Flag,
        }
    } else if let Some(value) = env {
        Sourced {
            value: Some(value),
            source: Source::Env,
        }
    } else if let Some(value) = file {
        Sourced {
            value: Some(value),
            source: Source::File,
        }
    } else {
        Sourced {
            value: None,
            source: Source::Default,
        }
    }
}

/// The default config file path (`~/.config/statecraft/config.toml` on Linux),
/// derived via the `directories` crate. `None` if no home directory is known.
pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "statecraft")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Load and parse the config file. A missing file is not an error (empty config).
pub fn load_file(path: &Path) -> Result<FileConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("parsing config file {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(err) => Err(err).with_context(|| format!("reading config file {}", path.display())),
    }
}

/// Read config overrides from the process environment (`STATECRAFT_*`).
///
/// Empty values are treated as unset. An unrecognized `STATECRAFT_OUTPUT`
/// is misuse (exit 2), surfaced as [`AppError::Usage`].
pub fn load_env() -> AppResult<EnvConfig> {
    let base_url = non_empty_var(&format!("{ENV_PREFIX}BASE_URL"));
    let output = match non_empty_var(&format!("{ENV_PREFIX}OUTPUT")) {
        Some(raw) => Some(OutputFormat::parse_token(&raw).ok_or_else(|| {
            AppError::Usage(format!(
                "{ENV_PREFIX}OUTPUT is `{raw}`; expected `human` or `json`"
            ))
        })?),
        None => None,
    };
    Ok(EnvConfig { base_url, output })
}

/// Read an env var, treating unset or empty as absent.
fn non_empty_var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(base: Option<&str>, out: Option<OutputFormat>) -> FileConfig {
        FileConfig {
            base_url: base.map(String::from),
            output: out,
        }
    }
    fn env(base: Option<&str>, out: Option<OutputFormat>) -> EnvConfig {
        EnvConfig {
            base_url: base.map(String::from),
            output: out,
        }
    }
    fn flags(base: Option<&str>, out: Option<OutputFormat>) -> FlagConfig {
        FlagConfig {
            base_url: base.map(String::from),
            output: out,
        }
    }

    #[test]
    fn default_when_no_layer_sets_a_value() {
        let r = resolve(file(None, None), env(None, None), flags(None, None));
        assert_eq!(r.base_url.value, None);
        assert_eq!(r.base_url.source, Source::Default);
        assert_eq!(r.output.value, OutputFormat::Human);
        assert_eq!(r.output.source, Source::Default);
    }

    #[test]
    fn file_beats_default() {
        let r = resolve(
            file(Some("https://file.example"), Some(OutputFormat::Json)),
            env(None, None),
            flags(None, None),
        );
        assert_eq!(r.base_url.value.as_deref(), Some("https://file.example"));
        assert_eq!(r.base_url.source, Source::File);
        assert_eq!(r.output.value, OutputFormat::Json);
        assert_eq!(r.output.source, Source::File);
    }

    #[test]
    fn env_beats_file() {
        let r = resolve(
            file(Some("https://file.example"), Some(OutputFormat::Human)),
            env(Some("https://env.example"), Some(OutputFormat::Json)),
            flags(None, None),
        );
        assert_eq!(r.base_url.value.as_deref(), Some("https://env.example"));
        assert_eq!(r.base_url.source, Source::Env);
        assert_eq!(r.output.value, OutputFormat::Json);
        assert_eq!(r.output.source, Source::Env);
    }

    #[test]
    fn flag_beats_env_and_file() {
        let r = resolve(
            file(Some("https://file.example"), Some(OutputFormat::Human)),
            env(Some("https://env.example"), Some(OutputFormat::Human)),
            flags(Some("https://flag.example"), Some(OutputFormat::Json)),
        );
        assert_eq!(r.base_url.value.as_deref(), Some("https://flag.example"));
        assert_eq!(r.base_url.source, Source::Flag);
        assert_eq!(r.output.value, OutputFormat::Json);
        assert_eq!(r.output.source, Source::Flag);
    }

    #[test]
    fn layers_are_resolved_per_field_independently() {
        // base_url from the file, output from a flag: sources do not have to agree.
        let r = resolve(
            file(Some("https://file.example"), None),
            env(None, None),
            flags(None, Some(OutputFormat::Json)),
        );
        assert_eq!(r.base_url.source, Source::File);
        assert_eq!(r.output.source, Source::Flag);
    }

    #[test]
    fn file_parses_known_keys_and_rejects_unknown() {
        let ok: FileConfig = toml::from_str("base_url = \"https://x\"\noutput = \"json\"").unwrap();
        assert_eq!(ok.base_url.as_deref(), Some("https://x"));
        assert_eq!(ok.output, Some(OutputFormat::Json));
        assert!(toml::from_str::<FileConfig>("nope = 1").is_err());
    }
}
