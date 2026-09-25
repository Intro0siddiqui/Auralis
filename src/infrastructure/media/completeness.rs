//! Download completeness verification by *decoded* length.
//!
//! Why this exists
//! ---------------
//! A truncated YouTube download looks perfectly healthy to a metadata reader.
//! YouTube's MP4 streams carry the full track length in the front `moov` atom
//! (`mvhd`/`mdhd`), and the resource is a fragmented MP4 whose per-fragment
//! sample tables sit in `moof` boxes *after* the front header. So a file that
//! stops at 1:39 of a 4:46 track still reports 4:46 to `lofty`, and every
//! duration-based check built on container metadata happily passes it. The
//! bytes on disk are short, the header says otherwise, and the download used
//! to be reported as completed.
//!
//! The only trustworthy signal is the decoder's own length: it reflects the
//! samples that are really present (which is exactly the number the player
//! shows, and the same value `player.rs` uses to auto-repair a lying database
//! duration after the fact).
//!
//! Behaviour
//! ---------
//! * `Ok(Some(decoded_secs))` — the file decoded and its length is known.
//! * `Ok(None)` — no decoder available for the container (rodio cannot open
//!   Opus-in-WebM), so completeness cannot be judged here. Not an error: the
//!   byte-accounting gate in `downloader.rs` remains the backstop.
//! * `Err(_)` — the decodable audio is materially shorter than the track is
//!   supposed to be, i.e. the download is truncated. The caller must not save
//!   the file.
//!
//! The threshold is deliberately loose (a file must be >10% short before it is
//! rejected) so that container-level rounding differences between
//! `mdhd`-derived and sample-derived lengths never produce false failures.

use rodio::Decoder;
use std::io::BufReader;
use std::path::Path;

/// A file must be at least this percentage of its expected length to be
/// accepted (90%). Anything shorter is treated as truncated.
const MIN_DECODABLE_PCT: u64 = 90;

/// Absolute slack, in seconds, absorbing decoder rounding so that very short
/// clips (where a single second is a large percentage) are not rejected.
const DURATION_SLACK_SECS: u64 = 2;

/// Tracks shorter than this are only checked with the percentage rule.
const MIN_DURATION_FOR_PCT_CHECK_SECS: u64 = 20;

/// Decode the file and report how much audio is actually present.
///
/// Returns `None` when no decoder can open the container (e.g. Opus-in-WebM,
/// which rodio does not support) — callers must treat that as "cannot verify",
/// not as "invalid".
pub fn decoded_duration_secs(path: &Path, ext: &str) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    if ext.is_empty() {
        return Decoder::new(BufReader::with_capacity(64 * 1024, file))
            .ok()
            .and_then(|d| d.total_duration().map(|dur| dur.as_secs()));
    }
    let hinted = Decoder::builder()
        .with_data(BufReader::with_capacity(64 * 1024, file))
        .with_hint(ext)
        .build();
    let decoder = match hinted {
        Ok(d) => d,
        // The extension hint can be wrong for mislabelled containers; retry
        // with a fresh handle (the first `File` was consumed by the builder).
        Err(_) => {
            let file = std::fs::File::open(path).ok()?;
            Decoder::new(BufReader::with_capacity(64 * 1024, file)).ok()?
        }
    };
    decoder.total_duration().map(|d| d.as_secs())
}

/// Verify a finished download against the track duration reported by the
/// resolver, using the decoded length rather than container metadata.
///
/// * `Ok(Some(decoded_secs))` — verified (or nothing to verify against).
/// * `Ok(None)` — no decoder available; completeness unverified.
/// * `Err(msg)` — truncated; the caller must discard the file.
pub fn verify_decoded_duration(
    path: &Path,
    ext: &str,
    expected_duration_secs: Option<u32>,
) -> Result<Option<u64>, String> {
    let Some(expected) = expected_duration_secs.filter(|e| *e > 0) else {
        return Ok(None); // nothing to compare against
    };
    let Some(decoded) = decoded_duration_secs(path, ext) else {
        return Ok(None); // container not decodable here (Opus/WebM)
    };
    if decoded == 0 {
        return Err(format!(
            "Decoded audio length is 0s for a track expected to last {expected}s \
             (container header is intact but no audio could be decoded)"
        ));
    }

    let expected_u64 = expected as u64;
    let pct_short = decoded
        .saturating_mul(100)
        .saturating_add(DURATION_SLACK_SECS * 100)
        < expected_u64.saturating_mul(MIN_DECODABLE_PCT);
    if pct_short && expected_u64 >= MIN_DURATION_FOR_PCT_CHECK_SECS {
        return Err(format!(
            "Truncated download: only {decoded}s of {expected_u64}s of audio is actually \
             present (>{}% missing). The container header still advertises the full length, \
             so the file was rejected instead of being saved as a {decoded}s track",
            100u64.saturating_sub(decoded.saturating_mul(100) / expected_u64.max(1))
        ));
    }

    Ok(Some(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "auralis_completeness_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("probe.m4a")
    }

    #[test]
    fn missing_file_is_not_verifiable() {
        let missing = PathBuf::from("/nonexistent/auralis/does-not-exist.m4a");
        assert_eq!(decoded_duration_secs(&missing, "m4a"), None);
        assert!(matches!(
            verify_decoded_duration(&missing, "m4a", Some(240)),
            Ok(None)
        ));
    }

    #[test]
    fn unknown_expected_duration_skips_verification() {
        let path = temp_path("no_expected");
        std::fs::write(&path, b"not really audio").unwrap();
        assert!(matches!(
            verify_decoded_duration(&path, "m4a", None),
            Ok(None)
        ));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn undecodable_container_is_not_treated_as_truncated() {
        // Garbage bytes that no decoder can open must not be reported as a
        // truncation: `downloader.rs` byte accounting owns that verdict.
        let path = temp_path("garbage");
        std::fs::write(&path, b"\x00\x01\x02\x03not audio at all").unwrap();
        assert!(matches!(
            verify_decoded_duration(&path, "m4a", Some(240)),
            Ok(None)
        ));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
