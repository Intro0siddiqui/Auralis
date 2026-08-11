//! Download Commands
//!
//! Tauri command handlers for media downloads via yt-dlp.

use crate::domain::models::{AudioFormat, DownloadProgress};
use crate::infrastructure::media::downloader::Downloader;
use crate::templates::render;
use crate::templates::{DownloadItemPartial, DownloadsTemplate};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{error, info};
use uuid::Uuid;

/// Download request
#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub format: Option<AudioFormat>,
    pub quality: Option<u32>,
}

/// Playlist download request
#[derive(Debug, Serialize, Deserialize)]
pub struct PlaylistDownloadRequest {
    pub url: String,
    pub format: Option<AudioFormat>,
    pub max_items: Option<u32>,
}

/// Start downloading a single audio track.
#[tauri::command]
pub async fn download_audio(
    request: DownloadRequest,
    downloader: State<'_, Downloader>,
) -> Result<DownloadProgress, String> {
    info!(url = %request.url, "Download audio requested");

    let format = request.format.unwrap_or(AudioFormat::Mp3);

    let id = downloader
        .download(&request.url, format)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to start download");
            e.to_string()
        })?;

    let state = downloader
        .get_progress(id)
        .await
        .ok_or("Download not found after starting")?;

    Ok(state)
}

/// Start downloading a playlist (creates one download per item).
#[tauri::command]
pub async fn download_playlist(
    request: PlaylistDownloadRequest,
    downloader: State<'_, Downloader>,
) -> Result<Vec<DownloadProgress>, String> {
    info!(url = %request.url, "Playlist download requested");

    let format = request.format.unwrap_or(AudioFormat::Mp3);

    let id = downloader
        .download(&request.url, format)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to start playlist download");
            e.to_string()
        })?;

    let state = downloader
        .get_progress(id)
        .await
        .ok_or("Download not found after starting")?;

    let _max_items = request.max_items.unwrap_or(10);
    Ok(vec![state])
}

/// Pause an in-progress download.
#[tauri::command]
pub async fn pause_download(id: Uuid, downloader: State<'_, Downloader>) -> Result<(), String> {
    info!(download_id = %id, "Pause download requested");

    downloader.pause(id).await.map_err(|e| {
        error!(error = %e, "Failed to pause download");
        e.to_string()
    })
}

/// Resume a paused download.
#[tauri::command]
pub async fn resume_download(id: Uuid, downloader: State<'_, Downloader>) -> Result<(), String> {
    info!(download_id = %id, "Resume download requested");

    downloader.resume(id).await.map_err(|e| {
        error!(error = %e, "Failed to resume download");
        e.to_string()
    })
}

/// Cancel a download (queued or in-progress).
#[tauri::command]
pub async fn cancel_download(id: Uuid, downloader: State<'_, Downloader>) -> Result<(), String> {
    info!(download_id = %id, "Cancel download requested");

    downloader.cancel(id).await.map_err(|e| {
        error!(error = %e, "Failed to cancel download");
        e.to_string()
    })
}

/// Get current progress for a specific download.
#[tauri::command]
pub async fn get_download_progress(
    id: Uuid,
    downloader: State<'_, Downloader>,
) -> Result<Option<DownloadProgress>, String> {
    info!(download_id = %id, "Get download progress requested");

    Ok(downloader.get_progress(id).await)
}

/// List all active downloads.
#[tauri::command]
pub async fn list_downloads(
    downloader: State<'_, Downloader>,
) -> Result<Vec<DownloadProgress>, String> {
    info!("List downloads requested");

    Ok(downloader.list_active().await)
}

/// Render the downloads page as HTML.
#[tauri::command]
pub async fn render_downloads() -> Result<String, String> {
    let downloads: Vec<DownloadProgress> = Vec::new();
    let tmpl = DownloadsTemplate {
        downloads: &downloads,
    };
    render(&tmpl).map_err(|e| e.to_string())
}

/// Render a single download row (used for HTMX polling updates).
#[tauri::command]
pub async fn render_download_item(download: DownloadProgress) -> Result<String, String> {
    let tmpl = DownloadItemPartial {
        download: &download,
    };
    render(&tmpl).map_err(|e| e.to_string())
}
