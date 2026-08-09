//! Library Service
//!
//! Handles library management operations including scanning, searching,
//! and metadata management.

use crate::domain::models::{ScanSummary, Track, TrackFilter, TrackMetadataUpdate};
use crate::domain::repositories::{TrackRepository, SettingsRepository};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Library service for managing the music library
pub struct LibraryService {
    track_repository: Arc<dyn TrackRepository>,
    settings_repository: Arc<dyn SettingsRepository>,
    /// Background library scanner; reserved for future use in `scan_library_paths`.
    #[allow(dead_code)]
    scanner: Arc<RwLock<Option<LibraryScanner>>>,
}

impl LibraryService {
    /// Create a new library service
    pub fn new(
        track_repository: Arc<dyn TrackRepository>,
        settings_repository: Arc<dyn SettingsRepository>,
    ) -> Self {
        Self {
            track_repository,
            settings_repository,
            scanner: Arc::new(RwLock::new(None)),
        }
    }

    /// Get all tracks with optional filtering
    pub async fn get_tracks(&self, filter: TrackFilter) -> Result<Vec<Track>, LibraryError> {
        debug!(?filter, "Fetching tracks with filter");
        self.track_repository.find_all(filter).await.map_err(|e| {
            error!(error = %e, "Failed to fetch tracks");
            LibraryError::DatabaseError(e.to_string())
        })
    }

    /// Get a single track by ID
    pub async fn get_track(&self, id: uuid::Uuid) -> Result<Option<Track>, LibraryError> {
        debug!(id = %id, "Fetching track by ID");
        self.track_repository.find_by_id(id).await.map_err(|e| {
            error!(error = %e, "Failed to fetch track");
            LibraryError::DatabaseError(e.to_string())
        })
    }

    /// Update track metadata
    pub async fn update_track_metadata(
        &self,
        id: uuid::Uuid,
        update: TrackMetadataUpdate,
    ) -> Result<Track, LibraryError> {
        info!(id = %id, "Updating track metadata");

        let mut track = self
            .track_repository
            .find_by_id(id)
            .await
            .map_err(|e| LibraryError::DatabaseError(e.to_string()))?
            .ok_or_else(|| LibraryError::TrackNotFound(id))?;

        // Apply updates
        if let Some(title) = update.title {
            track.title = title;
        }
        if let Some(artist) = update.artist {
            track.artist = Some(artist);
        }
        if let Some(album) = update.album {
            track.album = Some(album);
        }
        if let Some(album_artist) = update.album_artist {
            track.album_artist = Some(album_artist);
        }
        if let Some(genre) = update.genre {
            track.genre = Some(genre);
        }
        if let Some(year) = update.year {
            track.year = Some(year);
        }
        if let Some(track_number) = update.track_number {
            track.track_number = Some(track_number);
        }
        if let Some(disc_number) = update.disc_number {
            track.disc_number = Some(disc_number);
        }

        self.track_repository.update(&track).await.map_err(|e| {
            error!(error = %e, "Failed to update track");
            LibraryError::DatabaseError(e.to_string())
        })?;

        info!(id = %id, "Track metadata updated successfully");
        Ok(track)
    }

    /// Delete tracks by IDs
    pub async fn delete_tracks(&self, ids: Vec<uuid::Uuid>) -> Result<(), LibraryError> {
        info!(count = ids.len(), "Deleting tracks");

        for id in &ids {
            self.track_repository.delete(*id).await.map_err(|e| {
                error!(error = %e, id = %id, "Failed to delete track");
                LibraryError::DatabaseError(e.to_string())
            })?;
        }

        info!(count = ids.len(), "Tracks deleted successfully");
        Ok(())
    }

    /// Scan library paths for new music
    pub async fn scan_library_paths(&self) -> Result<ScanSummary, LibraryError> {
        info!("Starting library scan");

        let settings = self
            .settings_repository
            .get_settings()
            .await
            .map_err(|e| LibraryError::DatabaseError(e.to_string()))?;

        let scan_paths = settings.library.scan_paths;

        if scan_paths.is_empty() {
            warn!("No scan paths configured");
            return Ok(ScanSummary {
                tracks_added: 0,
                tracks_updated: 0,
                tracks_removed: 0,
                errors: vec!["No scan paths configured".to_string()],
            });
        }

        let mut summary = ScanSummary {
            tracks_added: 0,
            tracks_updated: 0,
            tracks_removed: 0,
            errors: Vec::new(),
        };

        for path in scan_paths {
            match self.scan_path(&path).await {
                Ok(path_summary) => {
                    summary.tracks_added += path_summary.tracks_added;
                    summary.tracks_updated += path_summary.tracks_updated;
                    summary.errors.extend(path_summary.errors);
                }
                Err(e) => {
                    error!(path = %path.display(), error = %e, "Failed to scan path");
                    summary.errors.push(format!("Failed to scan {}: {}", path.display(), e));
                }
            }
        }

        info!(
            added = summary.tracks_added,
            updated = summary.tracks_updated,
            errors = summary.errors.len(),
            "Library scan completed"
        );

        Ok(summary)
    }

    /// Scan a single path for music files
    async fn scan_path(&self, path: &Path) -> Result<ScanSummary, LibraryError> {
        debug!(path = %path.display(), "Scanning path");

        let summary = ScanSummary {
            tracks_added: 0,
            tracks_updated: 0,
            tracks_removed: 0,
            errors: Vec::new(),
        };

        // TODO: Implement actual file scanning and metadata extraction
        // This would use the infrastructure layer's filesystem scanner

        Ok(summary)
    }

    /// Search tracks by query string
    pub async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, LibraryError> {
        debug!(query = %query, "Searching tracks");

        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let filter = TrackFilter {
            artist: Some(query.to_string()),
            ..Default::default()
        };

        let mut results = self.get_tracks(filter).await?;

        // Also search by title and album
        let title_filter = TrackFilter {
            album: Some(query.to_string()),
            ..Default::default()
        };

        let title_results = self.track_repository.find_all(title_filter).await.map_err(|e| {
            LibraryError::DatabaseError(e.to_string())
        })?;

        // Merge results, avoiding duplicates
        for track in title_results {
            if !results.iter().any(|t| t.id == track.id) {
                results.push(track);
            }
        }

        debug!(query = %query, count = results.len(), "Search completed");
        Ok(results)
    }

    /// Get track count
    pub async fn get_track_count(&self) -> Result<u64, LibraryError> {
        self.track_repository.count().await.map_err(|e| {
            LibraryError::DatabaseError(e.to_string())
        })
    }

    /// Get total library duration
    pub async fn get_total_duration(&self) -> Result<u64, LibraryError> {
        self.track_repository.total_duration().await.map_err(|e| {
            LibraryError::DatabaseError(e.to_string())
        })
    }
}

/// Temporary scanner struct (will be implemented in infrastructure)
struct LibraryScanner;

/// Library-related errors
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("Track not found: {0}")]
    TrackNotFound(uuid::Uuid),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("File system error: {0}")]
    FileSystemError(String),

    #[error("Metadata extraction error: {0}")]
    MetadataError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
