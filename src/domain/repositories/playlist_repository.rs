//! Playlist Repository Interface
//!
//! Defines the data access contract for playlists.

use crate::domain::models::Playlist;
use async_trait::async_trait;
use uuid::Uuid;

/// Repository interface for playlist data access
#[async_trait]
pub trait PlaylistRepository: Send + Sync {
    /// Find all playlists
    async fn find_all(&self) -> Result<Vec<Playlist>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find a playlist by ID
    async fn find_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<Playlist>, Box<dyn std::error::Error + Send + Sync>>;

    /// Find a playlist by name
    async fn find_by_name(
        &self,
        name: &str,
    ) -> Result<Option<Playlist>, Box<dyn std::error::Error + Send + Sync>>;

    /// Insert a new playlist
    async fn insert(
        &self,
        playlist: &Playlist,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Update an existing playlist
    async fn update(
        &self,
        playlist: &Playlist,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Delete a playlist
    async fn delete(&self, id: Uuid) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Get playlist count
    async fn count(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
}
