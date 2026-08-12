//! Download Commands
//!
//! Tauri command handlers for media downloads via yt-dlp.

use crate::domain::models::{AudioFormat, DownloadProgress};
use crate::infrastructure::media::downloader::Downloader;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tauri::State;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tracing::{error, info, warn};
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
    let max_items = request.max_items.unwrap_or(50) as usize;

    // Use yt-dlp to extract individual track URLs from the playlist.
    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        "--flat-playlist",
        "--dump-json",
        "--playlist-end",
        &max_items.to_string(),
        &request.url,
    ]);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().await.map_err(|e| {
        error!(error = %e, "Failed to launch yt-dlp for playlist extraction");
        format!("Failed to launch yt-dlp: {e}")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(stderr = %stderr, "yt-dlp playlist extraction failed");
        return Err(format!("Failed to extract playlist: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut track_urls: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(entry) => {
                if let Some(url) = entry.get("url").and_then(|u| u.as_str()) {
                    track_urls.push(url.to_string());
                }
            }
            Err(e) => {
                warn!(error = %e, line = %line, "Failed to parse playlist entry JSON");
            }
        }
    }

    if track_urls.is_empty() {
        return Err("No tracks found in playlist".to_string());
    }

    info!(count = track_urls.len(), "Extracted playlist track URLs");

    let mut results: Vec<DownloadProgress> = Vec::with_capacity(track_urls.len());

    for url in track_urls {
        match downloader.download(&url, format).await {
            Ok(id) => {
                if let Some(progress) = downloader.get_progress(id).await {
                    results.push(progress);
                }
            }
            Err(e) => {
                error!(url = %url, error = %e, "Failed to queue playlist track");
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
