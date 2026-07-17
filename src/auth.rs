//! Authentication and the on-disk credentials store (spec 003 §2).
//!
//! `statecraft login` performs the browser-assisted bearer-token handoff
//! chosen in the spec's 2026-07-14 amendment: the operator signs in through a
//! browser at the control plane, pastes the resulting session token (piped or
//! at a prompt, so a headless machine with a browser elsewhere works), the CLI
//! validates it against `/auth/me`, and stores it. Tokens live in
//! `~/.config/statecraft/credentials.toml` at mode 0600, keyed by base URL,
//! never in the config file. The stored token is replayed as a bearer token by
//! the api module; the acquisition path can later become the RFC 8252 loopback
//! flow with no change to this store.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::{self, ApiClient, ApiError};
use crate::config::ResolvedConfig;
use crate::error::{AppError, AppResult};
use crate::output::{self, OutputFormat};

/// The credentials file: bearer tokens keyed by control-plane base URL.
/// Multiple planes may be present; each key is a normalized base URL.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub planes: BTreeMap<String, PlaneCredential>,
}

/// One plane's stored secret. A struct (not a bare string) so future fields
/// (issued-at, refresh token) extend the file without breaking the shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaneCredential {
    pub token: String,
}

/// The credentials file path: `~/.config/statecraft/credentials.toml` on Linux,
/// alongside `config.toml` but holding only secrets. `None` without a home dir.
pub fn default_credentials_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "statecraft")
        .map(|dirs| dirs.config_dir().join("credentials.toml"))
}

/// Load the credentials file from the platform default path (empty if absent).
pub fn load() -> Result<Credentials> {
    match default_credentials_path() {
        Some(path) => load_from(&path),
        None => Ok(Credentials::default()),
    }
}

/// Load and parse a credentials file. A missing file is not an error.
pub fn load_from(path: &Path) -> Result<Credentials> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .with_context(|| format!("parsing credentials file {}", path.display())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Credentials::default()),
        Err(err) => {
            Err(err).with_context(|| format!("reading credentials file {}", path.display()))
        }
    }
}

/// The stored token for `base_url`, if any. `base_url` must be normalized (the
/// api module's `normalize_base_url`) so it matches the key `login` wrote.
pub fn load_token(base_url: &str) -> AppResult<Option<String>> {
    let creds = load().map_err(AppError::Operational)?;
    Ok(creds.planes.get(base_url).map(|c| c.token.clone()))
}

/// Persist credentials to the platform default path (mode 0600), returning it.
pub fn save(creds: &Credentials) -> Result<PathBuf> {
    let path = default_credentials_path()
        .context("no home directory: cannot locate the credentials file")?;
    save_to(&path, creds)?;
    Ok(path)
}

/// Serialize and write credentials to `path` with private (0600) permissions,
/// creating parent directories as needed.
pub fn save_to(path: &Path, creds: &Credentials) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(creds).context("serializing credentials")?;
    write_private(path, &text)
}

/// Write `text` to `path` restricted to the owner (0600). Writes to a sibling
/// temp file, then renames over the target, so a crash cannot truncate an
/// existing file. The secret is never briefly world-readable: the mode is
/// forced to 0600 *before* any bytes are written, even if a stale temp file
/// from an earlier crash pre-existed (where `OpenOptions::mode` is a no-op
/// because it only applies on creation).
#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let tmp = {
        let mut raw = path.as_os_str().to_owned();
        raw.push(".tmp");
        PathBuf::from(raw)
    };
    // Clear any stale temp so the create below is a real creation and `mode` is
    // honored atomically; ignore a missing file.
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("clearing stale {}", tmp.display())),
    }
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        // Belt and suspenders against a race that recreated the temp wider:
        // tighten before the token is written, not after.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", tmp.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("writing {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} into place", tmp.display()))?;
    Ok(())
}

/// Non-Unix fallback: 0600 is a Unix concept; write the file best-effort.
#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// The `login` verb: the browser-assisted bearer-token handoff (spec 003 §2
/// amendment). Read a token, validate it against the plane, then store it.
pub fn run_login(resolved: &ResolvedConfig, format: OutputFormat, debug: bool) -> AppResult<()> {
    let base_url = api::require_base_url(resolved)?;
    let token = read_token(&base_url)?;
    if token.is_empty() {
        // A required input the operator failed to supply: usage (exit 2), like a
        // missing argument clap would reject. This differs from `whoami` finding
        // no stored credential, which is an operational state (exit 1): there the
        // command was invoked correctly but the plane precondition is unmet.
        return Err(AppError::Usage("no token was provided".to_string()));
    }

    // Validate before persisting: a token that cannot name its owner is not
    // worth storing. A 401 here means the paste was wrong or expired.
    let client = ApiClient::new(base_url.clone(), Some(token.clone()), debug)?;
    let identity = match api::block_on(client.fetch_identity())? {
        Ok(identity) => identity,
        Err(ApiError::Unauthenticated) => {
            return Err(AppError::Operational(anyhow::anyhow!(
                "the control plane rejected that token (it may be expired or for a different plane)"
            )))
        }
        Err(other) => return Err(other.into()),
    };

    let mut creds = load().map_err(AppError::Operational)?;
    creds
        .planes
        .insert(base_url.clone(), PlaneCredential { token });
    let path = save(&creds).map_err(AppError::Operational)?;

    let result = LoginResult {
        base_url,
        email: identity.email,
        credentials_path: path.display().to_string(),
    };
    output::emit(format, &result, || {
        format!(
            "Logged in to {} as {} (credentials: {})",
            result.base_url, result.email, result.credentials_path
        )
    });
    Ok(())
}

/// The `login` confirmation payload (stable JSON under `--output json`).
#[derive(Debug, Serialize)]
struct LoginResult {
    base_url: String,
    email: String,
    credentials_path: String,
}

/// Read the token: from a piped stdin when not a TTY, else an interactive
/// prompt. Guidance and the prompt go to stderr so stdout stays clean for
/// `--output json`. The token is read as a single trimmed line.
///
/// v1 does not suppress terminal echo on the interactive path: the operator is
/// pasting a token already visible in their browser devtools, and masking would
/// add a dependency. Hardening credential entry (echo-off, OS keychain) is
/// deferred alongside the keychain work the spec puts out of scope.
fn read_token(base_url: &str) -> AppResult<String> {
    use std::io::IsTerminal;

    if io::stdin().is_terminal() {
        eprintln!("Sign in at {base_url} in your browser, then paste your session token.");
        eprint!("Token: ");
        io::stderr().flush().ok();
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|e| AppError::Operational(e.into()))?;
        Ok(line.trim().to_string())
    } else {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| AppError::Operational(e.into()))?;
        Ok(buf.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch path per test so parallel runs never collide.
    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("statecraft-cred-{}-{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("credentials.toml")
    }

    #[test]
    fn round_trip_preserves_tokens_per_plane() {
        let path = scratch("roundtrip");
        let mut creds = Credentials::default();
        creds.planes.insert(
            "http://localhost:4000".to_string(),
            PlaneCredential {
                token: "tok-a".to_string(),
            },
        );
        creds.planes.insert(
            "https://plane.example".to_string(),
            PlaneCredential {
                token: "tok-b".to_string(),
            },
        );
        save_to(&path, &creds).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(
            loaded.planes.get("http://localhost:4000").unwrap().token,
            "tok-a"
        );
        assert_eq!(
            loaded.planes.get("https://plane.example").unwrap().token,
            "tok-b"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("perms");
        let mut creds = Credentials::default();
        creds.planes.insert(
            "http://localhost:4000".to_string(),
            PlaneCredential {
                token: "secret".to_string(),
            },
        );
        save_to(&path, &creds).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "credentials file must be owner-only");
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn save_tightens_permissions_on_a_prewidened_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("widen");
        std::fs::write(&path, "planes = {}\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        save_to(&path, &Credentials::default()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn save_over_a_stale_wide_temp_file_stays_0600() {
        // A crash can leave `credentials.toml.tmp` behind with wide bits. The
        // next save must not write the token into that file while it is still
        // readable: the final file (and the temp on the way) must be 0600.
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("stale-tmp");
        let tmp = {
            let mut raw = path.as_os_str().to_owned();
            raw.push(".tmp");
            PathBuf::from(raw)
        };
        std::fs::write(&tmp, "leftover").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o666)).unwrap();

        let mut creds = Credentials::default();
        creds.planes.insert(
            "http://localhost:4000".to_string(),
            PlaneCredential {
                token: "secret".to_string(),
            },
        );
        save_to(&path, &creds).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a stale wide temp must not widen the result");
        assert!(!tmp.exists(), "the temp file is renamed into place");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let path = std::env::temp_dir()
            .join(format!("statecraft-cred-{}-absent", std::process::id()))
            .join("credentials.toml");
        let loaded = load_from(&path).unwrap();
        assert!(loaded.planes.is_empty());
    }
}
