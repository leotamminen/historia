//! Ed25519 snapshot signing (CLAUDE.md CP14): the keypair, and signing/
//! verifying manifest bytes and per-snapshot signature files.
//!
//! **Key storage (§9-style contract):** the keypair lives at
//! `.historia/signing_key` (private) and `.historia/signing_key.pub` (public),
//! unencrypted, hex-encoded, plain text - the same "recoverable by hand"
//! convention as every other file in the store (CLAUDE.md Rule 8). This is a
//! deliberate simplicity tradeoff, not an oversight: anyone with filesystem
//! access to `.historia/` can sign as this identity. historia has no key
//! management story beyond "it's a file"; treat `.historia/` as sensitive if
//! that matters to you.
//!
//! **Signature storage:** a signature lives in its own sidecar file,
//! `.historia/snapshots/<n>.json.sig` (hex-encoded), never a field inside the
//! manifest JSON itself. This is required, not a style choice: CP13 hashes (and
//! this module signs) a manifest's *exact on-disk bytes* for its chain link;
//! embedding the signature inside the manifest it signs would be circular
//! (the bytes being signed would have to exclude the very field holding the
//! result). A sidecar file signs precisely the same bytes CP13 already hashes,
//! with no exclusion logic needed anywhere.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;

use crate::core::fsutil;
use crate::core::hash::hex_encode;

pub const PRIVATE_KEY_FILE_NAME: &str = "signing_key";
pub const PUBLIC_KEY_FILE_NAME: &str = "signing_key.pub";

fn private_key_path(store_dir: &Path) -> PathBuf {
    store_dir.join(PRIVATE_KEY_FILE_NAME)
}

fn public_key_path(store_dir: &Path) -> PathBuf {
    store_dir.join(PUBLIC_KEY_FILE_NAME)
}

/// Why a signing key couldn't be loaded or ensured.
#[derive(Debug)]
pub enum KeyError {
    /// Exactly one of the two key files is present. A key existed and is now
    /// partly gone - CLAUDE.md CP14's locked decision: never silently
    /// regenerate in this case, since that would replace an identity out from
    /// under the user without them asking for it. `historia keygen --force`
    /// is the explicit, intentional way to do that.
    PartiallyMissing,
    /// A key file exists but its content isn't a valid key (corrupted).
    Invalid,
    Io(io::Error),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::PartiallyMissing => write!(
                f,
                "signing key missing - run `historia keygen` to create a new one"
            ),
            KeyError::Invalid => write!(f, "signing key file is corrupted (not a valid key)"),
            KeyError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<io::Error> for KeyError {
    fn from(e: io::Error) -> Self {
        KeyError::Io(e)
    }
}

/// True if either key file is present (used by `historia keygen`'s
/// already-exists check - any trace of a prior key counts, not just a
/// complete pair, since a partial one still represents an identity in use).
pub fn any_key_file_present(store_dir: &Path) -> bool {
    private_key_path(store_dir).is_file() || public_key_path(store_dir).is_file()
}

/// Ensure a signing keypair exists at `store_dir`, generating one only if
/// NEITHER file is present yet (CLAUDE.md CP14: the common, no-prompt,
/// fresh-store case - "no key has ever existed"). If exactly one file is
/// present, that means a key existed and is now partly missing:
/// [`KeyError::PartiallyMissing`], never a silent regeneration.
pub fn ensure_key(store_dir: &Path) -> Result<SigningKey, KeyError> {
    let priv_exists = private_key_path(store_dir).is_file();
    let pub_exists = public_key_path(store_dir).is_file();

    match (priv_exists, pub_exists) {
        (true, true) => load_signing_key(store_dir),
        (false, false) => Ok(generate_and_save_key(store_dir)?),
        _ => Err(KeyError::PartiallyMissing),
    }
}

/// Generate a fresh keypair and write both files, OVERWRITING whatever is
/// there. Callers decide the "may I overwrite?" policy ([`ensure_key`] only
/// calls this when neither file exists; `historia keygen` gates it on
/// `--force` when one already does) - this function itself always writes.
pub fn generate_and_save_key(store_dir: &Path) -> io::Result<SigningKey> {
    let signing_key = SigningKey::generate(&mut OsRng);
    write_keypair(store_dir, &signing_key)?;
    Ok(signing_key)
}

/// Write both key files for `signing_key` atomically (write-then-rename, Rule
/// 5), hex-encoded plain text.
pub fn write_keypair(store_dir: &Path, signing_key: &SigningKey) -> io::Result<()> {
    let priv_hex = hex_encode(signing_key.to_bytes().as_slice());
    let pub_hex = hex_encode(signing_key.verifying_key().to_bytes().as_slice());
    fsutil::write_atomic(&private_key_path(store_dir), format!("{priv_hex}\n").as_bytes())?;
    fsutil::write_atomic(&public_key_path(store_dir), format!("{pub_hex}\n").as_bytes())?;
    Ok(())
}

/// Load the private signing key from disk.
pub fn load_signing_key(store_dir: &Path) -> Result<SigningKey, KeyError> {
    let hex = fs::read_to_string(private_key_path(store_dir))?;
    let bytes = hex_decode(hex.trim()).ok_or(KeyError::Invalid)?;
    let array: [u8; 32] = bytes.try_into().map_err(|_| KeyError::Invalid)?;
    Ok(SigningKey::from_bytes(&array))
}

/// Load the public verifying key from disk.
pub fn load_verifying_key(store_dir: &Path) -> Result<VerifyingKey, KeyError> {
    let hex = fs::read_to_string(public_key_path(store_dir))?;
    let bytes = hex_decode(hex.trim()).ok_or(KeyError::Invalid)?;
    let array: [u8; 32] = bytes.try_into().map_err(|_| KeyError::Invalid)?;
    VerifyingKey::from_bytes(&array).map_err(|_| KeyError::Invalid)
}

/// Sign `bytes` (a manifest file's exact on-disk bytes) with `signing_key`.
pub fn sign(signing_key: &SigningKey, bytes: &[u8]) -> Signature {
    signing_key.sign(bytes)
}

/// True if `signature` validates as `verifying_key`'s signature over `bytes`.
pub fn verify_signature(verifying_key: &VerifyingKey, bytes: &[u8], signature: &Signature) -> bool {
    verifying_key.verify(bytes, signature).is_ok()
}

/// Path to a snapshot's signature sidecar file,
/// `.historia/snapshots/<number>.json.sig` (CLAUDE.md CP14).
pub fn signature_path(store_dir: &Path, number: u64) -> PathBuf {
    store_dir.join("snapshots").join(format!("{number}.json.sig"))
}

/// Atomically write a snapshot's signature (write-then-rename, Rule 5),
/// hex-encoded plain text - same convention as the key files.
pub fn write_signature(store_dir: &Path, number: u64, signature: &Signature) -> io::Result<()> {
    let hex = hex_encode(&signature.to_bytes());
    fsutil::write_atomic(&signature_path(store_dir, number), format!("{hex}\n").as_bytes())
}

/// Read and parse a snapshot's signature file.
pub fn read_signature(store_dir: &Path, number: u64) -> io::Result<Signature> {
    let hex = fs::read_to_string(signature_path(store_dir, number))?;
    let bytes = hex_decode(hex.trim())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "signature file: invalid hex"))?;
    let array: [u8; 64] = bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "signature file: expected 64 bytes"))?;
    Ok(Signature::from_bytes(&array))
}

/// Decode a lowercase (or uppercase) hex string to bytes; `None` on malformed
/// input (odd length, non-hex characters) rather than panicking - callers
/// treat this the same as any other corrupted-file case.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::store::init_store;
    use tempfile::tempdir;

    #[test]
    fn hex_decode_round_trips_with_hex_encode() {
        let bytes = [0u8, 1, 255, 128, 16, 9];
        assert_eq!(hex_decode(&hex_encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn hex_decode_rejects_odd_length() {
        assert_eq!(hex_decode("abc"), None);
    }

    #[test]
    fn hex_decode_rejects_non_hex_characters() {
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn ensure_key_generates_a_fresh_key_when_neither_file_exists() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let key = ensure_key(&store_dir).unwrap();

        assert!(private_key_path(&store_dir).is_file());
        assert!(public_key_path(&store_dir).is_file());
        // The saved public key matches the generated key's own public half.
        assert_eq!(load_verifying_key(&store_dir).unwrap(), key.verifying_key());
    }

    #[test]
    fn ensure_key_loads_an_existing_key_without_regenerating() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let original = ensure_key(&store_dir).unwrap();

        let loaded = ensure_key(&store_dir).unwrap();

        assert_eq!(loaded.verifying_key(), original.verifying_key());
    }

    #[test]
    fn ensure_key_errors_when_only_the_private_key_is_missing() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        ensure_key(&store_dir).unwrap();
        fs::remove_file(private_key_path(&store_dir)).unwrap();

        let err = ensure_key(&store_dir).unwrap_err();

        assert!(matches!(err, KeyError::PartiallyMissing), "expected PartiallyMissing, got {err:?}");
    }

    #[test]
    fn ensure_key_errors_when_only_the_public_key_is_missing() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        ensure_key(&store_dir).unwrap();
        fs::remove_file(public_key_path(&store_dir)).unwrap();

        let err = ensure_key(&store_dir).unwrap_err();

        assert!(matches!(err, KeyError::PartiallyMissing), "expected PartiallyMissing, got {err:?}");
    }

    #[test]
    fn write_keypair_then_load_signing_key_round_trips() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let generated = SigningKey::generate(&mut OsRng);

        write_keypair(&store_dir, &generated).unwrap();
        let loaded = load_signing_key(&store_dir).unwrap();

        assert_eq!(loaded.to_bytes(), generated.to_bytes());
    }

    #[test]
    fn sign_then_verify_signature_succeeds_for_untampered_bytes() {
        let key = SigningKey::generate(&mut OsRng);
        let bytes = b"some manifest bytes";

        let sig = sign(&key, bytes);

        assert!(verify_signature(&key.verifying_key(), bytes, &sig));
    }

    #[test]
    fn verify_signature_fails_for_tampered_bytes() {
        let key = SigningKey::generate(&mut OsRng);
        let sig = sign(&key, b"original bytes");

        assert!(!verify_signature(&key.verifying_key(), b"tampered bytes!", &sig));
    }

    #[test]
    fn verify_signature_fails_for_the_wrong_key() {
        let key = SigningKey::generate(&mut OsRng);
        let other_key = SigningKey::generate(&mut OsRng);
        let bytes = b"some manifest bytes";
        let sig = sign(&key, bytes);

        assert!(!verify_signature(&other_key.verifying_key(), bytes, &sig));
    }

    #[test]
    fn write_signature_then_read_signature_round_trips() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let sig = sign(&key, b"manifest bytes");

        write_signature(&store_dir, 1, &sig).unwrap();
        let loaded = read_signature(&store_dir, 1).unwrap();

        assert_eq!(loaded.to_bytes(), sig.to_bytes());
    }
}
