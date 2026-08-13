//! Locate and open `.historia/` (walking up from the current directory, like git);
//! read and write content-addressed blobs under `objects/` by their hash; the
//! `.historia/lock` primitive (acquire/release, fail-fast, stale-lock reporting).
//! CLAUDE.md §8 (core/store.rs), §9 (on-disk layout), Rule 6 (locking).

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::hash::StreamHasher;
use crate::core::{fsutil, snapshot};
use crate::format::manifest;

/// Name of the store directory under a tracked folder (CLAUDE.md §9).
pub const STORE_DIR_NAME: &str = ".historia";

/// Why `init_store` could not create a new store.
#[derive(Debug)]
pub enum InitError {
    /// A store already exists at the target - `init_store` never overwrites it.
    AlreadyExists,
    /// Some other filesystem operation failed while building the store.
    Io(io::Error),
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InitError::AlreadyExists => write!(f, "store already exists"),
            InitError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InitError {}

impl From<io::Error> for InitError {
    fn from(e: io::Error) -> Self {
        InitError::Io(e)
    }
}

/// Create a fresh `.historia/` store under `target_dir` (which must already exist):
/// `objects/`, `snapshots/`, `format`, and `HEAD` (CLAUDE.md §9). `lock` is
/// deliberately not created here - it exists only while a `commit`/`restore` is
/// running (CP2/CP3, Rule 6).
///
/// The whole store is built in a staging directory next to the target and then
/// moved into place with a single rename, so a failure partway through never
/// leaves a `.historia/` that reads as valid but is actually incomplete (CLAUDE.md
/// §6 requirement, Rule 5's spirit applied to the whole store, not just one file).
pub fn init_store(target_dir: &Path) -> Result<PathBuf, InitError> {
    let store_dir = target_dir.join(STORE_DIR_NAME);
    if store_dir.exists() {
        return Err(InitError::AlreadyExists);
    }

    let staging_dir = target_dir.join(format!(".{STORE_DIR_NAME}.init-{}.tmp", std::process::id()));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }

    if let Err(e) = build_staging_store(&staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(e);
    }

    // Re-check right before the rename: another process could have won an init
    // race while we were building the staging dir. Back off rather than clobber.
    if store_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(InitError::AlreadyExists);
    }

    if let Err(e) = fs::rename(&staging_dir, &store_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(InitError::Io(e));
    }

    Ok(store_dir)
}

fn build_staging_store(staging_dir: &Path) -> Result<(), InitError> {
    fs::create_dir_all(staging_dir.join("objects"))?;
    fs::create_dir_all(staging_dir.join("snapshots"))?;
    fsutil::write_atomic(&staging_dir.join("format"), manifest::FORMAT_MARKER.as_bytes())?;
    fsutil::write_atomic(&staging_dir.join("HEAD"), snapshot::INITIAL_HEAD.as_bytes())?;
    Ok(())
}

// ---- content-addressed blob store ----

/// Monotonic counter mixed into temp-blob file names, so concurrent `write_blob`
/// calls within one process never collide (a bare PID is not unique per-call).
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The final on-disk path for a blob, sharded by hash prefix: `objects/ab/cdef...`
/// (CLAUDE.md §9). `hash` is always a 64-char SHA-256 hex digest in practice, but
/// this stays safe even given a shorter string.
fn blob_path(store_dir: &Path, hash: &str) -> PathBuf {
    let split = 2.min(hash.len());
    let (prefix, rest) = hash.split_at(split);
    store_dir.join("objects").join(prefix).join(rest)
}

/// True if a blob with this hash is already stored.
///
/// Not called yet - `commit` doesn't need it (`write_blob` already dedups
/// internally); `verify` (CP8) is the first real caller.
#[allow(dead_code)]
pub fn has_blob(store_dir: &Path, hash: &str) -> bool {
    blob_path(store_dir, hash).is_file()
}

/// Stream `reader` into the content-addressed object store under
/// `store_dir/objects/`, sharded by hash prefix, and return its hex SHA-256 hash.
/// The content is hashed and written to a temp file in a single pass (its final
/// name isn't known until hashing finishes), then moved into place with one atomic
/// rename (Rule 5). A no-op beyond hashing if the blob already exists - content
/// addressing gives deduplication for free (CLAUDE.md §9). Streams in fixed-size
/// chunks throughout, never holding the whole input in memory (Rule 11).
pub fn write_blob<R: Read>(store_dir: &Path, reader: &mut R) -> io::Result<String> {
    let objects_dir = store_dir.join("objects");
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = objects_dir.join(format!(".tmp-{}-{n}", std::process::id()));

    let write_result = (|| -> io::Result<String> {
        let mut tmp_file = fs::File::create(&tmp_path)?;
        let mut hasher = StreamHasher::new();
        crate::core::hash::for_each_chunk(reader, |chunk| {
            hasher.update(chunk);
            tmp_file.write_all(chunk)
        })?;
        tmp_file.sync_all()?;
        Ok(hasher.finish())
    })();

    let hash = match write_result {
        Ok(hash) => hash,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
    };

    let dest = blob_path(store_dir, &hash);
    if dest.is_file() {
        fs::remove_file(&tmp_path)?;
        return Ok(hash);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&tmp_path, &dest)?;
    Ok(hash)
}

/// Open a streaming reader onto the blob with the given hash.
///
/// Not called yet - `restore` (CP6) and `verify` (CP8) are the first real callers.
#[allow(dead_code)]
pub fn open_blob(store_dir: &Path, hash: &str) -> io::Result<fs::File> {
    fs::File::open(blob_path(store_dir, hash))
}

// ---- store discovery ----

/// Walk up from `start` looking for a `.historia/` directory, the way git finds
/// `.git/` from any subfolder - so every command can be run from a subdirectory of
/// the tracked folder, not just its root (CLAUDE.md §5).
pub fn locate_store(start: &Path) -> Option<PathBuf> {
    let mut dir = start.canonicalize().ok()?;
    loop {
        let candidate = dir.join(STORE_DIR_NAME);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ---- lock (Rule 6) ----

/// The `.historia/lock` primitive: `acquire` fails fast (never waits) if the store
/// is already locked, and reports a clearly stale lock instead of either blocking
/// or silently clearing it. `LockGuard` releases the lock on an explicit `release()`
/// or automatically on drop (including an early `?` return), so a command can never
/// forget to unlock the store.
pub mod lock {
    use std::fmt;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// Name of the lock file under `.historia/` (CLAUDE.md §9).
    pub const LOCK_FILE_NAME: &str = "lock";

    /// A lock file older than this is treated as stale even though we cannot
    /// portably check whether its PID is still alive without a new dependency
    /// (Rule 9: minimal dependencies) - age is the portable signal `std` gives us.
    const STALE_AGE: Duration = Duration::from_secs(60 * 60 * 12);

    /// Why `acquire` could not lock the store.
    #[derive(Debug)]
    pub enum LockError {
        /// Locked by what looks like a live, recent process.
        Held { pid: u32, since: SystemTime },
        /// Locked by what looks like a dead or abandoned process (old timestamp).
        /// Reported rather than silently cleared - the caller decides what to do.
        Stale { pid: u32, since: SystemTime },
        Io(io::Error),
    }

    impl fmt::Display for LockError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                LockError::Held { pid, since } => write!(
                    f,
                    "store is locked (held by process {pid}, {}); try again once it finishes",
                    age_description(*since)
                ),
                LockError::Stale { pid, since } => write!(
                    f,
                    "found a stale lock (process {pid} looks long gone, {}); remove .historia/{LOCK_FILE_NAME} manually if you're sure no historia process is running",
                    age_description(*since)
                ),
                LockError::Io(e) => write!(f, "{e}"),
            }
        }
    }

    /// A short human-readable description of how long ago `since` was, for lock
    /// error messages (e.g. "locked 3s ago").
    fn age_description(since: SystemTime) -> String {
        match SystemTime::now().duration_since(since) {
            Ok(age) => format!("locked {}s ago", age.as_secs()),
            Err(_) => "locked just now".to_string(),
        }
    }

    impl std::error::Error for LockError {}

    impl From<io::Error> for LockError {
        fn from(e: io::Error) -> Self {
            LockError::Io(e)
        }
    }

    /// Holds `.historia/lock` for as long as it is alive. Release explicitly with
    /// [`release`](LockGuard::release), or just let it drop - either way the lock
    /// file is removed exactly once.
    #[derive(Debug)]
    pub struct LockGuard {
        path: PathBuf,
        released: bool,
    }

    impl LockGuard {
        /// Release the lock now, observing any error removing the file.
        pub fn release(mut self) -> io::Result<()> {
            self.released = true;
            fs::remove_file(&self.path)
        }
    }

    impl Drop for LockGuard {
        fn drop(&mut self) {
            if !self.released {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    /// Acquire `store_dir/lock`. Fails fast (no waiting/retrying) if already held:
    /// [`LockError::Held`] for what looks like a live lock, [`LockError::Stale`]
    /// for one old enough to be almost certainly abandoned (CLAUDE.md Rule 6).
    pub fn acquire(store_dir: &Path) -> Result<LockGuard, LockError> {
        let path = store_dir.join(LOCK_FILE_NAME);

        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                use std::io::Write;
                let pid = std::process::id();
                let since = SystemTime::now();
                file.write_all(format_contents(pid, since).as_bytes())?;
                file.sync_all()?;
                Ok(LockGuard { path, released: false })
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let (pid, since) = read_contents(&path)?;
                if is_stale(since) {
                    Err(LockError::Stale { pid, since })
                } else {
                    Err(LockError::Held { pid, since })
                }
            }
            Err(e) => Err(LockError::Io(e)),
        }
    }

    fn is_stale(since: SystemTime) -> bool {
        match SystemTime::now().duration_since(since) {
            Ok(age) => age > STALE_AGE,
            // `since` is in the future (clock skew) - not stale, be conservative.
            Err(_) => false,
        }
    }

    /// Lock file contents: PID on the first line, Unix timestamp (seconds) on the
    /// second - plain decimal text, trivially parseable by hand (CLAUDE.md §8).
    fn format_contents(pid: u32, since: SystemTime) -> String {
        let secs = since.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        format!("{pid}\n{secs}\n")
    }

    fn read_contents(path: &Path) -> io::Result<(u32, SystemTime)> {
        let contents = fs::read_to_string(path)?;
        let mut lines = contents.lines();
        let pid: u32 = lines
            .next()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "lock file: missing or invalid PID"))?;
        let secs: u64 = lines
            .next()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "lock file: missing or invalid timestamp"))?;
        Ok((pid, UNIX_EPOCH + Duration::from_secs(secs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn creates_the_full_store_layout() {
        let dir = tempdir().unwrap();

        let store_dir = init_store(dir.path()).unwrap();

        assert_eq!(store_dir, dir.path().join(STORE_DIR_NAME));
        assert!(store_dir.join("objects").is_dir());
        assert!(store_dir.join("snapshots").is_dir());
        assert!(!store_dir.join("lock").exists());
        assert_eq!(
            fs::read_to_string(store_dir.join("format")).unwrap(),
            crate::format::manifest::FORMAT_MARKER
        );
        assert_eq!(
            fs::read_to_string(store_dir.join("HEAD")).unwrap(),
            crate::core::snapshot::INITIAL_HEAD
        );
    }

    #[test]
    fn second_init_fails_and_leaves_first_store_untouched() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let head_before = fs::read_to_string(store_dir.join("HEAD")).unwrap();
        let format_before = fs::read_to_string(store_dir.join("format")).unwrap();

        let err = init_store(dir.path()).unwrap_err();

        assert!(matches!(err, InitError::AlreadyExists));
        assert_eq!(fs::read_to_string(store_dir.join("HEAD")).unwrap(), head_before);
        assert_eq!(fs::read_to_string(store_dir.join("format")).unwrap(), format_before);
    }

    #[test]
    fn leaves_no_staging_leftovers_after_success() {
        let dir = tempdir().unwrap();

        init_store(dir.path()).unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .filter(|n| n != STORE_DIR_NAME)
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
    }

    // ---- blob store ----

    #[test]
    fn write_blob_then_open_blob_round_trips_content() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let hash = write_blob(&store_dir, &mut &b"hello, historia"[..]).unwrap();

        let mut buf = Vec::new();
        open_blob(&store_dir, &hash).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"hello, historia");
    }

    #[test]
    fn write_blob_shards_by_hash_prefix() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let hash = write_blob(&store_dir, &mut &b"shard me"[..]).unwrap();

        let expected = store_dir.join("objects").join(&hash[..2]).join(&hash[2..]);
        assert!(expected.is_file(), "expected blob at {expected:?}");
    }

    #[test]
    fn writing_identical_content_twice_dedups() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let hash1 = write_blob(&store_dir, &mut &b"same content"[..]).unwrap();

        let objects_before: Vec<_> = walk_objects(&store_dir);
        let hash2 = write_blob(&store_dir, &mut &b"same content"[..]).unwrap();
        let objects_after: Vec<_> = walk_objects(&store_dir);

        assert_eq!(hash1, hash2);
        assert_eq!(objects_before, objects_after, "dedup must not add a new object");
    }

    #[test]
    fn write_blob_streams_a_large_input_without_reading_it_all_first() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        struct ChunkCapReader {
            remaining: usize,
        }
        impl Read for ChunkCapReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                assert!(buf.len() <= crate::core::hash::CHUNK_SIZE, "write_blob read more than one chunk at once");
                let n = buf.len().min(self.remaining);
                buf[..n].fill(0x7);
                self.remaining -= n;
                Ok(n)
            }
        }

        let total = crate::core::hash::CHUNK_SIZE * 4 + 7;
        let mut reader = ChunkCapReader { remaining: total };
        let hash = write_blob(&store_dir, &mut reader).unwrap();

        let mut buf = Vec::new();
        open_blob(&store_dir, &hash).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), total);
    }

    fn walk_objects(store_dir: &Path) -> Vec<PathBuf> {
        let objects = store_dir.join("objects");
        let mut paths = Vec::new();
        for shard in fs::read_dir(&objects).unwrap() {
            let shard = shard.unwrap().path();
            if shard.is_dir() {
                for entry in fs::read_dir(&shard).unwrap() {
                    paths.push(entry.unwrap().path());
                }
            }
        }
        paths.sort();
        paths
    }

    // ---- store discovery ----

    #[test]
    fn locate_store_finds_it_from_a_nested_subdir() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();

        let found = locate_store(&nested).unwrap();

        assert_eq!(found, store_dir.canonicalize().unwrap());
    }

    #[test]
    fn locate_store_finds_the_nearest_store_not_a_farther_one() {
        let dir = tempdir().unwrap();
        init_store(dir.path()).unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        let nested_store = init_store(&nested).unwrap();

        let found = locate_store(&nested).unwrap();

        assert_eq!(found, nested_store.canonicalize().unwrap());
    }

    #[test]
    fn locate_store_returns_none_when_no_store_is_found_within_a_bounded_tree() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("x").join("y");
        fs::create_dir_all(&nested).unwrap();

        assert!(locate_store(&nested).is_none());
    }

    // ---- lock ----

    #[test]
    fn acquire_then_release_allows_a_later_acquire() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let guard = lock::acquire(&store_dir).unwrap();
        guard.release().unwrap();

        assert!(lock::acquire(&store_dir).is_ok());
    }

    #[test]
    fn dropping_the_guard_releases_the_lock() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        {
            let _guard = lock::acquire(&store_dir).unwrap();
            assert!(store_dir.join(lock::LOCK_FILE_NAME).exists());
        }

        assert!(!store_dir.join(lock::LOCK_FILE_NAME).exists());
        assert!(lock::acquire(&store_dir).is_ok());
    }

    #[test]
    fn a_held_lock_cannot_be_acquired_by_a_second_attempt() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let _guard = lock::acquire(&store_dir).unwrap();
        let err = lock::acquire(&store_dir).unwrap_err();

        assert!(matches!(err, lock::LockError::Held { .. }), "expected Held, got {err:?}");
    }

    #[test]
    fn a_clearly_stale_lock_is_reported_not_silently_ignored() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let lock_path = store_dir.join(lock::LOCK_FILE_NAME);

        let ancient = std::time::SystemTime::UNIX_EPOCH;
        let secs = ancient
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(&lock_path, format!("999999\n{secs}\n")).unwrap();

        let err = lock::acquire(&store_dir).unwrap_err();

        assert!(matches!(err, lock::LockError::Stale { .. }), "expected Stale, got {err:?}");
    }
}
