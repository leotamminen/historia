//! Streaming content hashing (SHA-256). Hashes files in fixed-size chunks rather
//! than reading them whole into memory, so a large tracked file costs O(chunk size)
//! RAM, not O(file size) (Rule 11).

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

/// Fixed read-buffer size for streaming: files are hashed (and, in `store`, written)
/// in pieces this large rather than loaded whole into memory (Rule 11).
pub const CHUNK_SIZE: usize = 64 * 1024;

/// Read `reader` in `CHUNK_SIZE` pieces, calling `visit` with each piece in turn.
/// The shared primitive behind both hashing and blob writing, so both stream an
/// input exactly once without ever holding more than one chunk in memory.
pub fn for_each_chunk<R: Read>(
    reader: &mut R,
    mut visit: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        visit(&buf[..n])?;
    }
    Ok(())
}

/// A SHA-256 hash accumulated incrementally, one chunk at a time. Keeps the choice
/// of hash algorithm local to this module - callers (e.g. `store::write_blob`) feed
/// it chunks without depending on `sha2` themselves.
#[derive(Default)]
pub struct StreamHasher(Sha256);

impl StreamHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.0.update(chunk);
    }

    /// Consume the hasher and return its digest as a lowercase hex string.
    pub fn finish(self) -> String {
        hex_encode(&self.0.finalize())
    }
}

/// Stream `reader` in fixed-size chunks and return its SHA-256 digest as a
/// lowercase hex string. Memory use is O(chunk size), not O(input size) (Rule 11).
///
/// Not called outside tests yet - `commit` hashes via `store::write_blob`, which
/// hashes and writes in the same pass instead of hashing alone. `status` (CP5) and
/// `verify` (CP8), which need a hash without writing a blob, are the first real
/// callers.
#[allow(dead_code)]
pub fn hash_reader<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut hasher = StreamHasher::new();
    for_each_chunk(reader, |chunk| {
        hasher.update(chunk);
        Ok(())
    })?;
    Ok(hasher.finish())
}

/// Stream the file at `path` and return its SHA-256 digest as a lowercase hex
/// string. See [`hash_reader`] for why this isn't called outside tests yet.
#[allow(dead_code)]
pub fn hash_file(path: &Path) -> io::Result<String> {
    hash_reader(&mut File::open(path)?)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(&mut s, "{b:02x}").expect("writing to a String never fails");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use tempfile::tempdir;

    // FIPS 180-2 test vectors - independent of this crate's implementation.
    const SHA256_EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn hash_reader_matches_known_test_vectors() {
        assert_eq!(hash_reader(&mut &b""[..]).unwrap(), SHA256_EMPTY);
        assert_eq!(hash_reader(&mut &b"abc"[..]).unwrap(), SHA256_ABC);
    }

    #[test]
    fn hash_file_matches_hash_reader() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, b"abc").unwrap();

        assert_eq!(hash_file(&path).unwrap(), SHA256_ABC);
    }

    /// A `Read` that panics if asked for more than `CHUNK_SIZE` bytes in one call,
    /// proving `hash_reader` never pulls more than one chunk into memory at a time
    /// (Rule 11) rather than merely happening to produce the right digest.
    struct ChunkSizeAssertingReader {
        remaining: usize,
    }

    impl Read for ChunkSizeAssertingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            assert!(
                buf.len() <= CHUNK_SIZE,
                "hash_reader requested {} bytes in one read, more than CHUNK_SIZE ({CHUNK_SIZE})",
                buf.len()
            );
            let n = buf.len().min(self.remaining);
            for b in &mut buf[..n] {
                *b = 0xAB;
            }
            self.remaining -= n;
            Ok(n)
        }
    }

    #[test]
    fn hash_reader_never_reads_more_than_one_chunk_at_a_time() {
        let total = CHUNK_SIZE * 3 + 1;
        let mut reader = ChunkSizeAssertingReader { remaining: total };

        let streamed = hash_reader(&mut reader).unwrap();

        let expected_bytes = vec![0xABu8; total];
        let expected = hash_reader(&mut &expected_bytes[..]).unwrap();
        assert_eq!(streamed, expected);
    }

    #[test]
    fn hash_file_streams_a_large_file_correctly() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let content = vec![0x5Au8; CHUNK_SIZE * 5 + 123];
        fs::write(&path, &content).unwrap();

        let streamed = hash_file(&path).unwrap();
        let expected = hash_reader(&mut &content[..]).unwrap();

        assert_eq!(streamed, expected);
    }
}
