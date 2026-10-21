//! Database Repository Implementations
//!
//! SQLite-backed repository implementations.

pub mod playlist;
pub mod settings;
pub mod sync;
pub mod track;

#[cfg(test)]
mod tests;

pub use playlist::SqlitePlaylistRepository;
pub use settings::SqliteSettingsRepository;
pub use sync::SqliteSyncRepository;
pub use track::SqliteTrackRepository;

#[cfg(test)]
pub(crate) use track::escape_like_pattern;

use crate::domain::models::AudioFormat;
use chrono::{DateTime, Utc};

// ============================================================================
// Helper functions
// ============================================================================

pub fn parse_format(s: &str) -> AudioFormat {
    match s.to_lowercase().as_str() {
        "mp3" => AudioFormat::Mp3,
        "flac" => AudioFormat::Flac,
        "aac" => AudioFormat::Aac,
        "ogg" | "oga" => AudioFormat::Ogg,
        "wav" => AudioFormat::Wav,
        "m4a" | "mp4" => AudioFormat::M4a,
        "opus" => AudioFormat::Opus,
        "webm" => AudioFormat::Webm,
        _ => AudioFormat::Mp3,
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn test_parse_format_valid() {
        assert_eq!(parse_format("mp3"), AudioFormat::Mp3);
        assert_eq!(parse_format("flac"), AudioFormat::Flac);
        assert_eq!(parse_format("aac"), AudioFormat::Aac);
        assert_eq!(parse_format("ogg"), AudioFormat::Ogg);
        assert_eq!(parse_format("oga"), AudioFormat::Ogg);
        assert_eq!(parse_format("wav"), AudioFormat::Wav);
        assert_eq!(parse_format("m4a"), AudioFormat::M4a);
        assert_eq!(parse_format("mp4"), AudioFormat::M4a);
        assert_eq!(parse_format("opus"), AudioFormat::Opus);
        assert_eq!(parse_format("webm"), AudioFormat::Webm);
    }

    #[test]
    fn test_parse_format_case_insensitive() {
        assert_eq!(parse_format("MP3"), AudioFormat::Mp3);
        assert_eq!(parse_format("FLAC"), AudioFormat::Flac);
        assert_eq!(parse_format("M4A"), AudioFormat::M4a);
        assert_eq!(parse_format("Opus"), AudioFormat::Opus);
        assert_eq!(parse_format("WEBM"), AudioFormat::Webm);
    }

    #[test]
    fn test_parse_format_invalid_defaults_to_mp3() {
        assert_eq!(parse_format("txt"), AudioFormat::Mp3);
        assert_eq!(parse_format(""), AudioFormat::Mp3);
        assert_eq!(parse_format("unknown"), AudioFormat::Mp3);
        assert_eq!(parse_format("jpg"), AudioFormat::Mp3);
    }

    #[test]
    fn test_parse_datetime_valid() {
        let dt = parse_datetime("2026-09-29T12:30:00Z");
        assert_eq!(dt.timestamp(), 1790685000);

        let dt = parse_datetime("2026-01-01T00:00:00+00:00");
        assert_eq!(dt.timestamp(), 1767225600);

        let dt = parse_datetime("2026-06-15T08:00:00+05:30");
        assert_eq!(dt.timestamp(), 1781490600);
    }

    #[test]
    fn test_parse_datetime_invalid_returns_now() {
        // Invalid date strings should return a timestamp close to now
        let before = Utc::now().timestamp();
        let dt = parse_datetime("not-a-date");
        let after = Utc::now().timestamp();

        assert!(dt.timestamp() >= before);
        assert!(dt.timestamp() <= after);

        let dt = parse_datetime("");
        assert!(dt.timestamp() >= before);
        assert!(dt.timestamp() <= after);

        let dt = parse_datetime("2026-13-45T99:99:99Z");
        assert!(dt.timestamp() >= before);
        assert!(dt.timestamp() <= after);
    }
}

pub fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
