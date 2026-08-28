//! The manifest schema: `{ number, timestamp, message, parent, entries: [ { path,
//! hash, mode } ] }`, plus the `.historia/format` version marker. Kept boring and
//! documented on purpose - see CLAUDE.md §9, the on-disk format contract.

/// Contents `init` writes to a fresh `.historia/format`: the on-disk format version
/// marker, in plain text so the store's format is identifiable without parsing JSON
/// or running this binary (CLAUDE.md §8, §9).
pub const FORMAT_MARKER: &str = "historia format v1\n";

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One tracked file within a snapshot: its path relative to the tracked folder
/// (forward-slash normalized, so manifests are portable across OSes), the SHA-256
/// hash of its content (the blob's name under `objects/`), and its best-effort
/// executable mode (Rule 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub hash: String,
    pub mode: u32,
}

/// A snapshot manifest, `.historia/snapshots/<number>.json` (CLAUDE.md §9). `parent`
/// is the previous snapshot's number, or `0` - the same "no snapshots yet" sentinel
/// `HEAD` uses (CP1, `core::snapshot::INITIAL_HEAD`) - for the very first snapshot.
///
/// `parent_hash` (CP13, hash-chain / tamper-evident history) is the SHA-256 hash
/// of the parent snapshot's manifest file, exactly as those bytes exist on disk
/// (the serialized JSON `write_manifest` wrote, byte-for-byte - not a
/// re-serialization of any in-memory value). It is OPTIONAL, on purpose, and the
/// format stays `v1`: pre-CP13 manifests were written before this field existed,
/// so it is simply absent from their JSON, and `#[serde(default)]` deserializes
/// that absence as `None` - "pre-chain", not an error. `None` also covers a
/// fresh store's very first snapshot, which has no parent manifest to hash at
/// all. Both cases behave identically for `verify`'s chain check (CP13): there is
/// nothing to validate either way. `skip_serializing_if` keeps a `None` out of
/// newly written JSON too, so "first snapshot ever" and "predates the chain"
/// manifests are byte-for-byte indistinguishable in their absence of the field -
/// exactly the shape a real pre-CP13 manifest already has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub number: u64,
    pub timestamp: String,
    pub message: String,
    pub parent: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_hash: Option<String>,
    pub entries: Vec<Entry>,
}

// ---- ISO 8601 UTC timestamp ----
//
// No chrono/time dependency (Rule 9): UTC-only formatting doesn't need a full
// timezone-aware date library, so this is a small hand-rolled civil-calendar
// conversion built on std::time alone. `civil_from_days`/`days_from_civil` are
// Howard Hinnant's public-domain days<->date algorithm
// (http://howardhinnant.github.io/date_algorithms.html), used by many date
// libraries (including `chrono` internally) - not novel math, just not worth a
// dependency for two directions of one calendar conversion.

/// Format `time` as `YYYY-MM-DDTHH:MM:SSZ` (ISO 8601, UTC, second precision).
pub fn iso8601_utc(time: SystemTime) -> String {
    let total_secs = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // Before the epoch: still representable, just negative.
        Err(e) => -(e.duration().as_secs() as i64),
    };
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// The current moment, formatted per [`iso8601_utc`].
pub fn now_iso8601_utc() -> String {
    iso8601_utc(SystemTime::now())
}

/// Days since 1970-01-01 -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// (year, month, day) -> days since 1970-01-01. The inverse of `civil_from_days`,
/// kept alongside it purely so tests can check the two agree across a wide range
/// rather than trusting either direction on faith.
#[cfg(test)]
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400); // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn format_marker_is_plain_text_ending_in_newline() {
        assert!(FORMAT_MARKER.ends_with('\n'));
        assert_eq!(FORMAT_MARKER.trim(), "historia format v1");
    }

    #[test]
    fn manifest_round_trips_through_json_with_exact_field_names() {
        let manifest = Manifest {
            number: 3,
            timestamp: "1970-01-02T01:01:01Z".to_string(),
            message: "fix bug".to_string(),
            parent: 2,
            parent_hash: Some("deadbeef".to_string()),
            entries: vec![Entry {
                path: "src/main.rs".to_string(),
                hash: "abc123".to_string(),
                mode: 0o644,
            }],
        };

        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["number"], 3);
        assert_eq!(json["timestamp"], "1970-01-02T01:01:01Z");
        assert_eq!(json["message"], "fix bug");
        assert_eq!(json["parent"], 2);
        assert_eq!(json["parent_hash"], "deadbeef");
        assert_eq!(json["entries"][0]["path"], "src/main.rs");
        assert_eq!(json["entries"][0]["hash"], "abc123");
        assert_eq!(json["entries"][0]["mode"], 0o644);

        let round_tripped: Manifest = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, manifest);
    }

    // ---- parent_hash (CP13 hash chain) ----

    #[test]
    fn a_none_parent_hash_is_omitted_from_the_json_entirely() {
        let manifest = Manifest {
            number: 1,
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            message: "first".to_string(),
            parent: 0,
            parent_hash: None,
            entries: vec![],
        };

        let json = serde_json::to_value(&manifest).unwrap();

        assert!(
            json.as_object().unwrap().get("parent_hash").is_none(),
            "expected no parent_hash key at all, got {json}"
        );
    }

    #[test]
    fn a_pre_chain_manifest_with_no_parent_hash_key_deserializes_to_none() {
        // Simulates a manifest written before CP13 existed: no such field at
        // all in its JSON, not even a null.
        let pre_chain_json = r#"{
            "number": 1,
            "timestamp": "1970-01-01T00:00:00Z",
            "message": "old snapshot",
            "parent": 0,
            "entries": []
        }"#;

        let manifest: Manifest = serde_json::from_str(pre_chain_json).unwrap();

        assert_eq!(manifest.parent_hash, None);
    }

    // ---- ISO 8601 UTC timestamp ----

    #[test]
    fn epoch_formats_as_1970_01_01_midnight() {
        assert_eq!(iso8601_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn one_day_one_hour_one_minute_one_second_after_epoch() {
        // 1 day (86400s) + 1h (3600s) + 1m (60s) + 1s = 90061s after epoch is,
        // by construction, 1970-01-02T01:01:01Z - hand-verifiable arithmetic, no
        // reliance on any external "well-known timestamp" trivia.
        let t = UNIX_EPOCH + Duration::from_secs(90_061);
        assert_eq!(iso8601_utc(t), "1970-01-02T01:01:01Z");
    }

    #[test]
    fn civil_from_days_round_trips_with_days_from_civil_across_a_wide_range() {
        for days in (-200_000i64..200_000).step_by(97) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(
                days_from_civil(y, m, d),
                days,
                "round-trip failed for days={days} -> ({y},{m},{d})"
            );
        }
    }
}
