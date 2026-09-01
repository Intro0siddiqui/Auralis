//! Track Repository Interface
//!
//! Defines the data access contract for tracks.

use crate::domain::models::{AlbumSummary, ArtistSummary, Track, TrackFilter};
use async_trait::async_trait;
use uuid::Uuid;

/// Repository interface for track data access
#[async_trait]
pub trait TrackRepository: Send + Sync {
    /// Find all tracks with optional filtering
    async fn find_all(
        &self,
        filter: TrackFilter,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find a track by ID
    async fn find_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find multiple tracks by ID list, preserving input order
    async fn find_by_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find tracks by file path
    async fn find_by_path(
        &self,
        path: &str,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find multiple tracks by file path list
    async fn find_by_paths(
        &self,
        paths: &[&str],
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find tracks by artist
    async fn find_by_artist(
        &self,
        artist: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find tracks by album
    async fn find_by_album(
        &self,
        album: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Search tracks by query
    async fn search(
        &self,
        query: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Insert a new track
    async fn insert(&self, track: &Track) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Update an existing track
    async fn update(&self, track: &Track) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Delete a track
    async fn delete(&self, id: Uuid) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Delete multiple tracks
    async fn delete_many(
        &self,
        ids: Vec<Uuid>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Get total track count
    async fn count(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;

    /// Get track count matching the same filters as `find_all`
    async fn count_filtered(
        &self,
        filter: TrackFilter,
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;

    /// Get total duration of all tracks
    async fn total_duration(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;

    /// Get recently added tracks
    async fn recent(
        &self,
        limit: u32,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get most played tracks
    async fn most_played(
        &self,
        limit: u32,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>>;

    /// Set favorite status for a track
    async fn set_favorite(
        &self,
        id: &str,
        is_favorite: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Get summary of all albums with track counts and artwork
    async fn get_albums_summary(
        &self,
    ) -> Result<Vec<AlbumSummary>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get summary of all artists with track counts
    async fn get_artists_summary(
        &self,
    ) -> Result<Vec<ArtistSummary>, Box<dyn std::error::Error + Send + Sync>>;
}
