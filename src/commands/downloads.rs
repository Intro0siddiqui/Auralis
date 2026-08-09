//! Download Commands
//!
//! Tauri command handlers for media downloads via yt-dlp.

use crate::templates::render;
use crate::domain::models::{AudioFormat, DownloadProgress};
use crate::templates::{DownloadItemPartial, DownloadsTemplate};
use serde::{Deserialize, Serialize};
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
pub async fn download_audio(request: DownloadRequest) -> Result<DownloadProgress, String> {
    // TODO: dispatch to Downloader with the configured format
    Ok(DownloadProgress::new(
        request.url,
        "Pending".to_string(),
        request.format.unwrap_or(AudioFormat::Mp3),
    ))
}

/// Start downloading a playlist (creates one download per item).
#[tauri::command]
pub async fn download_playlist(_request: PlaylistDownloadRequest) -> Result<Vec<DownloadProgress>, String> {
    // TODO: parse playlist URL and enqueue per-item downloads
    Ok(Vec::new())
}

/// Pause an in-progress download.
#[tauri::command]
pub async fn pause_download(_id: Uuid) -> Result<(), String> {
    // TODO: signal Downloader to pause
    Ok(())
}

/// Resume a paused download.
#[tauri::command]
pub async fn resume_download(_id: Uuid) -> Result<(), String> {
    // TODO: signal Downloader to resume
    Ok(())
}

/// Cancel a download (queued or in-progress).
#[tauri::command]
pub async fn cancel_download(_id: Uuid) -> Result<(), String> {
    // TODO: signal Downloader to cancel
    Ok(())
}

/// Get current progress for a specific download.
#[tauri::command]
pub async fn get_download_progress(_id: Uuid) -> Result<Option<DownloadProgress>, String> {
    // TODO: lookup in Downloader state
    Ok(None)
}

/// List all active downloads.
#[tauri::command]
pub async fn list_downloads() -> Result<Vec<DownloadProgress>, String> {
    // TODO: enumerate Downloader state
    Ok(Vec::new())
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
