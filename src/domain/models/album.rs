//! Album Model
//!
//! Represents an album, which is a collection of tracks.
//! Provides aggregation of album-level metadata and statistics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents an album in the music library
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    /// Unique identifier
    pub id: Uuid,

    /// Album title
    pub title: String,

    /// Primary artist (may differ from track artists for compilations)
    pub artist: Option<String>,

    /// Release year
    pub year: Option<i32>,

    /// Primary genre
    pub genre: Option<String>,

    /// Path to album art
    pub art_path: Option<String>,

    /// When the album was added to the library
    pub date_added: DateTime<Utc>,

    /// Number of tracks
    pub track_count: u32,

    /// Total duration in seconds
    pub total_duration_secs: u32,

    /// Total file size in bytes
    pub total_size: u64,
}

impl Album {
    /// Create a new album
    pub fn new(title: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            artist: None,
            year: None,
            genre: None,
            art_path: None,
            date_added: Utc::now(),
            track_count: 0,
            total_duration_secs: 0,
            total_size: 0,
        }
    }

    /// Calculate total duration formatted as string
    pub fn formatted_duration(&self) -> String {
        let hours = self.total_duration_secs / 3600;
        let minutes = (self.total_duration_secs % 3600) / 60;

        if hours > 0 {
            format!("{} hr {} min", hours, minutes)
        } else {
            format!("{} min", minutes)
        }
    }

    /// Build album from a collection of tracks
    pub fn from_tracks(tracks: &[crate::Track]) -> Option<Self> {
        if tracks.is_empty() {
            return None;
        }

        let first = &tracks[0];
        let mut album = Album::new(first.album.clone().unwrap_or_else(|| "Unknown Album".to_string()));
        album.artist = first.album_artist.clone().or_else(|| first.artist.clone());
        album.year = first.year;
        album.genre = first.genre.clone();
        album.art_path = first.album_art_path.clone();
        album.date_added = tracks.iter().map(|t| t.date_added).min().unwrap_or_else(Utc::now);
        album.track_count = tracks.len() as u32;
        album.total_duration_secs = tracks.iter().map(|t| t.duration_secs).sum();
        album.total_size = tracks.iter().map(|t| t.file_size).sum();

        Some(album)
    }

    /// Get display name with artist
    pub fn display_name(&self) -> String {
        match &self.artist {
            Some(artist) => format!("{} - {}", artist, self.title),
            None => self.title.clone(),
        }
    }
}

/// Album artwork management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumArt {
    /// Unique identifier
    pub id: Uuid,

    /// Album this art belongs to
    pub album_id: Uuid,

    /// File path to the image
    pub file_path: String,

    /// Image dimensions
    pub width: u32,
    pub height: u32,

    /// Image format (jpeg, png, etc.)
    pub format: String,

    /// File size in bytes
    pub file_size: u64,
}

impl AlbumArt {
    /// Check if this is a square image (standard album art)
    pub fn is_square(&self) -> bool {
        self.width == self.height
    }

    /// Get aspect ratio as a float
    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_album_creation() {
        let album = Album::new("Test Album".to_string());
        assert_eq!(album.title, "Test Album");
        assert_eq!(album.track_count, 0);
    }

    #[test]
    fn test_formatted_duration() {
        let mut album = Album::new("Test".to_string());
        album.total_duration_secs = 3661;
        assert_eq!(album.formatted_duration(), "1 hr 1 min");

        album.total_duration_secs = 180;
        assert_eq!(album.formatted_duration(), "3 min");
    }
}
