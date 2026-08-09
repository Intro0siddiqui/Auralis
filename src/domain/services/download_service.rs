//! Download Service
//!
//! Handles audio downloading from YouTube, Instagram, and other platforms.

use crate::domain::models::{AudioFormat, DownloadProgress, DownloadStatus};
use crate::domain::repositories::{SettingsRepository, TrackRepository};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Download service for managing media downloads
pub struct DownloadService {
    settings_repository: Arc<dyn SettingsRepository>,
    track_repository: Arc<dyn TrackRepository>,
    active_downloads: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    download_queue: Arc<RwLock<Vec<Uuid>>>,
}

impl DownloadService {
    /// Create a new download service
    pub fn new(
        settings_repository: Arc<dyn SettingsRepository>,
        track_repository: Arc<dyn TrackRepository>,
    ) -> Self {
        Self {
            settings_repository,
            track_repository,
            active_downloads: Arc::new(RwLock::new(HashMap::new())),
            download_queue: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Start a new download
    pub async fn download_audio(
        &self,
        url: String,
        format: AudioFormat,
    ) -> Result<Uuid, DownloadError> {
        info!(url = %url, format = ?format, "Starting download");

        // Validate URL
        if !self.is_supported_url(&url) {
            return Err(DownloadError::UnsupportedUrl(url));
        }

        // Get download settings
        let _settings = self
            .settings_repository
            .get_settings()
            .await
            .map_err(|e| DownloadError::SettingsError(e.to_string()))?;

        // Create download progress entry
        let title = self
            .extract_title(&url)
            .unwrap_or_else(|| "Unknown".to_string());
        let progress = DownloadProgress::new(url.clone(), title, format);

        let download_id = progress.id;

        // Store download progress
        {
            let mut downloads = self.active_downloads.write().await;
            downloads.insert(download_id, progress);
        }

        // Add to queue
        {
            let mut queue = self.download_queue.write().await;
            queue.push(download_id);
        }

        // Start download in background
        let settings_repo = self.settings_repository.clone();
        let track_repo = self.track_repository.clone();
        let downloads = self.active_downloads.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::process_download(
                download_id,
                url,
                format,
                settings_repo,
                track_repo,
                downloads,
            )
            .await
            {
                error!(download_id = %download_id, error = %e, "Download failed");
            }
        });

        Ok(download_id)
    }

    /// Download a playlist
    pub async fn download_playlist(
        &self,
        url: String,
        format: AudioFormat,
    ) -> Result<Vec<Uuid>, DownloadError> {
        info!(url = %url, "Starting playlist download");

        // Get video URLs from playlist
        let urls = self.extract_playlist_urls(&url).await?;

        if urls.is_empty() {
            return Err(DownloadError::EmptyPlaylist);
        }

        info!(
            count = urls.len(),
            "Playlist contains {} videos",
            urls.len()
        );

        let mut download_ids = Vec::new();

        for video_url in urls {
            match self.download_audio(video_url, format).await {
                Ok(id) => download_ids.push(id),
                Err(e) => {
                    warn!(error = %e, "Failed to queue playlist video");
                }
            }
        }

        Ok(download_ids)
    }

    /// Pause a download
    pub async fn pause_download(&self, id: Uuid) -> Result<(), DownloadError> {
        info!(download_id = %id, "Pausing download");

        let mut downloads = self.active_downloads.write().await;

        if let Some(download) = downloads.get_mut(&id) {
            download.pause();
            Ok(())
        } else {
            Err(DownloadError::DownloadNotFound(id))
        }
    }

    /// Resume a download
    pub async fn resume_download(&self, id: Uuid) -> Result<(), DownloadError> {
        info!(download_id = %id, "Resuming download");

        let downloads = self.active_downloads.read().await;

        if let Some(download) = downloads.get(&id) {
            if download.status == DownloadStatus::Paused {
                // TODO: Implement actual resume logic
                info!(download_id = %id, "Download resumed");
                Ok(())
            } else {
                Err(DownloadError::InvalidState(
                    "Download is not paused".to_string(),
                ))
            }
        } else {
            Err(DownloadError::DownloadNotFound(id))
        }
    }

    /// Cancel a download
    pub async fn cancel_download(&self, id: Uuid) -> Result<(), DownloadError> {
        info!(download_id = %id, "Cancelling download");

        let mut downloads = self.active_downloads.write().await;

        if let Some(download) = downloads.get_mut(&id) {
            download.cancel();
            Ok(())
        } else {
            Err(DownloadError::DownloadNotFound(id))
        }
    }

    /// Get download progress for all downloads
    pub async fn get_download_progress(&self) -> Vec<DownloadProgress> {
        let downloads = self.active_downloads.read().await;
        downloads.values().cloned().collect()
    }

    /// Get progress for a specific download
    pub async fn get_download(&self, id: Uuid) -> Option<DownloadProgress> {
        let downloads = self.active_downloads.read().await;
        downloads.get(&id).cloned()
    }

    /// Check if URL is supported
    fn is_supported_url(&self, url: &str) -> bool {
        url.contains("youtube.com") || url.contains("youtu.be") || url.contains("instagram.com")
    }

    /// Extract title from URL (placeholder)
    fn extract_title(&self, url: &str) -> Option<String> {
        // TODO: Implement actual title extraction via yt-dlp
        Some(format!("Track from {}", url))
    }

    /// Extract playlist URLs (placeholder)
    async fn extract_playlist_urls(&self, _url: &str) -> Result<Vec<String>, DownloadError> {
        // TODO: Implement actual playlist URL extraction via yt-dlp
        // For now, return empty
        Ok(Vec::new())
    }

    /// Process a download (background task)
    async fn process_download(
        download_id: Uuid,
        url: String,
        format: AudioFormat,
        settings_repo: Arc<dyn SettingsRepository>,
        _track_repo: Arc<dyn TrackRepository>,
        downloads: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    ) -> Result<(), DownloadError> {
        info!(download_id = %download_id, "Processing download");

        let settings = settings_repo
            .get_settings()
            .await
            .map_err(|e| DownloadError::SettingsError(e.to_string()))?;

        let output_dir = settings.downloads.download_path;
        std::fs::create_dir_all(&output_dir).map_err(|e| DownloadError::IoError(e))?;

        let output_file =
            output_dir.join(format!("download_{}.{}", download_id, format.extension()));

        // Update status to downloading
        {
            let mut downloads_guard = downloads.write().await;
            if let Some(download) = downloads_guard.get_mut(&download_id) {
                download.status = DownloadStatus::Downloading;
            }
        }

        // Build yt-dlp command
        let mut cmd = tokio::process::Command::new("yt-dlp");
        cmd.args([
            "-x",
            "--audio-format",
            format.extension(),
            "-o",
            output_file.to_str().unwrap(),
            &url,
        ]);

        // Run download
        let output = cmd.output().await.map_err(|e| {
            error!(error = %e, "Failed to execute yt-dlp");
            DownloadError::YtDlpError(e.to_string())
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(stderr = %stderr, "yt-dlp failed");

            let mut downloads_guard = downloads.write().await;
            if let Some(download) = downloads_guard.get_mut(&download_id) {
                download.fail(stderr.to_string());
            }

            return Err(DownloadError::YtDlpError(stderr.to_string()));
        }

        // Update to completed
        {
            let mut downloads_guard = downloads.write().await;
            if let Some(download) = downloads_guard.get_mut(&download_id) {
                download.complete(output_file.to_string_lossy().to_string());
            }
        }

        // Add to library
        // TODO: Extract metadata and create track entry

        info!(download_id = %download_id, "Download completed successfully");
        Ok(())
    }
}

/// Download-related errors
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("Unsupported URL: {0}")]
    UnsupportedUrl(String),

    #[error("Download not found: {0}")]
    DownloadNotFound(Uuid),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Empty playlist")]
    EmptyPlaylist,

    #[error("Settings error: {0}")]
    SettingsError(String),

    #[error("yt-dlp error: {0}")]
    YtDlpError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Database error: {0}")]
    DatabaseError(String),
}
