//! Media Downloader
//!
//! Streams media from a *resolved* direct audio URL (e.g. produced by the
//! frontend `youtube.js` resolver) to disk using `reqwest`, with pause/resume
//! (via HTTP `Range`) and cancel support.
//!
//! No external binaries (`yt-dlp` / `ffmpeg`) or dedicated Rust YouTube crates
//! are required — resolution of user-facing URLs (YouTube, SoundCloud, …) is
//! the frontend's responsibility; this layer only fetches bytes.

use crate::domain::models::{AudioFormat, DownloadProgress, DownloadStatus};
use chrono::Utc;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::{error, info};
use uuid::Uuid;

/// A fully-resolved download job submitted to the downloader.
///
/// The frontend resolves a user-facing URL into a directly streamable audio URL
/// (together with display metadata) before calling `download`.
pub struct StreamDownload {
    /// Direct, streamable audio URL (http/https).
    pub stream_url: String,
    /// Display title used for the output filename and UI.
    pub title: String,
    /// Source platform label (`youtube`, `direct`, …) for display.
    pub platform: String,
    /// Container/format metadata (display only; the bytes are saved with `ext`).
    pub format: AudioFormat,
    /// File extension for the saved bytes (e.g. `webm`, `m4a`, `mp3`).
    pub ext: String,
    /// Known total size in bytes, if available up-front.
    pub total_bytes: Option<u64>,
    /// Optional thumbnail/cover URL, fetched and saved as `<audio>.jpg`.
    pub thumbnail: Option<String>,
}

/// Per-job bookkeeping required to (re)start and resume a download.
#[derive(Clone)]
struct DownloadJob {
    stream_url: String,
    output_path: PathBuf,
    thumbnail: Option<String>,
}

/// Streams a resolved audio URL to disk with progress tracking.
#[derive(Clone)]
pub struct Downloader {
    output_dir: PathBuf,
    active_downloads: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    jobs: Arc<RwLock<HashMap<Uuid, DownloadJob>>>,
    tasks: Arc<RwLock<HashMap<Uuid, tokio::task::JoinHandle<()>>>>,
}

/// Downloader errors.
#[derive(Debug, thiserror::Error)]
pub enum DownloaderError {
    #[error("Download not found: {0}")]
    DownloadNotFound(Uuid),

    #[error("Invalid or unsupported URL: {0}")]
    InvalidUrl(String),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Download failed: {0}")]
    DownloadFailed(String),
}

/// Replace filesystem-unsafe characters so titles produce valid filenames.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    if trimmed.is_empty() {
        "audio_track".to_string()
    } else {
        trimmed
    }
}

impl Downloader {
    /// Create a new downloader that writes files into `output_dir`.
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            active_downloads: Arc::new(RwLock::new(HashMap::new())),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Begin streaming a resolved download. Returns the job id immediately;
    /// progress is tracked in `active_downloads` and surfaced via the
    /// `download:progress` / `download:completed` events emitted by the command.
    pub async fn download(&self, req: StreamDownload) -> Result<Uuid, DownloaderError> {
        info!(url = %req.stream_url, title = %req.title, "Starting download");

        if !req.stream_url.starts_with("https://") && !req.stream_url.starts_with("http://") {
            return Err(DownloaderError::InvalidUrl(req.stream_url));
        }

        let id = Uuid::new_v4();
        let ext = if req.ext.is_empty() {
            req.format.extension().to_string()
        } else {
            req.ext.clone()
        };
        let filename = format!("{}.{}", sanitize_filename(&req.title), ext);
        let path = self.output_dir.join(&filename);

        let mut progress =
            DownloadProgress::new(req.stream_url.clone(), req.title.clone(), req.format);
        progress.platform = req.platform.clone();
        progress.total_bytes = req.total_bytes;
        progress.status = DownloadStatus::Downloading;
        progress.output_path = Some(path.to_string_lossy().to_string());
        progress.started_at = Utc::now();
        progress.updated_at = Utc::now();

        {
            let mut downloads = self.active_downloads.write().await;
            downloads.insert(id, progress);
        }
        {
            let mut jobs = self.jobs.write().await;
            jobs.insert(
                id,
                DownloadJob {
                    stream_url: req.stream_url.clone(),
                    output_path: path,
                    thumbnail: req.thumbnail,
                },
            );
        }

        self.spawn_stream(id, 0).await;

        Ok(id)
    }

    /// Spawn the streaming task for `id`, resuming from `start_byte`.
    async fn spawn_stream(&self, id: Uuid, start_byte: u64) {
        let jobs = self.jobs.clone();
        let active = self.active_downloads.clone();
        let tasks = self.tasks.clone();

        let handle = tokio::spawn(async move {
            let job = {
                let jobs = jobs.read().await;
                match jobs.get(&id) {
                    Some(j) => j.clone(),
                    None => return,
                }
            };

            if let Err(e) = Self::run_stream(id, &job, start_byte, active.clone()).await {
                error!(download_id = %id, error = %e, "Download failed");
                let mut guard = active.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    state.fail(e.to_string());
                }
            }

            tasks.write().await.remove(&id);
        });

        let mut tasks = self.tasks.write().await;
        tasks.insert(id, handle);
    }

    /// Stream `job.stream_url` to `job.output_path`, updating progress.
    async fn run_stream(
        id: Uuid,
        job: &DownloadJob,
        start_byte: u64,
        active: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    ) -> Result<(), DownloaderError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| DownloaderError::HttpError(format!("failed to build HTTP client: {e}")))?;

        let mut req = client.get(&job.stream_url);
        if start_byte > 0 {
            req = req.header("Range", format!("bytes={}-", start_byte));
        }
        let mut res = req
            .send()
            .await
            .map_err(|e| DownloaderError::HttpError(format!("request failed: {e}")))?;

        let resuming = start_byte > 0 && res.status().as_u16() == 206;

        let total: Option<u64> = if resuming {
            res.headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.rsplit('/').next())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .or_else(|| res.content_length())
        } else {
            res.content_length()
        };

        if !res.status().is_success() && res.status().as_u16() != 206 {
            return Err(DownloaderError::HttpError(format!(
                "HTTP status {}",
                res.status()
            )));
        }

        {
            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.status = DownloadStatus::Downloading;
                if total.is_some() {
                    state.total_bytes = total;
                }
                state.updated_at = Utc::now();
            }
        }

        let mut file = if resuming {
            // File already holds the `start_byte` bytes we committed before the
            // pause; append the remainder.
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&job.output_path)
                .await?
        } else {
            tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&job.output_path)
                .await?
        };

        if !resuming {
            file.seek(SeekFrom::Start(0)).await?;
        }

        let start_instant = Instant::now();
        let mut downloaded = start_byte;

        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| DownloaderError::HttpError(format!("stream read error: {e}")))?
        {
            file.write_all(&chunk)
                .await
                .map_err(DownloaderError::IoError)?;
            downloaded += chunk.len() as u64;

            let elapsed = start_instant.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                ((downloaded - start_byte) as f64 / elapsed) as u64
            } else {
                0
            };

            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.downloaded_bytes = downloaded;
                if let Some(t) = total {
                    if t > 0 {
                        state.progress = (downloaded as f32) / (t as f32);
                    }
                }
                state.speed_bps = speed;
                state.updated_at = Utc::now();
            }
        }

        file.flush().await.map_err(DownloaderError::IoError)?;

        {
            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.complete(job.output_path.to_string_lossy().to_string());
            }
        }

        if let Some(thumb) = &job.thumbnail {
            Self::save_thumbnail(&client, thumb, &job.output_path).await;
        }

        info!(download_id = %id, path = ?job.output_path, "Download complete");
        Ok(())
    }

    /// Fetch a thumbnail/cover URL and save it as a `<audio>.jpg` sidecar so the
    /// library scanner can associate it with the downloaded track. Non-fatal.
    async fn save_thumbnail(client: &reqwest::Client, url: &str, audio_path: &Path) {
        let cover_path = audio_path.with_extension("jpg");
        let res = match client
            .get(url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };
        if let Ok(bytes) = res.bytes().await {
            if let Ok(mut f) = tokio::fs::File::create(&cover_path).await {
                let _ = f.write_all(&bytes).await;
                let _ = f.flush().await;
            }
        }
    }

    /// Pause an in-progress download by aborting its task and truncating the
    /// partial file to the last fully-written byte (so resume is clean).
    pub async fn pause(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Pausing download");

        if let Some(handle) = self.tasks.write().await.remove(&id) {
            handle.abort();
        }

        let snapshot = {
            let downloads = self.active_downloads.read().await;
            downloads
                .get(&id)
                .map(|s| (s.output_path.clone(), s.downloaded_bytes))
        };
        if let Some((Some(path), downloaded)) = snapshot {
            if let Ok(f) = tokio::fs::OpenOptions::new().write(true).open(&path).await {
                let _ = f.set_len(downloaded).await;
            }
        }

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            state.pause();
        }

        Ok(())
    }

    /// Resume a paused download from the last committed byte via HTTP Range.
    pub async fn resume(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Resuming download");

        let start = {
            let downloads = self.active_downloads.read().await;
            downloads
                .get(&id)
                .ok_or(DownloaderError::DownloadNotFound(id))?
                .downloaded_bytes
        };

        {
            let mut downloads = self.active_downloads.write().await;
            if let Some(state) = downloads.get_mut(&id) {
                if state.status == DownloadStatus::Paused {
                    state.status = DownloadStatus::Downloading;
                    state.updated_at = Utc::now();
                }
            }
        }

        self.spawn_stream(id, start).await;
        Ok(())
    }

    /// Cancel a download, killing its task and removing any partial file.
    pub async fn cancel(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Cancelling download");

        if let Some(handle) = self.tasks.write().await.remove(&id) {
            handle.abort();
        }

        let path = {
            let downloads = self.active_downloads.read().await;
            downloads.get(&id).and_then(|s| s.output_path.clone())
        };
        if let Some(path) = path {
            let _ = tokio::fs::remove_file(&path).await;
        }

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            state.cancel();
        }

        Ok(())
    }

    /// Get current progress for a download.
    pub async fn get_progress(&self, id: Uuid) -> Option<DownloadProgress> {
        let downloads = self.active_downloads.read().await;
        downloads.get(&id).cloned()
    }

    /// List all active downloads.
    pub async fn list_active(&self) -> Vec<DownloadProgress> {
        let downloads = self.active_downloads.read().await;
        downloads.values().cloned().collect()
    }
}
