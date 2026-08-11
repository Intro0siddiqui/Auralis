//! Media Downloader
//!
//! Downloads media using yt-dlp with progress tracking.

use crate::domain::models::{AudioFormat, DownloadProgress, DownloadStatus};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info};
use uuid::Uuid;

/// Active download process handle
struct ActiveDownload {
    id: Uuid,
    url: String,
    format: AudioFormat,
    child: Mutex<Option<tokio::process::Child>>,
}

/// Downloads media from various platforms using yt-dlp
#[derive(Clone)]
pub struct Downloader {
    output_dir: PathBuf,
    ffmpeg_path: Option<String>,
    active_downloads: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    processes: Arc<RwLock<HashMap<Uuid, Arc<ActiveDownload>>>>,
}

/// Media information
#[derive(Debug)]
pub struct MediaInfo {
    pub title: String,
    pub duration: u32,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
}

/// Downloader errors
#[derive(Debug, thiserror::Error)]
pub enum DownloaderError {
    #[error("yt-dlp not found")]
    YtDlpNotFound,

    #[error("Process error: {0}")]
    ProcessError(String),

    #[error("Download failed with code {0}")]
    DownloadFailed(i32),

    #[error("Failed to get info")]
    InfoFailed,

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Output file not found")]
    OutputNotFound,

    #[error("Download not found: {0}")]
    DownloadNotFound(Uuid),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl Downloader {
    /// Create a new downloader
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            ffmpeg_path: None,
            active_downloads: Arc::new(RwLock::new(HashMap::new())),
            processes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set custom ffmpeg path
    pub fn with_ffmpeg(mut self, path: String) -> Self {
        self.ffmpeg_path = Some(path);
        self
    }

    /// Start a download
    pub async fn download(&self, url: &str, format: AudioFormat) -> Result<Uuid, DownloaderError> {
        info!(url = %url, format = ?format, "Starting download");

        if !self.is_ytdlp_available().await {
            return Err(DownloaderError::YtDlpNotFound);
        }

        let title = format!("Download from {}", Self::detect_platform(url));
        let progress = DownloadProgress::new(url.to_string(), title, format);
        let id = progress.id;

        {
            let mut downloads = self.active_downloads.write().await;
            downloads.insert(id, progress);
        }

        let active = Arc::new(ActiveDownload {
            id,
            url: url.to_string(),
            format,
            child: Mutex::new(None),
        });

        {
            let mut processes = self.processes.write().await;
            processes.insert(id, active.clone());
        }

        let downloads = self.active_downloads.clone();
        let processes = self.processes.clone();
        let ffmpeg_path = self.ffmpeg_path.clone();
        let output_dir = self.output_dir.clone();

        let url = url.to_string();

        tokio::spawn(async move {
            if let Err(e) = Self::run_download(
                id,
                &url,
                format,
                ffmpeg_path,
                output_dir,
                downloads.clone(),
                processes.clone(),
            )
            .await
            {
                error!(download_id = %id, error = %e, "Download failed");
                let mut guard = downloads.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    state.fail(e.to_string());
                }
            }
        });

        Ok(id)
    }

    /// Pause a download by sending SIGSTOP to the process
    pub async fn pause(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Pausing download");

        {
            let downloads = self.active_downloads.read().await;
            if !downloads.contains_key(&id) {
                return Err(DownloaderError::DownloadNotFound(id));
            }
        }

        let processes = self.processes.read().await;
        let active = processes
            .get(&id)
            .ok_or(DownloaderError::DownloadNotFound(id))?;

        let child_guard = active.child.lock().await;
        if let Some(ref child) = *child_guard {
            let pid = child.id().ok_or_else(|| {
                DownloaderError::ProcessError("Failed to get process ID".to_string())
            })?;

            let nix_pid = Pid::from_raw(pid as i32);
            kill(nix_pid, Signal::SIGSTOP).map_err(|e| {
                DownloaderError::ProcessError(format!("Failed to send SIGSTOP: {e}"))
            })?;
        }
        drop(child_guard);

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            state.pause();
        }

        Ok(())
    }

    /// Resume a paused download by sending SIGCONT
    pub async fn resume(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Resuming download");

        {
            let downloads = self.active_downloads.read().await;
            if !downloads.contains_key(&id) {
                return Err(DownloaderError::DownloadNotFound(id));
            }
        }

        let processes = self.processes.read().await;
        let active = processes
            .get(&id)
            .ok_or(DownloaderError::DownloadNotFound(id))?;

        let child_guard = active.child.lock().await;
        if let Some(ref child) = *child_guard {
            let pid = child.id().ok_or_else(|| {
                DownloaderError::ProcessError("Failed to get process ID".to_string())
            })?;

            let nix_pid = Pid::from_raw(pid as i32);
            kill(nix_pid, Signal::SIGCONT).map_err(|e| {
                DownloaderError::ProcessError(format!("Failed to send SIGCONT: {e}"))
            })?;
        }
        drop(child_guard);

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            if state.status == DownloadStatus::Paused {
                state.status = DownloadStatus::Downloading;
                state.updated_at = chrono::Utc::now();
            }
        }

        Ok(())
    }

    /// Cancel a download by killing the process
    pub async fn cancel(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Cancelling download");

        {
            let downloads = self.active_downloads.read().await;
            if !downloads.contains_key(&id) {
                return Err(DownloaderError::DownloadNotFound(id));
            }
        }

        let processes = self.processes.read().await;
        let active = processes
            .get(&id)
            .ok_or(DownloaderError::DownloadNotFound(id))?;

        let mut child_guard = active.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            child
                .kill()
                .await
                .map_err(|e| DownloaderError::ProcessError(e.to_string()))?;
        }
        drop(child_guard);

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            state.cancel();
        }

        Ok(())
    }

    /// Get current progress for a download
    pub async fn get_progress(&self, id: Uuid) -> Option<DownloadProgress> {
        let downloads = self.active_downloads.read().await;
        downloads.get(&id).cloned()
    }

    /// List all active downloads
    pub async fn list_active(&self) -> Vec<DownloadProgress> {
        let downloads = self.active_downloads.read().await;
        downloads.values().cloned().collect()
    }

    /// Get video info without downloading
    pub async fn get_info(&self, url: &str) -> Result<MediaInfo, DownloaderError> {
        info!(url = %url, "Getting media info");

        let output = Command::new("yt-dlp")
            .args(["--dump-json", "--no-download", url])
            .output()
            .await
            .map_err(|e| DownloaderError::ProcessError(e.to_string()))?;

        if !output.status.success() {
            return Err(DownloaderError::InfoFailed);
        }

        let json = String::from_utf8_lossy(&output.stdout);
        let info: serde_json::Value = serde_json::from_str(&json)
            .map_err(|_| DownloaderError::ParseError("Invalid JSON".to_string()))?;

        Ok(MediaInfo {
            title: info["title"].as_str().unwrap_or("Unknown").to_string(),
            duration: info["duration"].as_f64().unwrap_or(0.0) as u32,
            uploader: info["uploader"].as_str().map(|s| s.to_string()),
            thumbnail: info["thumbnail"].as_str().map(|s| s.to_string()),
        })
    }

    /// Check if yt-dlp is available
    async fn is_ytdlp_available(&self) -> bool {
        Command::new("yt-dlp")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Detect platform from URL
    fn detect_platform(url: &str) -> String {
        if url.contains("youtube.com") || url.contains("youtu.be") {
            "youtube".to_string()
        } else if url.contains("soundcloud.com") {
            "soundcloud".to_string()
        } else if url.contains("instagram.com") {
            "instagram".to_string()
        } else if url.contains("bandcamp.com") {
            "bandcamp".to_string()
        } else {
            "unknown".to_string()
        }
    }

    /// Run the actual yt-dlp download process
    async fn run_download(
        id: Uuid,
        url: &str,
        format: AudioFormat,
        ffmpeg_path: Option<String>,
        output_dir: PathBuf,
        downloads: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
        processes: Arc<RwLock<HashMap<Uuid, Arc<ActiveDownload>>>>,
    ) -> Result<(), DownloaderError> {
        let output_template = output_dir
            .join("%(title)s-%(id)s.%(ext)s")
            .to_string_lossy()
            .to_string();

        let mut cmd = Command::new("yt-dlp");
        cmd.args([
            "-x",
            "--audio-format",
            format.extension(),
            "-o",
            &output_template,
            "--no-playlist",
            "--newline",
            "--progress-template",
            "download:%(progress._percent_str)s %(progress._speed_str)s %(progress._eta_str)s",
            "-f",
            "bestaudio/best",
        ]);

        if let Some(ffmpeg) = ffmpeg_path {
            cmd.arg("--ffmpeg-location").arg(ffmpeg);
        }

        cmd.arg(url);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        debug!(command = ?cmd, "Executing yt-dlp");

        let child = cmd
            .spawn()
            .map_err(|e| DownloaderError::ProcessError(e.to_string()))?;

        {
            let processes = processes.read().await;
            if let Some(active) = processes.get(&id) {
                let mut child_guard = active.child.lock().await;
                *child_guard = Some(child);
            }
        }

        {
            let mut guard = downloads.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.status = DownloadStatus::Downloading;
                state.updated_at = chrono::Utc::now();
            }
        }

        let stdout = {
            let processes = processes.read().await;
            let active = processes
                .get(&id)
                .ok_or(DownloaderError::DownloadNotFound(id))?;
            let mut child_guard = active.child.lock().await;
            let child = child_guard.as_mut().ok_or_else(|| {
                DownloaderError::ProcessError("Child process not available".to_string())
            })?;
            child
                .stdout
                .take()
                .ok_or_else(|| DownloaderError::ProcessError("No stdout".to_string()))?
        };

        let mut reader = BufReader::new(stdout).lines();

        while let Some(line_result) = reader.next_line().await.transpose() {
            let line = line_result.map_err(|e| DownloaderError::ProcessError(e.to_string()))?;
            debug!(line = %line, "yt-dlp output");

            if line.contains("[download]") && line.contains('%') {
                let mut guard = downloads.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    if let Some(pct) = Self::parse_percentage(&line) {
                        state.progress = pct / 100.0;
                    }
                    if let Some(speed) = Self::parse_speed(&line) {
                        state.speed_bps = speed;
                    }
                    if let Some(eta) = Self::parse_eta(&line) {
                        state.eta_secs = Some(eta);
                    }
                    if let Some(total) = Self::parse_total_bytes(&line) {
                        state.total_bytes = Some(total);
                    }
                    state.downloaded_bytes =
                        (state.progress * state.total_bytes.unwrap_or(0) as f32) as u64;
                    state.updated_at = chrono::Utc::now();
                }
            }

            if line.contains("Destination:") || line.contains("[ExtractAudio]") {
                let mut guard = downloads.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    if let Some(start) = line.find("Destination:") {
                        let path = line[start + 13..].trim().to_string();
                        state.output_path = Some(path);
                    }
                }
            }
        }

        let status = {
            let processes = processes.read().await;
            let active = processes
                .get(&id)
                .ok_or(DownloaderError::DownloadNotFound(id))?;
            let mut child_guard = active.child.lock().await;
            if let Some(mut child) = child_guard.take() {
                child
                    .wait()
                    .await
                    .map_err(|e| DownloaderError::ProcessError(e.to_string()))?
            } else {
                return Err(DownloaderError::ProcessError(
                    "Process already gone".to_string(),
                ));
            }
        };

        let mut guard = downloads.write().await;
        if let Some(state) = guard.get_mut(&id) {
            if status.success() {
                state.status = DownloadStatus::Completed;
                state.progress = 1.0;
                state.completed_at = Some(chrono::Utc::now());
                state.updated_at = chrono::Utc::now();

                if state.output_path.is_none() {
                    state.output_path = Self::find_output_file(&output_dir).await;
                }

                info!(download_id = %id, "Download completed");
            } else {
                state.status = DownloadStatus::Failed;
                state.error = Some(format!(
                    "yt-dlp exited with code {}",
                    status.code().unwrap_or(-1)
                ));
                state.completed_at = Some(chrono::Utc::now());
                state.updated_at = chrono::Utc::now();
            }
        }

        if !status.success() {
            return Err(DownloaderError::DownloadFailed(status.code().unwrap_or(-1)));
        }

        Ok(())
    }

    /// Parse download percentage from output line
    fn parse_percentage(line: &str) -> Option<f32> {
        if let Some(idx) = line.find('%') {
            let start = line[..idx]
                .rfind(|c: char| !c.is_ascii_digit() && c != '.' && c != ' ')
                .map(|i| i + 1)
                .unwrap_or(0);
            line[start..idx].trim().parse().ok()
        } else {
            None
        }
    }

    /// Parse download speed from output line
    fn parse_speed(line: &str) -> Option<u64> {
        if let Some(idx) = line.find("at ") {
            let rest = &line[idx + 3..];
            if let Some(end_idx) = rest.find(['/', 's']) {
                let speed_str = rest[..end_idx].trim();
                if speed_str.contains("MiB") {
                    let value: f64 = speed_str.replace("MiB", "").trim().parse().ok()?;
                    Some((value * 1024.0 * 1024.0) as u64)
                } else if speed_str.contains("KiB") {
                    let value: f64 = speed_str.replace("KiB", "").trim().parse().ok()?;
                    Some((value * 1024.0) as u64)
                } else {
                    speed_str.parse().ok()
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Parse ETA from output line
    fn parse_eta(line: &str) -> Option<u32> {
        if let Some(idx) = line.find("ETA ") {
            let eta_str = &line[idx + 4..];
            let parts: Vec<&str> = eta_str.split(':').collect();
            match parts.len() {
                2 => {
                    let mins: u32 = parts[0].trim().parse().ok()?;
                    let secs: u32 = parts[1].trim().parse().ok()?;
                    Some(mins * 60 + secs)
                }
                3 => {
                    let hrs: u32 = parts[0].trim().parse().ok()?;
                    let mins: u32 = parts[1].trim().parse().ok()?;
                    let secs: u32 = parts[2].trim().parse().ok()?;
                    Some(hrs * 3600 + mins * 60 + secs)
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Parse total bytes from output line
    fn parse_total_bytes(line: &str) -> Option<u64> {
        if let Some(idx) = line.find("of ~") {
            let rest = &line[idx + 4..];
            let end_idx = rest.find(" at").or_else(|| rest.find(" ETA"))?;
            let size_str = rest[..end_idx].trim();
            if size_str.contains("MiB") {
                let value: f64 = size_str.replace("MiB", "").trim().parse().ok()?;
                Some((value * 1024.0 * 1024.0) as u64)
            } else if size_str.contains("KiB") {
                let value: f64 = size_str.replace("KiB", "").trim().parse().ok()?;
                Some((value * 1024.0) as u64)
            } else if size_str.contains("GiB") {
                let value: f64 = size_str.replace("GiB", "").trim().parse().ok()?;
                Some((value * 1024.0 * 1024.0 * 1024.0) as u64)
            } else {
                size_str.parse().ok()
            }
        } else {
            None
        }
    }

    /// Find the output file after download
    async fn find_output_file(output_dir: &PathBuf) -> Option<String> {
        let mut entries = tokio::fs::read_dir(output_dir).await.ok()?;
        let mut latest: Option<(String, std::time::SystemTime)> = None;

        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(metadata) = entry.metadata().await {
                if metadata.is_file() {
                    let modified = metadata.modified().ok()?;
                    let path = entry.path().to_string_lossy().to_string();
                    if latest.as_ref().map_or(true, |(_, t)| modified > *t) {
                        latest = Some((path, modified));
                    }
                }
            }
        }

        latest.map(|(p, _)| p)
    }
}
