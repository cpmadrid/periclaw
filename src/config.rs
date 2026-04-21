//! Token bootstrap and config paths.
//!
//! **Lookup order** (first hit wins):
//! 1. `OPENCLAW_TOKEN` env var — the preferred source of truth;
//!    when present we never fall through.
//! 2. Plaintext fallback file at `$XDG_CONFIG_HOME/periclaw/gateway-token`
//!    (mode 0600).
//! 3. OS keychain — only a read path, kept for backward-compat with
//!    installs that stashed here under older builds.
//! 4. Bootstrap from `~/.openclaw/openclaw.json` → `auth.token`.
//!
//! **Writes go to the file, not the keychain.** On macOS dev builds
//! the code-signing identity changes every `cargo run`, so the OS
//! treats a keychain write as a fresh app asking for confidential
//! access and prompts for the login password — every launch. Writing
//! to a 0600 file under the user's config dir matches OpenClaw's own
//! secrets layout and avoids the prompt entirely. The keychain read
//! path is still consulted so an older install with a stashed token
//! keeps working until it's migrated.

use std::fs;
use std::path::PathBuf;

use directories::{BaseDirs, UserDirs};
use serde::Deserialize;

const KEYRING_SERVICE: &str = "com.cpmadrid.periclaw";
const KEYRING_USER: &str = "openclaw-gateway-token";

/// Resolve the gateway URL from `OPENCLAW_GATEWAY_URL`. Returns `None`
/// when the env var is unset or empty — callers should surface a
/// helpful error rather than fall back to a hardcoded endpoint.
///
/// The WS path is the root (`/`). Do NOT point this at
/// `/__openclaw__/ws`, which is the canvas WS path (different protocol,
/// intercepts WS upgrades first). Gateway WS lives at root and falls
/// through after canvas declines.
pub fn gateway_url() -> Option<String> {
    std::env::var("OPENCLAW_GATEWAY_URL").ok().and_then(|s| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    })
}

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

/// Try to load a gateway token. Returns `None` when no token is
/// available — that's the Tailscale-Serve case, where the gateway
/// authenticates the connection via Tailscale whois headers and a
/// client-side token would only interfere (it flips the gateway into
/// token-comparison mode; see `openclaw/src/gateway/auth.ts:577`).
pub fn try_load_token() -> Option<String> {
    match load_token() {
        Ok(tok) => Some(tok),
        Err(e) => {
            tracing::debug!(error = %e, "no gateway token; relying on ambient auth");
            None
        }
    }
}

/// Load the OpenClaw gateway token. See module docs for the lookup
/// order. Writes persist to the plaintext fallback file (0600); the
/// keychain is read-only to avoid prompting on macOS dev builds.
pub fn load_token() -> Result<String, ConfigError> {
    if let Ok(tok) = std::env::var("OPENCLAW_TOKEN") {
        let tok = tok.trim().to_string();
        if !tok.is_empty() {
            // Env var is the preferred source — use it directly. No
            // stash: writing to the keychain here triggers a
            // login-password prompt on every dev launch because the
            // binary signature changes with each build.
            tracing::debug!("OPENCLAW_TOKEN env var provided");
            return Ok(tok);
        }
    }

    if let Some(tok) = try_plaintext_fallback()? {
        tracing::debug!("token loaded from plaintext fallback file");
        return Ok(tok);
    }

    if let Some(tok) = try_keyring() {
        tracing::debug!("token loaded from keyring (legacy install)");
        // Mirror into the file so the next launch doesn't consult
        // the keychain at all — best-effort; the legacy read path
        // still works if this fails.
        if let Err(e) = write_plaintext_fallback(&tok) {
            tracing::debug!(error = %e, "could not mirror keychain token to fallback file");
        }
        return Ok(tok);
    }

    tracing::info!("bootstrapping token from ~/.openclaw/openclaw.json");
    let tok = read_openclaw_config()?;
    if let Err(e) = write_plaintext_fallback(&tok) {
        tracing::warn!(error = %e, "could not persist bootstrapped token to fallback file");
    }
    Ok(tok)
}

fn try_keyring() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()
        .and_then(|entry| entry.get_password().ok())
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
    Ok(base.config_dir().join("periclaw").join("gateway-token"))
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
    let dir = base.config_dir().join("periclaw");
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
