//! Playlist Model
//!
//! Represents a user-created or smart playlist with ordered tracks.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// User playlist with ordered tracks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    /// Unique identifier
    pub id: Uuid,

    /// Playlist name
    pub name: String,

    /// Optional description
    pub description: Option<String>,

    /// When the playlist was created
    pub created_at: DateTime<Utc>,

    /// When the playlist was last modified
    pub updated_at: DateTime<Utc>,

    /// Ordered list of track IDs
    pub track_ids: Vec<Uuid>,

    /// Whether this is a smart playlist
    pub is_smart: bool,

    /// Smart playlist criteria (if applicable)
    pub smart_criteria: Option<SmartPlaylistCriteria>,
}

impl Playlist {
    /// Create a new empty playlist
    pub fn new(name: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: None,
            created_at: now,
            updated_at: now,
            track_ids: Vec::new(),
            is_smart: false,
            smart_criteria: None,
        }
    }

    /// Add a track to the end of the playlist
    pub fn add_track(&mut self, track_id: Uuid) {
        self.track_ids.push(track_id);
        self.updated_at = Utc::now();
    }

    /// Add multiple tracks to the end of the playlist
    pub fn add_tracks(&mut self, track_ids: Vec<Uuid>) {
        self.track_ids.extend(track_ids);
        self.updated_at = Utc::now();
    }

    /// Remove a track from the playlist
    pub fn remove_track(&mut self, track_id: Uuid) -> bool {
        if let Some(pos) = self.track_ids.iter().position(|&id| id == track_id) {
            self.track_ids.remove(pos);
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Reorder tracks in the playlist
    pub fn reorder(&mut self, track_ids: Vec<Uuid>) {
        self.track_ids = track_ids;
        self.updated_at = Utc::now();
    }

    /// Get the number of tracks in the playlist
    pub fn track_count(&self) -> usize {
        self.track_ids.len()
    }

    /// Clear all tracks from the playlist
    pub fn clear(&mut self) {
        self.track_ids.clear();
        self.updated_at = Utc::now();
    }
}

/// Criteria for smart playlists
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartPlaylistCriteria {
    /// Match artist (partial match)
    pub artist: Option<String>,

    /// Match album (partial match)
    pub album: Option<String>,

    /// Match genre (exact match)
    pub genre: Option<String>,

    /// Match year range
    pub year_from: Option<i32>,
    pub year_to: Option<i32>,

    /// Minimum play count
    pub min_play_count: Option<u32>,

    /// Maximum play count
    pub max_play_count: Option<u32>,

    /// Include only downloaded tracks
    pub downloaded_only: bool,

    /// Sort by field
    pub sort_by: SmartSortField,

    /// Maximum number of tracks
    pub limit: u32,

    /// Sort descending
    pub sort_desc: bool,
}

impl Default for SmartPlaylistCriteria {
    fn default() -> Self {
        Self {
            artist: None,
            album: None,
            genre: None,
            year_from: None,
            year_to: None,
            min_play_count: None,
            max_play_count: None,
            downloaded_only: false,
            sort_by: SmartSortField::DateAdded,
            limit: 100,
            sort_desc: true,
        }
    }
}

/// Fields for smart playlist sorting
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartSortField {
    Title,
    Artist,
    Album,
    DateAdded,
    LastPlayed,
    PlayCount,
    Year,
    Random,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playlist_creation() {
        let playlist = Playlist::new("My Playlist".to_string());
        assert_eq!(playlist.name, "My Playlist");
        assert_eq!(playlist.track_count(), 0);
        assert!(!playlist.is_smart);
    }

    #[test]
    fn test_add_remove_tracks() {
        let mut playlist = Playlist::new("Test".to_string());
        let track1 = Uuid::new_v4();
        let track2 = Uuid::new_v4();

        playlist.add_track(track1);
        playlist.add_tracks(vec![track2, Uuid::new_v4()]);
        assert_eq!(playlist.track_count(), 3);

        assert!(playlist.remove_track(track1));
        assert_eq!(playlist.track_count(), 2);
        assert!(!playlist.remove_track(Uuid::new_v4())); // Not in playlist
    }

    #[test]
    fn test_reorder() {
        let mut playlist = Playlist::new("Test".to_string());
        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        let t3 = Uuid::new_v4();

        playlist.add_tracks(vec![t1, t2, t3]);
        playlist.reorder(vec![t3, t1, t2]);

        assert_eq!(playlist.track_ids, vec![t3, t1, t2]);
    }
}
