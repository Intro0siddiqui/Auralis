//! Playlist Service
//!
//! Handles playlist management operations.

use crate::domain::models::{Playlist, SmartPlaylistCriteria};
use crate::domain::repositories::PlaylistRepository;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Playlist service for managing playlists
pub struct PlaylistService {
    repository: Arc<dyn PlaylistRepository>,
    cache: Arc<RwLock<Vec<Playlist>>>,
}

impl PlaylistService {
    /// Create a new playlist service
    pub fn new(repository: Arc<dyn PlaylistRepository>) -> Self {
        Self {
            repository,
            cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Initialize cache from repository
    pub async fn init_cache(&self) -> Result<(), PlaylistError> {
        let playlists = self.repository.find_all().await.map_err(|e| {
            error!(error = %e, "Failed to load playlists");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        let mut cache = self.cache.write().await;
        *cache = playlists;
        debug!(count = cache.len(), "Playlist cache initialized");
        Ok(())
    }

    /// Get all playlists
    pub async fn get_playlists(&self) -> Result<Vec<Playlist>, PlaylistError> {
        let cache = self.cache.read().await;
        Ok(cache.clone())
    }

    /// Get a playlist by ID
    pub async fn get_playlist(&self, id: Uuid) -> Result<Option<Playlist>, PlaylistError> {
        let cache = self.cache.read().await;
        Ok(cache.iter().find(|p| p.id == id).cloned())
    }

    /// Create a new playlist
    pub async fn create_playlist(&self, name: String) -> Result<Playlist, PlaylistError> {
        info!(name = %name, "Creating playlist");

        if name.trim().is_empty() {
            return Err(PlaylistError::InvalidName(
                "Name cannot be empty".to_string(),
            ));
        }

        let playlist = Playlist::new(name);

        self.repository.insert(&playlist).await.map_err(|e| {
            error!(error = %e, "Failed to create playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.push(playlist.clone());
        }

        info!(id = %playlist.id, "Playlist created");
        Ok(playlist)
    }

    /// Update playlist metadata
    pub async fn update_playlist(
        &self,
        id: Uuid,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<Playlist, PlaylistError> {
        info!(id = %id, "Updating playlist");

        let mut playlist = self
            .repository
            .find_by_id(id)
            .await
            .map_err(|e| PlaylistError::DatabaseError(e.to_string()))?
            .ok_or(PlaylistError::PlaylistNotFound(id))?;

        if let Some(name) = name {
            if name.trim().is_empty() {
                return Err(PlaylistError::InvalidName(
                    "Name cannot be empty".to_string(),
                ));
            }
            playlist.name = name;
        }

        playlist.description = description;
        playlist.updated_at = chrono::Utc::now();

        self.repository.update(&playlist).await.map_err(|e| {
            error!(error = %e, "Failed to update playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            if let Some(pos) = cache.iter().position(|p| p.id == id) {
                cache[pos] = playlist.clone();
            }
        }

        info!(id = %id, "Playlist updated");
        Ok(playlist)
    }

    /// Delete a playlist
    pub async fn delete_playlist(&self, id: Uuid) -> Result<(), PlaylistError> {
        info!(id = %id, "Deleting playlist");

        self.repository.delete(id).await.map_err(|e| {
            error!(error = %e, "Failed to delete playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.retain(|p| p.id != id);
        }

        info!(id = %id, "Playlist deleted");
        Ok(())
    }

    /// Add tracks to playlist
    pub async fn add_tracks_to_playlist(
        &self,
        playlist_id: Uuid,
        track_ids: Vec<Uuid>,
    ) -> Result<(), PlaylistError> {
        debug!(playlist_id = %playlist_id, count = track_ids.len(), "Adding tracks to playlist");

        let mut playlist = self
            .repository
            .find_by_id(playlist_id)
            .await
            .map_err(|e| PlaylistError::DatabaseError(e.to_string()))?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))?;

        playlist.add_tracks(track_ids);

        self.repository.update(&playlist).await.map_err(|e| {
            error!(error = %e, "Failed to update playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            if let Some(pos) = cache.iter().position(|p| p.id == playlist_id) {
                cache[pos] = playlist;
            }
        }

        Ok(())
    }

    /// Remove tracks from playlist
    pub async fn remove_tracks_from_playlist(
        &self,
        playlist_id: Uuid,
        track_ids: Vec<Uuid>,
    ) -> Result<(), PlaylistError> {
        debug!(playlist_id = %playlist_id, count = track_ids.len(), "Removing tracks from playlist");

        let mut playlist = self
            .repository
            .find_by_id(playlist_id)
            .await
            .map_err(|e| PlaylistError::DatabaseError(e.to_string()))?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))?;

        for track_id in track_ids {
            playlist.remove_track(track_id);
        }

        self.repository.update(&playlist).await.map_err(|e| {
            error!(error = %e, "Failed to update playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            if let Some(pos) = cache.iter().position(|p| p.id == playlist_id) {
                cache[pos] = playlist;
            }
        }

        Ok(())
    }

    /// Reorder tracks in playlist
    pub async fn reorder_playlist_tracks(
        &self,
        playlist_id: Uuid,
        track_ids: Vec<Uuid>,
    ) -> Result<(), PlaylistError> {
        debug!(playlist_id = %playlist_id, "Reordering playlist tracks");

        let mut playlist = self
            .repository
            .find_by_id(playlist_id)
            .await
            .map_err(|e| PlaylistError::DatabaseError(e.to_string()))?
            .ok_or(PlaylistError::PlaylistNotFound(playlist_id))?;

        playlist.reorder(track_ids);

        self.repository.update(&playlist).await.map_err(|e| {
            error!(error = %e, "Failed to update playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            if let Some(pos) = cache.iter().position(|p| p.id == playlist_id) {
                cache[pos] = playlist;
            }
        }

        Ok(())
    }

    /// Create a smart playlist
    pub async fn create_smart_playlist(
        &self,
        name: String,
        criteria: SmartPlaylistCriteria,
    ) -> Result<Playlist, PlaylistError> {
        info!(name = %name, "Creating smart playlist");

        let mut playlist = Playlist::new(name);
        playlist.is_smart = true;
        playlist.smart_criteria = Some(criteria);

        self.repository.insert(&playlist).await.map_err(|e| {
            error!(error = %e, "Failed to create smart playlist");
            PlaylistError::DatabaseError(e.to_string())
        })?;

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.push(playlist.clone());
        }

        info!(id = %playlist.id, "Smart playlist created");
        Ok(playlist)
    }
}

/// Playlist-related errors
#[derive(Debug, thiserror::Error)]
pub enum PlaylistError {
    #[error("Playlist not found: {0}")]
    PlaylistNotFound(Uuid),

    #[error("Invalid playlist name: {0}")]
    InvalidName(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
