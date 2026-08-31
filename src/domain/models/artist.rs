//! Artist Model
//!
//! Represents a music artist with aggregated track statistics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a music artist in the library
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    /// Unique identifier
    pub id: Uuid,

    /// Artist name
    pub name: String,

    /// Biography or description
    pub bio: Option<String>,

    /// Path to artist image
    pub image_path: Option<String>,

    /// When the artist was first added to the library
    pub date_added: DateTime<Utc>,

    /// Number of tracks by this artist
    pub track_count: u32,

    /// Number of albums by this artist
    pub album_count: u32,

    /// Total duration of all tracks in seconds
    pub total_duration_secs: u32,
}

impl Artist {
    /// Create a new artist
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            bio: None,
            image_path: None,
            date_added: Utc::now(),
            track_count: 0,
            album_count: 0,
            total_duration_secs: 0,
        }
    }

    /// Build artist from track collection
    pub fn from_tracks(name: &str, tracks: &[crate::Track]) -> Self {
        let albums: Vec<_> = tracks.iter().filter_map(|t| t.album.clone()).collect();
        let unique_albums: std::collections::HashSet<_> = albums.iter().collect();

        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            bio: None,
            image_path: tracks.first().and_then(|t| t.album_art_path.clone()),
            date_added: tracks
                .iter()
                .map(|t| t.date_added)
                .min()
                .unwrap_or_else(Utc::now),
            track_count: tracks.len() as u32,
            album_count: unique_albums.len() as u32,
            total_duration_secs: tracks.iter().map(|t| t.duration_secs).sum(),
        }
    }

    /// Get formatted total duration
    pub fn formatted_duration(&self) -> String {
        let hours = self.total_duration_secs / 3600;
        let minutes = (self.total_duration_secs % 3600) / 60;

        if hours > 0 {
            format!("{} hr {} min", hours, minutes)
        } else {
            format!("{} min", minutes)
        }
    }
}

/// Summary representation of an artist aggregated from tracks for catalog and grid views
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtistSummary {
    /// Artist name
    pub artist: String,

    /// Number of tracks by this artist
    pub track_count: u32,

    /// ID of the first track by this artist (for quick one-click playback)
    pub first_track_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioFormat;

    #[test]
    fn test_artist_creation() {
        let artist = Artist::new("Test Artist".to_string());
        assert_eq!(artist.name, "Test Artist");
        assert_eq!(artist.track_count, 0);
    }

    #[test]
    fn test_artist_from_tracks() {
        let tracks = vec![
            crate::Track::new(
                "Song 1".to_string(),
                "/song1.mp3".to_string(),
                180,
                AudioFormat::Mp3,
            ),
            crate::Track::new(
                "Song 2".to_string(),
                "/song2.mp3".to_string(),
                240,
                AudioFormat::Mp3,
            ),
        ];

        let artist = Artist::from_tracks("Test Artist", &tracks);
        assert_eq!(artist.name, "Test Artist");
        assert_eq!(artist.track_count, 2);
        assert_eq!(artist.total_duration_secs, 420);
    }

    #[test]
    fn test_artist_summary() {
        let summary = ArtistSummary {
            artist: "Pink Floyd".to_string(),
            track_count: 24,
            first_track_id: Some("uuid-5678".to_string()),
        };
        assert_eq!(summary.artist, "Pink Floyd");
        assert_eq!(summary.track_count, 24);
    }
}
