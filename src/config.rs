//! Gateway URL + token resolution.
//!
//! ## Gateway URL
//!
//! [`gateway_url`] resolves the ws endpoint from (in order):
//!
//! 1. `OPENCLAW_GATEWAY_URL` env var — wins when set, for CI / Doppler
//!    / one-shot overrides.
//! 2. Persisted setting (typically `UiState.settings.gateway_url`,
//!    threaded in by the caller).
//! 3. `None` — the ws subscription stays idle and the Settings tab's
//!    first-run banner asks the operator to configure one.
//!
//! ## Token
//!
//! [`try_load_token`] follows the same env-wins-then-persisted rule,
//! but persistence lives in [`crate::secret_store`] (keychain in
//! release, 0600 file in debug). See that module for storage details.
//!
//! **Last-resort bootstrap**: if no token is available from env or
//! the secret store, we check `~/.openclaw/openclaw.json` for an
//! `auth.token`. This is purely a convenience for OpenClaw-CLI users
//! — it lets a co-installed gateway's token pair work automatically
//! on first launch. A successful bootstrap writes the token to the
//! secret store for future launches.
//!
//! Returning `None` is valid — that's the Tailscale-Serve case where
//! the gateway authenticates via whois headers and a client-side
//! token would only interfere.

use std::fs;
use std::path::PathBuf;

use directories::{BaseDirs, UserDirs};
use serde::Deserialize;

use crate::secret_store;

/// Resolve the gateway URL. Env var wins, persisted setting (pass
/// `Some(state.settings.gateway_url.as_deref())`) is the fallback.
/// Returns `None` when neither is set — the caller should either
/// stay idle (ws path) or skip connecting (mock path).
///
/// The WS path is the root (`/`). Do NOT point this at
/// `/__openclaw__/ws`, which is the canvas WS path (different
/// protocol, intercepts WS upgrades first). Gateway WS lives at root
/// and falls through after canvas declines.
pub fn gateway_url(persisted: Option<&str>) -> Option<String> {
    if let Some(url) = env_str("OPENCLAW_GATEWAY_URL") {
        return Some(url);
    }
    persisted
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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
    #[error("secret store: {0}")]
    Secret(#[from] secret_store::SecretStoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Best-effort token resolution. Logs at debug and returns `None` on
/// any failure — callers should treat `None` as "no token, try
/// ambient auth" rather than as an error.
pub fn try_load_token() -> Option<String> {
    match load_token() {
        Ok(tok) => Some(tok),
        Err(e) => {
            tracing::debug!(error = %e, "no gateway token; relying on ambient auth");
            None
        }
    }
}

/// Resolve the gateway token. Env > secret store > openclaw.json
/// bootstrap. The bootstrap path, when it succeeds, also stashes the
/// token in the secret store so the next launch skips `~/.openclaw`
/// entirely.
pub fn load_token() -> Result<String, ConfigError> {
    if let Some(tok) = env_str("OPENCLAW_TOKEN") {
        tracing::debug!("OPENCLAW_TOKEN env var provided");
        return Ok(tok);
    }

    if let Some(tok) = secret_store::load_token() {
        tracing::debug!("token loaded from secret store");
        return Ok(tok);
    }

    tracing::info!("bootstrapping token from ~/.openclaw/openclaw.json");
    let tok = read_openclaw_config()?;
    if let Err(e) = secret_store::save_token(&tok) {
        tracing::warn!(error = %e, "could not persist bootstrapped token to secret store");
    }
    Ok(tok)
}

/// Read a trimmed, non-empty env var. Returns `None` when unset or
/// whitespace-only — the call sites all want this exact semantics.
fn env_str(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|s| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    })
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
