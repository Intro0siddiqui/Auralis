//! Download Commands
//!
//! Tauri command handlers for media downloads. The frontend resolves a
//! user-facing URL (e.g. YouTube) into a direct audio stream URL via
//! `youtube.js`; these commands stream that URL to disk through the
//! [`Downloader`].

use crate::domain::models::{AudioFormat, DownloadProgress, DownloadStatus};
use crate::infrastructure::media::downloader::{Downloader, StreamDownload};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tracing::{error, info};
use uuid::Uuid;

/// A single resolved download request.
///
/// The `url` is a *direct* audio stream URL (already resolved by the frontend
/// `youtube.js` resolver), not a raw user-facing link.
#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// Resolved direct audio stream URL (https).
    pub url: String,
    /// Display title.
    pub title: String,
    /// Source platform label (e.g. `youtube`, `direct`).
    pub platform: Option<String>,
    /// Container/format metadata (display only).
    pub format: Option<AudioFormat>,
    /// File extension of the saved bytes (e.g. `webm`, `m4a`).
    pub ext: Option<String>,
    /// Known total size in bytes, if known.
    pub total_bytes: Option<u64>,
    /// Optional thumbnail URL for display.
    pub thumbnail: Option<String>,
}

/// Playlist download request — a pre-resolved list of items.
#[derive(Debug, Serialize, Deserialize)]
pub struct PlaylistDownloadRequest {
    pub items: Vec<DownloadRequest>,
    pub format: Option<AudioFormat>,
    pub max_items: Option<u32>,
}

/// Start downloading a single resolved audio track.
#[tauri::command]
pub async fn download_audio(
    request: DownloadRequest,
    app: AppHandle,
    downloader: State<'_, Downloader>,
) -> Result<DownloadProgress, String> {
    info!(url = %request.url, "Download audio requested");

    if !request.url.starts_with("https://") {
        return Err("Only secure HTTPS URLs are supported".to_string());
    }

    let format = request.format.unwrap_or(AudioFormat::Mp3);
    let stream = StreamDownload {
        stream_url: request.url.clone(),
        title: request.title.clone(),
        platform: request
            .platform
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        format,
        ext: request
            .ext
            .clone()
            .unwrap_or_else(|| format.extension().to_string()),
        total_bytes: request.total_bytes,
    };

    let id = downloader.download(stream).await.map_err(|e| {
        error!(error = %e, "Failed to start download");
        e.to_string()
    })?;

    let state = downloader
        .get_progress(id)
        .await
        .ok_or("Download not found after starting")?;

    // Stream progress + completion events to the frontend while the
    // download task runs.
    let app_handle = app.clone();
    let dl = (*downloader).clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match dl.get_progress(id).await {
                Some(progress) => {
                    let _ = app_handle.emit("download:progress", &progress);
                    if matches!(
                        progress.status,
                        DownloadStatus::Completed
                            | DownloadStatus::Failed
                            | DownloadStatus::Cancelled
                    ) {
                        let _ = app_handle.emit("download:completed", &progress);
                        break;
                    }
                }
                None => break,
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    Ok(state)
}

/// Start downloading a playlist (one download per pre-resolved item).
#[tauri::command]
pub async fn download_playlist(
    request: PlaylistDownloadRequest,
    downloader: State<'_, Downloader>,
) -> Result<Vec<DownloadProgress>, String> {
    info!(count = request.items.len(), "Playlist download requested");

    let max_items = request.max_items.unwrap_or(25) as usize;
    let mut results: Vec<DownloadProgress> = Vec::new();

    for item in request.items.into_iter().take(max_items) {
        if !item.url.starts_with("https://") {
            error!(url = %item.url, "Skipping non-https playlist item");
            continue;
        }

        let format = item.format.or(request.format).unwrap_or(AudioFormat::Mp3);
        let stream = StreamDownload {
            stream_url: item.url.clone(),
            title: item.title.clone(),
            platform: item
                .platform
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            format,
            ext: item
                .ext
                .clone()
                .unwrap_or_else(|| format.extension().to_string()),
            total_bytes: item.total_bytes,
        };

        match downloader.download(stream).await {
            Ok(id) => {
                if let Some(progress) = downloader.get_progress(id).await {
                    results.push(progress);
                }
            }
            Err(e) => {
                error!(url = %item.url, error = %e, "Failed to queue playlist track");
            }
        }
    }

    Ok(results)
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
