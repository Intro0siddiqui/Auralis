//! Track Model
//!
//! Represents a single audio track in the Auralis library.
//! Contains all metadata, playback information, and source tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Audio format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Mp3,
    Flac,
    Aac,
    Ogg,
    Wav,
    M4a,
}

impl AudioFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::Aac => "aac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Wav => "wav",
            AudioFormat::M4a => "m4a",
        }
    }

    /// Get MIME type for this format
    pub fn mime_type(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Aac => "audio/aac",
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::Wav => "audio/wav",
            AudioFormat::M4a => "audio/mp4",
        }
    }

    /// Parse format from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "mp3" => Some(AudioFormat::Mp3),
            "flac" => Some(AudioFormat::Flac),
            "aac" => Some(AudioFormat::Aac),
            "ogg" | "oga" => Some(AudioFormat::Ogg),
            "wav" => Some(AudioFormat::Wav),
            "m4a" | "mp4" => Some(AudioFormat::M4a),
            _ => None,
        }
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.extension().to_uppercase())
    }
}

impl PartialEq<&str> for AudioFormat {
    fn eq(&self, other: &&str) -> bool {
        self.extension() == *other
    }
}

/// Repeat mode for playback
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    /// No repetition
    #[default]
    Off,
    /// Repeat the current track
    One,
    /// Repeat the queue
    All,
}

impl RepeatMode {
    /// Move to the next repeat mode in sequence
    pub fn next(&self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

/// A single audio track
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// Unique identifier
    pub id: Uuid,

    /// Track title
    pub title: String,

    /// Primary artist name
    pub artist: Option<String>,

    /// Album name
    pub album: Option<String>,

    /// Album artist (may differ from track artist for compilations)
    pub album_artist: Option<String>,

    /// Genre classification
    pub genre: Option<String>,

    /// Release year
    pub year: Option<i32>,

    /// Track number within the album
    pub track_number: Option<u32>,

    /// Disc number for multi-disc albums
    pub disc_number: Option<u32>,

    /// Duration in seconds
    pub duration_secs: u32,

    /// Absolute file path
    pub file_path: String,

    /// File size in bytes
    pub file_size: u64,

    /// Audio format
    pub format: AudioFormat,

    /// Bitrate in kbps (if available)
    pub bitrate: Option<u32>,

    /// Sample rate in Hz (if available)
    pub sample_rate: Option<u32>,

    /// Path to album art image (if embedded or extracted)
    pub album_art_path: Option<String>,

    /// When the track was added to the library
    pub date_added: DateTime<Utc>,

    /// When the track was last played
    pub last_played: Option<DateTime<Utc>>,

    /// Number of times played
    pub play_count: u32,

    /// Whether this track was downloaded via Auralis
    pub is_downloaded: bool,

    /// Source URL for downloaded tracks (YouTube, Instagram, etc.)
    pub source_url: Option<String>,
}

impl Track {
    /// Create a new track with required fields
    pub fn new(title: String, file_path: String, duration_secs: u32, format: AudioFormat) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            artist: None,
            album: None,
            album_artist: None,
            genre: None,
            year: None,
            track_number: None,
            disc_number: None,
            duration_secs,
            file_path,
            file_size: 0,
            format,
            bitrate: None,
            sample_rate: None,
            album_art_path: None,
            date_added: Utc::now(),
            last_played: None,
            play_count: 0,
            is_downloaded: false,
            source_url: None,
        }
    }

    /// Format duration as MM:SS or HH:MM:SS
    pub fn formatted_duration(&self) -> String {
        let hours = self.duration_secs / 3600;
        let minutes = (self.duration_secs % 3600) / 60;
        let seconds = self.duration_secs % 60;

        if hours > 0 {
            format!("{}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{}:{:02}", minutes, seconds)
        }
    }

    /// Get formatted artist and title
    pub fn display_name(&self) -> String {
        match &self.artist {
            Some(artist) => format!("{} - {}", artist, self.title),
            None => self.title.clone(),
        }
    }
}

/// Filter criteria for querying tracks
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackFilter {
    /// Filter by artist name (partial match)
    pub artist: Option<String>,

    /// Filter by album name (partial match)
    pub album: Option<String>,

    /// Filter by genre
    pub genre: Option<String>,

    /// Filter by year
    pub year: Option<i32>,

    /// Only include downloaded tracks
    pub downloaded_only: bool,

    /// Sort field
    pub sort_by: Option<TrackSortField>,

    /// Sort direction
    pub sort_desc: bool,

    /// Limit results
    pub limit: Option<u32>,

    /// Offset for pagination
    pub offset: Option<u32>,
}

/// Fields available for sorting tracks
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackSortField {
    Title,
    Artist,
    Album,
    DateAdded,
    LastPlayed,
    PlayCount,
    Duration,
    Year,
}

impl Default for TrackSortField {
    fn default() -> Self {
        TrackSortField::DateAdded
    }
}

/// Updates to track metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMetadataUpdate {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_creation() {
        let track = Track::new(
            "Test Song".to_string(),
            "/music/test.mp3".to_string(),
            180,
            AudioFormat::Mp3,
        );

        assert_eq!(track.title, "Test Song");
        assert_eq!(track.duration_secs, 180);
        assert_eq!(track.format, AudioFormat::Mp3);
    }

    #[test]
    fn test_formatted_duration() {
        let mut track = Track::new(
            "Test".to_string(),
            "/test.mp3".to_string(),
            65,
            AudioFormat::Mp3,
        );

        assert_eq!(track.formatted_duration(), "1:05");

        track.duration_secs = 3661;
        assert_eq!(track.formatted_duration(), "1:01:01");
    }

    #[test]
    fn test_repeat_mode_next() {
        assert_eq!(RepeatMode::Off.next(), RepeatMode::All);
        assert_eq!(RepeatMode::All.next(), RepeatMode::One);
        assert_eq!(RepeatMode::One.next(), RepeatMode::Off);
    }
}
