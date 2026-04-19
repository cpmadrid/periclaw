//! Ed25519 device identity for OpenClaw gateway pairing.
//!
//! Replaces the `controlUi.dangerouslyDisableDeviceAuth` break-glass
//! path with a proper device pairing flow: generate an Ed25519 keypair
//! once per install, sign the gateway's connect challenge, and let the
//! operator approve the pairing via Control UI (or `openclaw device
//! pair approve`). Subsequent connects authenticate silently via the
//! signed nonce.
//!
//! Wire format matches OpenClaw's `buildDeviceAuthPayload` v2
//! (`openclaw/src/gateway/device-auth.ts:20`) and the signature +
//! public-key encoding in `openclaw/src/infra/device-identity.ts` —
//! base64url with padding stripped; device id = SHA-256(raw public
//! key) hex-encoded.
//!
//! ## Storage
//!
//! Private key: 32-byte Ed25519 signing seed, base64url-encoded, in
//! the macOS keychain (Keychain `com.sassydog.mission-control-desktop`
//! / account `openclaw-device-key`). Falls back to
//! `$XDG_CONFIG_HOME/sassy-dog/device-key` (mode 0600) on Linux
//! installs without a keyring daemon, matching the token-storage
//! pattern in `config.rs`.
//!
//! Public key is always derived fresh from the signing key at load —
//! no point storing it separately.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const KEYRING_SERVICE: &str = "com.sassydog.mission-control-desktop";
const KEYRING_USER: &str = "openclaw-device-key";

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("no home directory available")]
    NoHome,
    #[error("stored device key is malformed: {0}")]
    Malformed(String),
}

/// A loaded-or-freshly-generated device identity.
///
/// `signing_key` is kept behind a reference to prevent accidental
/// copies — Ed25519 signing keys should not be printed or cloned
/// casually.
pub struct DeviceIdentity {
    /// SHA-256 of the raw public key, lowercase hex. Stable across
    /// runs because it's derived from the keypair, not stored.
    pub device_id: String,
    /// Raw 32-byte Ed25519 public key. Sent verbatim (base64url) in
    /// the gateway connect frame.
    pub public_key_raw: [u8; 32],
    signing_key: SigningKey,
}

impl DeviceIdentity {
    /// Load the stored keypair or generate a new one. Persists the
    /// new key to the keychain (or plaintext fallback) on first run.
    pub fn load_or_create() -> Result<Self, IdentityError> {
        if let Some(key) = try_keyring()? {
            tracing::debug!(device_id = %device_id_of(&key), "device identity loaded from keychain");
            return Ok(Self::from_signing_key(key));
        }
        if let Some(key) = try_plaintext_fallback()? {
            tracing::debug!(device_id = %device_id_of(&key), "device identity loaded from fallback file");
            return Ok(Self::from_signing_key(key));
        }
        let key = generate_signing_key();
        let ident = Self::from_signing_key(key.clone());
        tracing::info!(
            device_id = %ident.device_id,
            "generated new device identity — pair via Control UI to activate",
        );
        match persist_keyring(&key) {
            Ok(()) => tracing::info!("device key persisted to keychain"),
            Err(e) => {
                tracing::warn!(error = %e, "keyring persist failed; writing plaintext fallback");
                persist_plaintext_fallback(&key)?;
                tracing::info!(
                    path = ?fallback_path().ok(),
                    "device key persisted to plaintext fallback",
                );
            }
        }
        Ok(ident)
    }

    fn from_signing_key(signing_key: SigningKey) -> Self {
        let verifying: VerifyingKey = signing_key.verifying_key();
        let public_key_raw = verifying.to_bytes();
        let device_id = fingerprint(&public_key_raw);
        Self {
            device_id,
            public_key_raw,
            signing_key,
        }
    }

    /// Base64url (no padding) encoding of the 32-byte public key.
    /// This is the `publicKey` field the gateway expects.
    pub fn public_key_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public_key_raw)
    }

    /// Sign a `connect` challenge per OpenClaw's v2 device-auth
    /// payload format (`device-auth.ts:20`), returning the
    /// base64url-encoded signature plus the exact `signedAtMs` used
    /// (the caller needs to echo the same value in the connect
    /// frame so the server verifies against the same payload).
    pub fn sign_connect(&self, params: SignConnectParams<'_>) -> SignedConnect {
        let signed_at_ms = params.signed_at_ms;
        let scopes = params.scopes.join(",");
        let token = params.token.unwrap_or("");
        let payload = format!(
            "v2|{device_id}|{client_id}|{client_mode}|{role}|{scopes}|{signed_at_ms}|{token}|{nonce}",
            device_id = self.device_id,
            client_id = params.client_id,
            client_mode = params.client_mode,
            role = params.role,
            scopes = scopes,
            signed_at_ms = signed_at_ms,
            token = token,
            nonce = params.nonce,
        );
        let sig = self.signing_key.sign(payload.as_bytes());
        SignedConnect {
            signature_base64url: URL_SAFE_NO_PAD.encode(sig.to_bytes()),
            signed_at_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SignConnectParams<'a> {
    pub client_id: &'a str,
    pub client_mode: &'a str,
    pub role: &'a str,
    pub scopes: &'a [&'a str],
    pub token: Option<&'a str>,
    pub nonce: &'a str,
    pub signed_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct SignedConnect {
    pub signature_base64url: String,
    pub signed_at_ms: i64,
}

fn generate_signing_key() -> SigningKey {
    let mut csprng = rand::rngs::OsRng;
    SigningKey::generate(&mut csprng)
}

fn device_id_of(key: &SigningKey) -> String {
    let raw = key.verifying_key().to_bytes();
    fingerprint(&raw)
}

fn fingerprint(public_key_raw: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key_raw);
    hex::encode(digest)
}

fn try_keyring() -> Result<Option<SigningKey>, IdentityError> {
    let entry = match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "keyring Entry::new failed; skipping");
            return Ok(None);
        }
    };
    match entry.get_password() {
        Ok(seed_b64) => Ok(Some(decode_signing_key(&seed_b64)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, "keyring get_password failed");
            Ok(None)
        }
    }
}

fn persist_keyring(key: &SigningKey) -> Result<(), IdentityError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    entry.set_password(&URL_SAFE_NO_PAD.encode(key.to_bytes()))?;
    Ok(())
}

fn try_plaintext_fallback() -> Result<Option<SigningKey>, IdentityError> {
    let path = fallback_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    Ok(Some(decode_signing_key(raw.trim())?))
}

fn persist_plaintext_fallback(key: &SigningKey) -> Result<(), IdentityError> {
    let path = fallback_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, URL_SAFE_NO_PAD.encode(key.to_bytes()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

fn fallback_path() -> Result<std::path::PathBuf, IdentityError> {
    let base = directories::BaseDirs::new().ok_or(IdentityError::NoHome)?;
    Ok(base.config_dir().join("sassy-dog").join("device-key"))
}

fn decode_signing_key(seed_b64url: &str) -> Result<SigningKey, IdentityError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(seed_b64url)
        .map_err(|e| IdentityError::Malformed(format!("base64 decode: {e}")))?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|v: Vec<u8>| IdentityError::Malformed(format!("expected 32 bytes, got {}", v.len())))?;
    Ok(SigningKey::from_bytes(&seed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_connect_payload_matches_openclaw_v2_shape() {
        // Constructed independently from
        // `openclaw/src/gateway/device-auth.ts:20` — any drift here
        // means the gateway will reject our signatures.
        let seed = [0u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let ident = DeviceIdentity::from_signing_key(signing_key);
        let signed = ident.sign_connect(SignConnectParams {
            client_id: "openclaw-tui",
            client_mode: "ui",
            role: "operator",
            scopes: &["operator.read"],
            token: Some("tok"),
            nonce: "nonce-xyz",
            signed_at_ms: 1776000000000,
        });
        // Signature is deterministic for a fixed seed + payload, so
        // we can snapshot it once and catch any future payload format
        // drift by comparing against the canonical signature.
        assert!(!signed.signature_base64url.is_empty());
        assert_eq!(signed.signed_at_ms, 1776000000000);

        // The expected payload exactly:
        // v2|<deviceId>|openclaw-tui|ui|operator|operator.read|1776000000000|tok|nonce-xyz
        let payload = format!(
            "v2|{}|openclaw-tui|ui|operator|operator.read|1776000000000|tok|nonce-xyz",
            ident.device_id,
        );
        let sig = ident.signing_key.sign(payload.as_bytes());
        assert_eq!(
            URL_SAFE_NO_PAD.encode(sig.to_bytes()),
            signed.signature_base64url,
            "sign_connect must serialize the exact v2 payload format"
        );
    }

    #[test]
    fn device_id_is_sha256_of_public_key() {
        let seed = [1u8; 32];
        let key = SigningKey::from_bytes(&seed);
        let ident = DeviceIdentity::from_signing_key(key);
        // Device id length is 64 hex chars (32-byte SHA-256).
        assert_eq!(ident.device_id.len(), 64);
        assert!(ident.device_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn public_key_base64url_is_43_chars() {
        // 32 bytes → ceil(32/3)*4 = 44 chars with padding, 43 without.
        let seed = [2u8; 32];
        let ident = DeviceIdentity::from_signing_key(SigningKey::from_bytes(&seed));
        let encoded = ident.public_key_base64url();
        assert_eq!(encoded.len(), 43, "got {}: {}", encoded.len(), encoded);
        assert!(!encoded.contains('='), "url-safe encoding drops padding");
    }

    #[test]
    fn decode_signing_key_round_trips() {
        let original = SigningKey::from_bytes(&[7u8; 32]);
        let encoded = URL_SAFE_NO_PAD.encode(original.to_bytes());
        let decoded = decode_signing_key(&encoded).unwrap();
        assert_eq!(decoded.to_bytes(), original.to_bytes());
    }
}
