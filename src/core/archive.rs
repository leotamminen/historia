//! Whole-store packing/unpacking for CP15's encrypted backups: a minimal,
//! std-only "poor man's tar" - the store's durable components
//! (`core::store::for_each_component`, CLAUDE.md §9) concatenated into one
//! byte stream, with just enough structure to reconstitute them. No tar
//! crate: the format only ever needs to round-trip within this binary, so a
//! small hand-rolled layout avoids a dependency neither Cargo.toml nor Rule 9
//! need (CLAUDE.md §7, CP15).
//!
//! **Format ("historia archive v1"):** a fixed magic string, then zero or
//! more entries back to back until end-of-stream. Each entry is:
//!   - 4 bytes, little-endian u32: the length of the entry's path, in bytes
//!   - that many bytes: the path (UTF-8, forward-slash separated, relative to
//!     the store root - e.g. `"objects/ab/cdef..."`, `"snapshots/1.json"`, `"HEAD"`)
//!   - 8 bytes, little-endian u64: the length of the entry's content, in bytes
//!   - that many bytes: the raw file content
//!
//! No entry count and no footer: both the packer and the unpacker only ever
//! need to stream forward (Rule 11 - never buffering a whole archive, or a
//! whole file within it, in memory), and "keep reading entries until a clean
//! end-of-stream" needs neither.
//!
//! This format is never itself a durability contract the way the manifest
//! JSON is (CLAUDE.md §9): it exists only as a transient encoding on the way
//! into and back out of age encryption, never stored as a first-class file of
//! its own.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::core::store::{self, StoreComponentCounts};

const ARCHIVE_MAGIC: &[u8] = b"historia archive v1\n";

/// Fixed-size streaming copy buffer (mirrors `core::hash::CHUNK_SIZE`'s
/// role): reused so unpacking a large blob never holds more than one chunk of
/// it in memory at a time (Rule 11).
const COPY_CHUNK_SIZE: usize = 64 * 1024;

/// Pack every durable component of the store at `store_dir` into `writer`,
/// streaming each file's content directly rather than buffering it (Rule 11).
pub fn pack_store_into<W: Write>(store_dir: &Path, writer: &mut W) -> io::Result<StoreComponentCounts> {
    writer.write_all(ARCHIVE_MAGIC)?;
    store::for_each_component(store_dir, |relative_path, absolute_path| {
        write_entry(writer, relative_path, absolute_path)
    })
}

fn write_entry<W: Write>(writer: &mut W, relative_path: &str, absolute_path: &Path) -> io::Result<()> {
    let path_bytes = relative_path.as_bytes();
    writer.write_all(&(path_bytes.len() as u32).to_le_bytes())?;
    writer.write_all(path_bytes)?;

    let expected_len = fs::metadata(absolute_path)?.len();
    writer.write_all(&expected_len.to_le_bytes())?;

    let mut file = fs::File::open(absolute_path)?;
    let copied = io::copy(&mut file, writer)?;
    if copied != expected_len {
        // The file changed size between the metadata() call above and the
        // copy - vanishingly unlikely for a historia store's append-only
        // objects and atomically-written manifests, but report it as a clear
        // error rather than silently writing a corrupt entry.
        return Err(io::Error::other(format!(
            "'{relative_path}' changed size while being archived (expected {expected_len} bytes, copied {copied})"
        )));
    }
    Ok(())
}

/// Unpack an archive produced by [`pack_store_into`] from `reader`, writing
/// every entry under `dest_store_dir` (creating parent directories as
/// needed). Streams throughout (Rule 11): never holds a whole entry, let
/// alone the whole archive, in memory.
pub fn unpack_store_from<R: Read>(reader: &mut R, dest_store_dir: &Path) -> io::Result<StoreComponentCounts> {
    let mut magic_buf = vec![0u8; ARCHIVE_MAGIC.len()];
    reader.read_exact(&mut magic_buf).map_err(|_| invalid_archive("truncated or empty input"))?;
    if magic_buf != ARCHIVE_MAGIC {
        return Err(invalid_archive("bad magic - not a historia archive"));
    }

    let mut counts = StoreComponentCounts::default();
    let mut copy_buf = vec![0u8; COPY_CHUNK_SIZE];

    loop {
        let mut len_buf = [0u8; 4];
        if !try_fill(reader, &mut len_buf)? {
            break; // clean end of archive, exactly at an entry boundary
        }
        let path_len = u32::from_le_bytes(len_buf) as usize;

        let mut path_buf = vec![0u8; path_len];
        reader.read_exact(&mut path_buf).map_err(|_| invalid_archive("truncated entry path"))?;
        let relative_path =
            String::from_utf8(path_buf).map_err(|_| invalid_archive("entry path is not valid UTF-8"))?;

        let mut content_len_buf = [0u8; 8];
        reader
            .read_exact(&mut content_len_buf)
            .map_err(|_| invalid_archive("truncated entry length"))?;
        let content_len = u64::from_le_bytes(content_len_buf);

        let dest_path = resolve_entry_path(dest_store_dir, &relative_path)?;
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut dest_file = fs::File::create(&dest_path)?;
        copy_exact(reader, &mut dest_file, content_len, &mut copy_buf)
            .map_err(|_| invalid_archive("truncated entry content"))?;
        dest_file.sync_all()?;

        if relative_path.starts_with("objects/") {
            counts.objects += 1;
        } else if relative_path.starts_with("snapshots/") && relative_path.ends_with(".json") {
            counts.snapshots += 1;
        }
    }

    Ok(counts)
}

/// Join `relative_path` onto `root`, refusing anything that would escape
/// `root` (e.g. a `..` component) - defense in depth. In practice every
/// archive this binary ever produces only contains paths
/// `core::store::for_each_component` generated itself, which are always
/// well-formed, but an archive is just bytes from a file on disk by the time
/// it reaches this function, so it is treated with the same suspicion as any
/// other untrusted input.
fn resolve_entry_path(root: &Path, relative_path: &str) -> io::Result<PathBuf> {
    if relative_path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(invalid_archive(format!("unsafe entry path '{relative_path}'")));
    }
    Ok(root.join(relative_path))
}

fn invalid_archive(reason: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("not a valid historia archive: {}", reason.into()))
}

/// Fill `buf` completely from `reader`. Returns `Ok(true)` if it succeeded,
/// `Ok(false)` if end-of-stream was hit with zero bytes read (a clean end,
/// exactly at an entry boundary) - distinguishing that from a partial read
/// (which IS corruption) is why this isn't just `read_exact`, which can't
/// tell the two apart.
fn try_fill<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<bool> {
    let mut total = 0;
    while total < buf.len() {
        let n = reader.read(&mut buf[total..])?;
        if n == 0 {
            if total == 0 {
                return Ok(false);
            }
            return Err(invalid_archive("truncated entry header"));
        }
        total += n;
    }
    Ok(true)
}

/// Copy exactly `remaining` bytes from `reader` to `writer`, in fixed-size
/// chunks via `buf` (Rule 11 - never the whole entry at once).
fn copy_exact<R: Read, W: Write>(reader: &mut R, writer: &mut W, mut remaining: u64, buf: &mut [u8]) -> io::Result<()> {
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        reader.read_exact(&mut buf[..want])?;
        writer.write_all(&buf[..want])?;
        remaining -= want as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::store::init_store;
    use tempfile::tempdir;

    #[test]
    fn pack_then_unpack_round_trips_a_committed_store() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        let hash = crate::core::store::write_blob(&store_dir, &mut &b"hello archive"[..]).unwrap();
        let manifest = crate::format::manifest::Manifest {
            number: 1,
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            message: "test".to_string(),
            parent: 0,
            parent_hash: None,
            entries: vec![crate::format::manifest::Entry { path: "a.txt".into(), hash, mode: 0o644 }],
        };
        crate::core::snapshot::write_manifest(&store_dir, &manifest).unwrap();
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        let mut buf = Vec::new();
        let pack_counts = pack_store_into(&store_dir, &mut buf).unwrap();

        let dest_dir = tempdir().unwrap();
        let dest_store = dest_dir.path().join("restored");
        let unpack_counts = unpack_store_from(&mut &buf[..], &dest_store).unwrap();

        assert_eq!(pack_counts, unpack_counts);
        assert_eq!(fs::read_to_string(dest_store.join("HEAD")).unwrap(), "1\n");
        assert_eq!(
            fs::read_to_string(dest_store.join("format")).unwrap(),
            crate::format::manifest::FORMAT_MARKER
        );
        assert_eq!(fs::read_to_string(dest_store.join("snapshots").join("1.json")).unwrap(), fs::read_to_string(store_dir.join("snapshots").join("1.json")).unwrap());
    }

    #[test]
    fn pack_then_unpack_round_trips_a_large_blob() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        let content = vec![0xABu8; COPY_CHUNK_SIZE * 3 + 17];
        let hash = crate::core::store::write_blob(&store_dir, &mut &content[..]).unwrap();

        let mut buf = Vec::new();
        pack_store_into(&store_dir, &mut buf).unwrap();

        let dest_dir = tempdir().unwrap();
        let dest_store = dest_dir.path().join("restored");
        unpack_store_from(&mut &buf[..], &dest_store).unwrap();

        let blob_path = dest_store.join("objects").join(&hash[..2]).join(&hash[2..]);
        assert_eq!(fs::read(blob_path).unwrap(), content);
    }

    #[test]
    fn unpack_rejects_empty_input() {
        let dest_dir = tempdir().unwrap();
        let err = unpack_store_from(&mut &b""[..], &dest_dir.path().join("x")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn unpack_rejects_bad_magic() {
        let dest_dir = tempdir().unwrap();
        let err = unpack_store_from(&mut &b"not a historia archive at all!!"[..], &dest_dir.path().join("x")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn unpack_rejects_a_truncated_entry() {
        let source_dir = tempdir().unwrap();
        let store_dir = init_store(source_dir.path()).unwrap();
        crate::core::store::write_blob(&store_dir, &mut &b"some content"[..]).unwrap();

        let mut buf = Vec::new();
        pack_store_into(&store_dir, &mut buf).unwrap();
        buf.truncate(buf.len() - 5); // cut off the middle of the last entry

        let dest_dir = tempdir().unwrap();
        let err = unpack_store_from(&mut &buf[..], &dest_dir.path().join("x")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn unpack_rejects_a_path_traversal_attempt() {
        // Hand-craft a malicious archive: valid magic, one entry whose path
        // tries to escape the destination directory.
        let mut buf = Vec::new();
        buf.extend_from_slice(ARCHIVE_MAGIC);
        let evil_path = b"../../evil.txt";
        buf.extend_from_slice(&(evil_path.len() as u32).to_le_bytes());
        buf.extend_from_slice(evil_path);
        buf.extend_from_slice(&4u64.to_le_bytes());
        buf.extend_from_slice(b"evil");

        let dest_dir = tempdir().unwrap();
        let err = unpack_store_from(&mut &buf[..], &dest_dir.path().join("x")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(!dest_dir.path().join("evil.txt").exists(), "must never escape the destination");
    }
}
