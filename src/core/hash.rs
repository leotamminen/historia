//! Streaming content hashing (SHA-256). Hashes files in fixed-size chunks rather
//! than reading them whole into memory, so a large tracked file costs O(chunk size)
//! RAM, not O(file size) (Rule 11).
