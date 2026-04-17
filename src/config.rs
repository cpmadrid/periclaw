//! Token bootstrap and config paths.
//!
//! **Keyring first**: on subsequent runs the token is fetched from the
//! OS keychain (Keychain on macOS, Secret Service on Linux).
//!
//! **First-run bootstrap**: if keyring lookup fails, we read the token
//! from `~/.openclaw/openclaw.json` → `auth.token` and stash it in the
//! keyring for next time.
//!
//! **Headless Linux fallback**: if the keyring is unavailable (no
//! `secret-service` daemon on a bare ubu-3xdv wall-display install),
//! fall back to plaintext `$XDG_CONFIG_HOME/sassy-dog/token` with
//! mode 0600.

use std::fs;
use std::path::PathBuf;

use directories::{BaseDirs, UserDirs};
use serde::Deserialize;

const KEYRING_SERVICE: &str = "com.sassydog.mission-control-desktop";
const KEYRING_USER: &str = "openclaw-gateway-token";

pub const GATEWAY_URL: &str = "ws://100.87.202.125:18789/__openclaw__/ws";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no home directory available on this platform")]
    NoHome,
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse openclaw.json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("openclaw.json present but has no auth.token")]
    MissingToken,
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("file write error: {0}")]
    Io(#[from] std::io::Error),
}

/// Load the OpenClaw gateway token, bootstrapping to the keyring on first run.
///
/// Order of operations:
/// 1. If `OPENCLAW_TOKEN` env var is set, use it and seed the keyring.
/// 2. Try the keyring.
/// 3. Try the plaintext fallback file at `$XDG_CONFIG_HOME/sassy-dog/token`.
/// 4. Try reading from `~/.openclaw/openclaw.json` → `auth.token`
///    (and migrate into the keyring for next time).
pub fn load_token() -> Result<String, ConfigError> {
    if let Ok(tok) = std::env::var("OPENCLAW_TOKEN") {
        let tok = tok.trim().to_string();
        if !tok.is_empty() {
            tracing::info!("OPENCLAW_TOKEN env var provided; seeding keyring");
            if let Err(e) = stash_in_keyring(&tok) {
                tracing::warn!(error = %e, "keyring stash failed; writing plaintext fallback");
                write_plaintext_fallback(&tok)?;
            }
            return Ok(tok);
        }
    }

    if let Some(tok) = try_keyring() {
        tracing::debug!("token loaded from keyring");
        return Ok(tok);
    }

    if let Some(tok) = try_plaintext_fallback()? {
        tracing::debug!("token loaded from plaintext fallback file");
        return Ok(tok);
    }

    tracing::info!("bootstrapping token from ~/.openclaw/openclaw.json");
    let tok = read_openclaw_config()?;

    if let Err(e) = stash_in_keyring(&tok) {
        tracing::warn!(error = %e, "keyring stash failed; writing plaintext fallback");
        write_plaintext_fallback(&tok)?;
    }

    Ok(tok)
}

fn try_keyring() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()
        .and_then(|entry| entry.get_password().ok())
}

fn stash_in_keyring(token: &str) -> Result<(), ConfigError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    entry.set_password(token)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct OpenclawConfig {
    #[serde(default)]
    gateway: Option<OpenclawGateway>,
}

#[derive(Debug, Deserialize)]
struct OpenclawGateway {
    #[serde(default)]
    auth: Option<OpenclawAuth>,
}

#[derive(Debug, Deserialize)]
struct OpenclawAuth {
    token: Option<String>,
}

fn read_openclaw_config() -> Result<String, ConfigError> {
    let path = openclaw_config_path()?;
    let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let cfg: OpenclawConfig = serde_json::from_str(&raw)?;
    cfg.gateway
        .and_then(|g| g.auth)
        .and_then(|a| a.token)
        .ok_or(ConfigError::MissingToken)
}

fn openclaw_config_path() -> Result<PathBuf, ConfigError> {
    let home = UserDirs::new().ok_or(ConfigError::NoHome)?;
    Ok(home.home_dir().join(".openclaw").join("openclaw.json"))
}

fn fallback_file_path() -> Result<PathBuf, ConfigError> {
    let base = BaseDirs::new().ok_or(ConfigError::NoHome)?;
    Ok(base.config_dir().join("sassy-dog").join("gateway-token"))
}

fn try_plaintext_fallback() -> Result<Option<String>, ConfigError> {
    let path = fallback_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    Ok(Some(raw.trim().to_string()))
}

fn write_plaintext_fallback(token: &str) -> Result<(), ConfigError> {
    let path = fallback_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, token)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

/// Stable per-install UUID, stored beside the token fallback file.
/// Used as `client.instanceId` in the gateway connect frame so the
/// server can distinguish restarts of the same install from a new one.
pub fn instance_id() -> Result<String, ConfigError> {
    let base = BaseDirs::new().ok_or(ConfigError::NoHome)?;
    let dir = base.config_dir().join("sassy-dog");
    fs::create_dir_all(&dir)?;
    let path = dir.join("instance-id");

    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    fs::write(&path, &id)?;
    Ok(id)
}
