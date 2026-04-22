//! OS-keychain-backed (release) or plaintext-fallback (debug) store
//! for the gateway token. Single entry point so the rest of the app
//! doesn't have to know which backend is active — save, load, clear,
//! and a user-visible storage-location hint for the Settings UI.
//!
//! ## Why the build-flavor split
//!
//! On macOS dev builds, the code-signing identity changes every
//! `cargo run` / `cargo build`, so the OS treats each launch as a
//! new app requesting access to keychain items it didn't create and
//! prompts for the login password. That's fine once; it's infuriating
//! dozens of times a day. Release builds (signed consistently once,
//! re-launched without rebuilding) don't hit the prompt, so the
//! keychain is the right home for them.
//!
//! Linux/Windows don't have this problem — we still route by
//! `cfg!(debug_assertions)` for symmetry and to keep the behavior
//! predictable across platforms.
//!
//! ## Migration
//!
//! A release-build `load_token()` that finds a token in the plaintext
//! fallback file (left there by a prior debug-build session) copies
//! it into the keychain and deletes the file, so the user doesn't
//! keep two copies of the secret around.
//!
//! ## Clear
//!
//! `clear_token()` purges **both** stores regardless of build flavor.
//! The guarantee the UI makes is "Clear means clear everywhere",
//! which is the only user-intelligible semantic.

use std::fs;
use std::path::PathBuf;

use directories::BaseDirs;

/// Keychain service identifier. Matches the constant in `config.rs`
/// so existing installs' stored tokens migrate cleanly.
pub const KEYRING_SERVICE: &str = "com.cpmadrid.periclaw";
/// Keychain account/user identifier for the gateway token.
pub const KEYRING_USER: &str = "openclaw-gateway-token";

/// Relative name of the plaintext fallback file within the periclaw
/// config dir. Full path is `$XDG_CONFIG_HOME/periclaw/gateway-token`.
const FALLBACK_FILE: &str = "gateway-token";

#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    #[error("no home directory available on this platform")]
    NoHome,
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Persist the token in the build-flavor-appropriate backend.
/// Release → keychain; debug → 0600 plaintext file.
pub fn save_token(token: &str) -> Result<(), SecretStoreError> {
    if cfg!(debug_assertions) {
        save_to_file(token)
    } else {
        save_to_keychain(token)
    }
}

/// Read the token, migrating across backends as needed.
///
/// **Release build**: prefer the keychain. If absent but a plaintext
/// file exists (left over from a previous debug-build session), copy
/// it into the keychain and delete the file — the secret ends up in
/// one place.
///
/// **Debug build**: prefer the file (keychain writes prompt the
/// login password on every `cargo run`). If absent but a keychain
/// entry exists (left over from a prior release-build install), read
/// through — no migration back to the file, the release keychain
/// entry stays put.
pub fn load_token() -> Option<String> {
    if cfg!(debug_assertions) {
        load_from_file().or_else(load_from_keychain)
    } else {
        if let Some(t) = load_from_keychain() {
            return Some(t);
        }
        // Migrate a leftover plaintext token into the keychain.
        if let Some(t) = load_from_file() {
            if let Err(e) = save_to_keychain(&t) {
                tracing::warn!(error = %e, "migrating plaintext token to keychain failed");
            }
            if let Err(e) = delete_file() {
                tracing::warn!(error = %e, "deleting migrated plaintext token failed");
            }
            return Some(t);
        }
        None
    }
}

/// Delete the token from every store we know about. Errors are
/// logged at debug and swallowed — "clear" must succeed even if one
/// backend can't be reached.
pub fn clear_token() {
    if let Err(e) = delete_keychain() {
        tracing::debug!(error = %e, "clearing keychain token failed");
    }
    if let Err(e) = delete_file() {
        tracing::debug!(error = %e, "clearing plaintext token file failed");
    }
}

/// Short human-readable string for the Settings UI so the operator
/// can see *where* their secret lives under the current build flavor.
pub fn storage_location_hint() -> &'static str {
    if cfg!(debug_assertions) {
        "on-disk fallback file (debug build)"
    } else {
        "OS keychain"
    }
}

/// `true` if a token is currently stored in either backend. Used by
/// the Settings UI to toggle between the "save new token" form state
/// and the "token present, offer Clear" state. Does not expose the
/// value.
pub fn has_token() -> bool {
    load_from_keychain().is_some() || fallback_file_path().is_some_and(|p| p.exists())
}

// ---------------------------------------------------------------------
// Backend helpers
// ---------------------------------------------------------------------

fn save_to_keychain(token: &str) -> Result<(), SecretStoreError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    entry.set_password(token)?;
    Ok(())
}

fn load_from_keychain() -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    entry.get_password().ok()
}

fn delete_keychain() -> Result<(), SecretStoreError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        // "Not found" is fine — clear is idempotent.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn save_to_file(token: &str) -> Result<(), SecretStoreError> {
    let path = fallback_file_path().ok_or(SecretStoreError::NoHome)?;
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

fn load_from_file() -> Option<String> {
    let path = fallback_file_path()?;
    if !path.exists() {
        return None;
    }
    let raw = fs::read_to_string(&path).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn delete_file() -> Result<(), SecretStoreError> {
    let Some(path) = fallback_file_path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path)?;
    Ok(())
}

fn fallback_file_path() -> Option<PathBuf> {
    let base = BaseDirs::new()?;
    Some(base.config_dir().join("periclaw").join(FALLBACK_FILE))
}
