//! age encryption for whole-store encrypted backups (CLAUDE.md CP15): key
//! management (mirroring CP14's `core::signing` pattern closely) and the
//! encrypt/decrypt entry points that glue `core::archive` (packing) to the
//! `age` crate (the actual cipher).
//!
//! **Key storage (§9-style contract):** the identity/recipient keypair lives
//! at `.historia/age_key` (identity, private) and `.historia/age_key.pub`
//! (recipient, public), unencrypted, plain text - age's own native encoding
//! (`AGE-SECRET-KEY-1...` / `age1...`). Same "recoverable by hand" convention,
//! and the same deliberate simplicity tradeoff, as CP14's signing key
//! (`core::signing`): anyone with filesystem access to `.historia/` can
//! decrypt backups made under this identity. Treat `.historia/` as sensitive
//! if that matters to you.
//!
//! **What gets encrypted:** the ENTIRE store - format, HEAD, every snapshot
//! manifest and its optional CP14 signature sidecar, every blob, and both the
//! CP14 signing keypair and this CP15 age keypair if present - packed by
//! `core::archive::pack_store_into` into one archive and encrypted with age to
//! a SINGLE OUTPUT FILE. This is deliberately different from CP11's `backup`,
//! which produces a plain directory: an encrypted backup's destination is one
//! opaque file, safe to push to untrusted storage.
//!
//! **Decrypting requires the identity to already exist locally, in the store
//! `historia backup --decrypt` is run from** - not inside the file being
//! decrypted. The identity travels INSIDE encrypted backups too (so a
//! decrypted store can go on to make further encrypted backups under the same
//! identity without extra setup), but that only reconstitutes the store's
//! *contents* - it can't bootstrap the very ability to decrypt in the first
//! place. If you want to decrypt a backup somewhere new, either bring
//! `.historia/age_key` along yourself, or decrypt it once wherever the
//! original identity still lives and re-copy from there.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use age::x25519::Identity;

use crate::core::fsutil;

pub const IDENTITY_FILE_NAME: &str = "age_key";
pub const RECIPIENT_FILE_NAME: &str = "age_key.pub";

fn identity_path(store_dir: &Path) -> PathBuf {
    store_dir.join(IDENTITY_FILE_NAME)
}

fn recipient_path(store_dir: &Path) -> PathBuf {
    store_dir.join(RECIPIENT_FILE_NAME)
}

/// Why an age key couldn't be loaded, ensured, or used.
#[derive(Debug)]
pub enum KeyError {
    /// Neither key file is present - there is no identity to encrypt or
    /// decrypt with at all (e.g. decrypting before this store ever made an
    /// encrypted backup, or after total key loss).
    Missing,
    /// Exactly one of the two key files is present - a key existed and is now
    /// partly gone. Same CP14 pattern: never silently regenerate.
    PartiallyMissing,
    /// A key file exists but its content isn't a valid age key.
    Invalid,
    Io(io::Error),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Missing => write!(
                f,
                "no age encryption key found - run `historia keygen --encryption` to create one"
            ),
            KeyError::PartiallyMissing => write!(
                f,
                "age encryption key missing - run `historia keygen --encryption` to create a new one"
            ),
            KeyError::Invalid => write!(f, "age key file is corrupted (not a valid age key)"),
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

/// True if either key file is present. Mirrors `signing::any_key_file_present`,
/// used by `historia keygen`'s already-exists check: any trace of a prior key
/// counts, not just a complete pair.
pub fn any_key_file_present(store_dir: &Path) -> bool {
    identity_path(store_dir).is_file() || recipient_path(store_dir).is_file()
}

/// Ensure an age keypair exists at `store_dir`, generating one only if
/// NEITHER file is present yet (the common, no-prompt case: this store's
/// first `--encrypt` backup). If exactly one file is present, a key existed
/// and is now partly missing: [`KeyError::PartiallyMissing`], never a silent
/// regeneration.
pub fn ensure_key(store_dir: &Path) -> Result<Identity, KeyError> {
    let id_exists = identity_path(store_dir).is_file();
    let pub_exists = recipient_path(store_dir).is_file();

    match (id_exists, pub_exists) {
        (true, true) => load_identity(store_dir),
        (false, false) => Ok(generate_and_save_key(store_dir)?),
        _ => Err(KeyError::PartiallyMissing),
    }
}

/// Generate a fresh keypair and write both files, OVERWRITING whatever is
/// there. Callers decide the "may I overwrite?" policy ([`ensure_key`] only
/// calls this when neither file exists; `historia keygen --encryption` gates
/// it on `--force` when one already does) - this function itself always writes.
pub fn generate_and_save_key(store_dir: &Path) -> io::Result<Identity> {
    let identity = Identity::generate();
    write_keypair(store_dir, &identity)?;
    Ok(identity)
}

/// Write both key files for `identity` atomically (write-then-rename, Rule
/// 5), age's own native plain-text encoding.
pub fn write_keypair(store_dir: &Path, identity: &Identity) -> io::Result<()> {
    let secret = identity.to_string();
    let recipient = identity.to_public().to_string();
    fsutil::write_atomic(&identity_path(store_dir), format!("{}\n", secret.expose_secret()).as_bytes())?;
    fsutil::write_atomic(&recipient_path(store_dir), format!("{recipient}\n").as_bytes())?;
    Ok(())
}

/// Load the private identity from disk. Distinguishes "neither file present"
/// ([`KeyError::Missing`]) from any other read/parse failure - `decrypt`
/// (which must never auto-generate) uses this directly rather than
/// [`ensure_key`].
pub fn load_identity(store_dir: &Path) -> Result<Identity, KeyError> {
    let text = match fs::read_to_string(identity_path(store_dir)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(KeyError::Missing),
        Err(e) => return Err(KeyError::Io(e)),
    };
    Identity::from_str(text.trim()).map_err(|_| KeyError::Invalid)
}

/// Why an encrypted backup could not be created.
#[derive(Debug)]
pub enum EncryptBackupError {
    /// The destination file already exists and `force` was not set (CLAUDE.md
    /// CP11's data-safety pattern, extended to CP15's single-file destination).
    DestinationAlreadyExists,
    Key(KeyError),
    Age(age::EncryptError),
    Io(io::Error),
}

impl fmt::Display for EncryptBackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncryptBackupError::DestinationAlreadyExists => write!(f, "the destination file already exists"),
            EncryptBackupError::Key(e) => write!(f, "{e}"),
            EncryptBackupError::Age(e) => write!(f, "{e}"),
            EncryptBackupError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EncryptBackupError {}

impl From<KeyError> for EncryptBackupError {
    fn from(e: KeyError) -> Self {
        EncryptBackupError::Key(e)
    }
}

impl From<age::EncryptError> for EncryptBackupError {
    fn from(e: age::EncryptError) -> Self {
        EncryptBackupError::Age(e)
    }
}

impl From<io::Error> for EncryptBackupError {
    fn from(e: io::Error) -> Self {
        EncryptBackupError::Io(e)
    }
}

/// Pack the whole store at `store_dir` and encrypt it to the single file
/// `dest_file` (CLAUDE.md CP15). Auto-generates an age key if this store has
/// never had one (the common case); fails with [`KeyError::PartiallyMissing`]
/// (wrapped in [`EncryptBackupError::Key`]) if a key existed and is now only
/// partly present, never silently minting a new identity. Refuses to
/// overwrite an existing `dest_file` unless `force` is set - same data-safety
/// pattern as CP11's directory backup, extended to a single-file destination.
///
/// Written atomically (write-then-rename, Rule 5): the encrypted bytes go to
/// a temp file next to `dest_file` first, and only replace it once the whole
/// encryption has finished and been flushed to disk - a failure partway
/// through never leaves a truncated (and therefore undecryptable) file at
/// `dest_file` itself.
pub fn encrypt_backup(
    store_dir: &Path,
    dest_file: &Path,
    force: bool,
) -> Result<crate::core::store::StoreComponentCounts, EncryptBackupError> {
    if dest_file.exists() && !force {
        return Err(EncryptBackupError::DestinationAlreadyExists);
    }

    let identity = ensure_key(store_dir)?;
    let recipient = identity.to_public();

    if let Some(parent) = dest_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = tmp_path_for(dest_file);

    let result = (|| -> Result<crate::core::store::StoreComponentCounts, EncryptBackupError> {
        let tmp_file = fs::File::create(&tmp_path)?;
        let encryptor = age::Encryptor::with_recipients(vec![Box::new(recipient)])
            .expect("exactly one recipient was provided");
        let mut writer = encryptor.wrap_output(tmp_file)?;
        let counts = crate::core::archive::pack_store_into(store_dir, &mut writer)?;
        let tmp_file = writer.finish()?;
        tmp_file.sync_all()?;
        Ok(counts)
    })();

    match result {
        Ok(counts) => {
            fs::rename(&tmp_path, dest_file)?;
            Ok(counts)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

fn tmp_path_for(dest_file: &Path) -> PathBuf {
    let file_name = dest_file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest_file.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()))
}

/// Why a decrypted backup could not be produced.
#[derive(Debug)]
pub enum DecryptBackupError {
    /// A store already exists at `dest_root/.historia` and `force` was not
    /// set - same semantics as CP11's `backup_store` destination refusal, so
    /// decrypting is exactly as safe against clobbering as a plain backup is.
    DestinationAlreadyExists,
    Key(KeyError),
    /// Decryption itself failed: wrong identity, or the file isn't a valid
    /// age file this identity can open. age's authenticated encryption
    /// guarantees this is reported as a clean error, never silently-wrong
    /// plaintext (CLAUDE.md CP15) - see `age::DecryptError::NoMatchingKeys`
    /// specifically for "this identity doesn't match this file".
    Decrypt(age::DecryptError),
    /// Decryption succeeded (the identity WAS right) but the resulting bytes
    /// aren't a valid historia archive - reported distinctly for a clear
    /// diagnosis rather than a confusing generic I/O error.
    InvalidArchive(io::Error),
    Io(io::Error),
}

impl fmt::Display for DecryptBackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecryptBackupError::DestinationAlreadyExists => write!(f, "a store already exists at the destination"),
            DecryptBackupError::Key(e) => write!(f, "{e}"),
            DecryptBackupError::Decrypt(e) => write!(f, "decryption failed: {e}"),
            DecryptBackupError::InvalidArchive(e) => write!(f, "{e}"),
            DecryptBackupError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DecryptBackupError {}

impl From<KeyError> for DecryptBackupError {
    fn from(e: KeyError) -> Self {
        DecryptBackupError::Key(e)
    }
}

impl From<age::DecryptError> for DecryptBackupError {
    fn from(e: age::DecryptError) -> Self {
        DecryptBackupError::Decrypt(e)
    }
}

impl From<io::Error> for DecryptBackupError {
    fn from(e: io::Error) -> Self {
        DecryptBackupError::Io(e)
    }
}

/// Decrypt `source_file` (produced by [`encrypt_backup`]) using the identity
/// found at `store_dir` (CLAUDE.md CP15 - decryption always uses the CURRENT
/// store's own age identity, never one bundled inside the file being
/// decrypted; see this module's doc comment for why that has to be true), and
/// unpack the result to `dest_root/.historia` - the same destination
/// semantics as CP11's plain `backup_store`, so `dest_root` becomes an
/// ordinary tracked-folder root. Refuses to overwrite an existing store at
/// the destination unless `force` is set. Never auto-generates a key: a
/// missing identity is [`KeyError::Missing`], not a trigger to mint a new
/// (useless, mismatched) one.
pub fn decrypt_backup(
    store_dir: &Path,
    source_file: &Path,
    dest_root: &Path,
    force: bool,
) -> Result<crate::core::store::StoreComponentCounts, DecryptBackupError> {
    let dest_store_dir = dest_root.join(crate::core::store::STORE_DIR_NAME);
    if dest_store_dir.exists() && !force {
        return Err(DecryptBackupError::DestinationAlreadyExists);
    }

    let identity = load_identity(store_dir)?;

    let file = fs::File::open(source_file)?;
    let decryptor = match age::Decryptor::new_buffered(io::BufReader::new(file))? {
        age::Decryptor::Recipients(d) => d,
        _ => return Err(DecryptBackupError::Decrypt(age::DecryptError::InvalidHeader)),
    };
    let mut stream = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))?;

    fs::create_dir_all(dest_root)?;
    let staging_dir =
        dest_root.join(format!(".{}.decrypt-{}.tmp", crate::core::store::STORE_DIR_NAME, std::process::id()));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }

    let counts = match crate::core::archive::unpack_store_from(&mut stream, &staging_dir) {
        Ok(counts) => counts,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(DecryptBackupError::InvalidArchive(e));
        }
    };

    if dest_store_dir.exists() {
        if !force {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(DecryptBackupError::DestinationAlreadyExists);
        }
        fs::remove_dir_all(&dest_store_dir)?;
    }

    if let Err(e) = fs::rename(&staging_dir, &dest_store_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(DecryptBackupError::Io(e));
    }

    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::store::init_store;
    use tempfile::tempdir;

    #[test]
    fn ensure_key_generates_a_fresh_key_when_neither_file_exists() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let identity = ensure_key(&store_dir).unwrap();

        assert!(identity_path(&store_dir).is_file());
        assert!(recipient_path(&store_dir).is_file());
        // The saved public key file matches the generated key's own public
        // half, parsed straight from disk the same way `age::x25519::Recipient
        // as FromStr` is defined (no `load_recipient` production function
        // exists for this - nothing else in the codebase needs one; §4).
        let recipient_text = fs::read_to_string(recipient_path(&store_dir)).unwrap();
        let recipient: age::x25519::Recipient = recipient_text.trim().parse().unwrap();
        assert_eq!(recipient, identity.to_public());
    }

    #[test]
    fn ensure_key_loads_an_existing_key_without_regenerating() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let original = ensure_key(&store_dir).unwrap();

        let loaded = ensure_key(&store_dir).unwrap();

        assert_eq!(loaded.to_public(), original.to_public());
    }

    #[test]
    fn ensure_key_errors_when_only_the_identity_is_missing() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        ensure_key(&store_dir).unwrap();
        fs::remove_file(identity_path(&store_dir)).unwrap();

        // Not `.unwrap_err()`: `age::x25519::Identity` (the `Ok` type) does
        // not implement `Debug`, which `unwrap_err` requires for its panic
        // message on the (here, absent) success path.
        assert!(
            matches!(ensure_key(&store_dir), Err(KeyError::PartiallyMissing)),
            "expected PartiallyMissing"
        );
    }

    #[test]
    fn ensure_key_errors_when_only_the_recipient_is_missing() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        ensure_key(&store_dir).unwrap();
        fs::remove_file(recipient_path(&store_dir)).unwrap();

        assert!(
            matches!(ensure_key(&store_dir), Err(KeyError::PartiallyMissing)),
            "expected PartiallyMissing"
        );
    }

    #[test]
    fn load_identity_reports_missing_when_neither_file_exists() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        assert!(matches!(load_identity(&store_dir), Err(KeyError::Missing)), "expected Missing");
    }

    #[test]
    fn write_keypair_then_load_identity_round_trips() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let generated = Identity::generate();

        write_keypair(&store_dir, &generated).unwrap();
        let loaded = load_identity(&store_dir).unwrap();

        assert_eq!(loaded.to_public(), generated.to_public());
    }

    #[test]
    fn any_key_file_present_is_false_for_a_fresh_store() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        assert!(!any_key_file_present(&store_dir));
    }

    #[test]
    fn any_key_file_present_is_true_after_ensure_key() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        ensure_key(&store_dir).unwrap();

        assert!(any_key_file_present(&store_dir));
    }

    // ---- encrypt_backup / decrypt_backup (CP15) ----

    fn commit_sample(store_dir: &Path) {
        let hash = crate::core::store::write_blob(store_dir, &mut &b"hello, encrypted world"[..]).unwrap();
        let manifest = crate::format::manifest::Manifest {
            number: 1,
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            message: "test".to_string(),
            parent: 0,
            parent_hash: None,
            entries: vec![crate::format::manifest::Entry { path: "a.txt".into(), hash, mode: 0o644 }],
        };
        crate::core::snapshot::write_manifest(store_dir, &manifest).unwrap();
        crate::core::snapshot::write_head(store_dir, 1).unwrap();
    }

    #[test]
    fn encrypt_then_decrypt_round_trips_to_a_matching_store() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        commit_sample(&store_dir);

        let backup_dir = tempdir().unwrap();
        let encrypted_file = backup_dir.path().join("backup.age");
        let encrypt_counts = encrypt_backup(&store_dir, &encrypted_file, false).unwrap();

        // The output is one opaque file - not a directory - and its bytes do
        // not contain the plaintext manifest message anywhere.
        assert!(encrypted_file.is_file());
        let encrypted_bytes = fs::read(&encrypted_file).unwrap();
        assert!(!String::from_utf8_lossy(&encrypted_bytes).contains("hello, encrypted world"));

        let dest_dir = tempdir().unwrap();
        let decrypt_counts = decrypt_backup(&store_dir, &encrypted_file, dest_dir.path(), false).unwrap();

        assert_eq!(encrypt_counts, decrypt_counts);
        let restored_store = dest_dir.path().join(crate::core::store::STORE_DIR_NAME);
        assert_eq!(fs::read_to_string(restored_store.join("HEAD")).unwrap(), "1\n");
        assert_eq!(
            fs::read_to_string(restored_store.join("snapshots").join("1.json")).unwrap(),
            fs::read_to_string(store_dir.join("snapshots").join("1.json")).unwrap()
        );
    }

    #[test]
    fn encrypt_backup_auto_generates_a_key_if_none_existed() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        assert!(!any_key_file_present(&store_dir));

        let backup_dir = tempdir().unwrap();
        encrypt_backup(&store_dir, &backup_dir.path().join("backup.age"), false).unwrap();

        assert!(any_key_file_present(&store_dir));
    }

    #[test]
    fn encrypt_backup_fails_cleanly_if_the_key_is_only_partially_present() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        ensure_key(&store_dir).unwrap();
        fs::remove_file(identity_path(&store_dir)).unwrap();

        let backup_dir = tempdir().unwrap();
        let err = encrypt_backup(&store_dir, &backup_dir.path().join("backup.age"), false).unwrap_err();

        assert!(matches!(err, EncryptBackupError::Key(KeyError::PartiallyMissing)), "got {err:?}");
    }

    #[test]
    fn encrypt_backup_refuses_an_existing_destination_without_force() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let backup_dir = tempdir().unwrap();
        let dest = backup_dir.path().join("backup.age");
        fs::write(&dest, b"pre-existing content").unwrap();

        let err = encrypt_backup(&store_dir, &dest, false).unwrap_err();

        assert!(matches!(err, EncryptBackupError::DestinationAlreadyExists));
        assert_eq!(fs::read(&dest).unwrap(), b"pre-existing content");
    }

    #[test]
    fn encrypt_backup_with_force_overwrites_an_existing_destination() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let backup_dir = tempdir().unwrap();
        let dest = backup_dir.path().join("backup.age");
        fs::write(&dest, b"pre-existing content").unwrap();

        encrypt_backup(&store_dir, &dest, true).unwrap();

        assert_ne!(fs::read(&dest).unwrap(), b"pre-existing content");
    }

    #[test]
    fn decrypting_with_the_wrong_key_fails_cleanly() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        commit_sample(&store_dir);
        let backup_dir = tempdir().unwrap();
        let encrypted_file = backup_dir.path().join("backup.age");
        encrypt_backup(&store_dir, &encrypted_file, false).unwrap();

        // A second, unrelated store with its own (different) identity.
        let other_dir = tempdir().unwrap();
        let other_store = init_store(other_dir.path()).unwrap();
        ensure_key(&other_store).unwrap();

        let dest_dir = tempdir().unwrap();
        let err = decrypt_backup(&other_store, &encrypted_file, dest_dir.path(), false).unwrap_err();

        assert!(matches!(err, DecryptBackupError::Decrypt(_)), "got {err:?}");
        assert!(
            !dest_dir.path().join(crate::core::store::STORE_DIR_NAME).exists(),
            "a failed decrypt must not leave a partial store behind"
        );
    }

    #[test]
    fn decrypting_without_any_key_fails_cleanly() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        commit_sample(&store_dir);
        let backup_dir = tempdir().unwrap();
        let encrypted_file = backup_dir.path().join("backup.age");
        encrypt_backup(&store_dir, &encrypted_file, false).unwrap();

        // A fresh store with no age key at all.
        let other_dir = tempdir().unwrap();
        let other_store = init_store(other_dir.path()).unwrap();

        let dest_dir = tempdir().unwrap();
        let err = decrypt_backup(&other_store, &encrypted_file, dest_dir.path(), false).unwrap_err();

        assert!(matches!(err, DecryptBackupError::Key(KeyError::Missing)), "got {err:?}");
    }

    #[test]
    fn decrypt_backup_refuses_an_existing_destination_store_without_force() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        commit_sample(&store_dir);
        let backup_dir = tempdir().unwrap();
        let encrypted_file = backup_dir.path().join("backup.age");
        encrypt_backup(&store_dir, &encrypted_file, false).unwrap();

        let dest_dir = tempdir().unwrap();
        init_store(dest_dir.path()).unwrap();
        let head_before =
            fs::read_to_string(dest_dir.path().join(crate::core::store::STORE_DIR_NAME).join("HEAD")).unwrap();

        let err = decrypt_backup(&store_dir, &encrypted_file, dest_dir.path(), false).unwrap_err();

        assert!(matches!(err, DecryptBackupError::DestinationAlreadyExists));
        assert_eq!(
            fs::read_to_string(dest_dir.path().join(crate::core::store::STORE_DIR_NAME).join("HEAD")).unwrap(),
            head_before
        );
    }

    fn collect_store_bytes(store_dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn collect(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    collect(root, &path, out);
                } else if path.is_file() {
                    let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        collect(store_dir, store_dir, &mut out);
        out
    }

    #[test]
    fn source_store_is_unchanged_after_encrypt_backup_beyond_key_auto_generation() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        commit_sample(&store_dir);
        // Pre-generate the key so encrypt_backup's own auto-generation step
        // (the ONE expected mutation on a never-encrypted-before store) is
        // out of the way before taking the "before" snapshot - this test is
        // about everything else staying untouched.
        ensure_key(&store_dir).unwrap();
        let before = collect_store_bytes(&store_dir);

        let backup_dir = tempdir().unwrap();
        encrypt_backup(&store_dir, &backup_dir.path().join("backup.age"), false).unwrap();

        assert_eq!(collect_store_bytes(&store_dir), before, "encrypt_backup must not modify the source store");
    }
}
