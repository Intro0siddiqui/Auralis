//! Media Downloader
//!
//! Downloads media using yt-dlp.

use crate::domain::models::AudioFormat;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, info};

/// Downloads media from various platforms using yt-dlp
pub struct Downloader {
    output_template: String,
    ffmpeg_path: Option<String>,
}

impl Downloader {
    /// Create a new downloader
    pub fn new(output_dir: PathBuf) -> Self {
        let output_template = output_dir
            .join("%(title)s-%(id)s.%(ext)s")
            .to_string_lossy()
            .to_string();

        Self {
            output_template,
            ffmpeg_path: None,
        }
    }

    /// Set custom ffmpeg path
    pub fn with_ffmpeg(mut self, path: String) -> Self {
        self.ffmpeg_path = Some(path);
        self
    }

    /// Download audio from a URL
    pub async fn download(
        &self,
        url: &str,
        format: AudioFormat,
    ) -> Result<DownloadResult, DownloaderError> {
        info!(url = %url, format = ?format, "Starting download");

        // Check if yt-dlp is available
        if !self.is_ytdlp_available().await {
            return Err(DownloaderError::YtDlpNotFound);
        }

        // Build command
        let mut cmd = Command::new("yt-dlp");
        cmd.args([
            "-x",                      // Extract audio
            "--audio-format", format.extension(),
            "-o", &self.output_template,
            "--no-playlist",
            "--newline",
            "-f", "bestaudio/best",
        ]);

        if let Some(ref ffmpeg) = self.ffmpeg_path {
            cmd.arg("--ffmpeg-location").arg(ffmpeg);
        }

        cmd.arg(url);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        debug!(command = ?cmd, "Executing yt-dlp");

        let mut child = cmd.spawn()
            .map_err(|e| DownloaderError::ProcessError(e.to_string()))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| DownloaderError::ProcessError("Failed to capture stdout".to_string()))?;

        let mut reader = BufReader::new(stdout).lines();

        // Parse progress from yt-dlp output
        while let Ok(Some(line)) = reader.next_line().await {
            debug!(line = %line, "yt-dlp output");
            // Parse progress information from line
            // yt-dlp outputs progress like: [download]   0.0% of ~50.00MiB at  1.00MiB/s ETA 00:49
            if line.contains("[download]") && line.contains('%') {
                if let Some(pct) = self.parse_percentage(&line) {
                    debug!(progress = pct / 100.0, "Download progress");
                }
                if let Some(speed) = self.parse_speed(&line) {
                    debug!(speed_bps = speed, "Download speed");
                }
            }
        }

        let status = child.wait().await
            .map_err(|e| DownloaderError::ProcessError(e.to_string()))?;

        if !status.success() {
            return Err(DownloaderError::DownloadFailed(
                status.code().unwrap_or(-1)
            ));
        }

        info!(url = %url, "Download completed");
        Ok(DownloadResult {
            output_path: self.find_output_file().await?,
            duration_secs: 0, // Would need to extract from metadata
            metadata: None,
        })
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

    /// Parse download percentage
    fn parse_percentage(&self, line: &str) -> Option<f32> {
        // Extract percentage from lines like "[download]  45.2% of ~100.00MiB"
        if let Some(idx) = line.find('%') {
            let start = line[..idx].rfind(|c: char| !c.is_ascii_digit() && c != '.')
                .map(|i| i + 1)
                .unwrap_or(0);
            line[start..idx].parse().ok()
        } else {
            None
        }
    }

    /// Parse download speed
    fn parse_speed(&self, line: &str) -> Option<u64> {
        // Extract speed from lines like "at  1.00MiB/s"
        if let Some(idx) = line.find("at") {
            let rest = &line[idx + 2..];
            if let Some(end_idx) = rest.find("B/s") {
                let speed_str = rest[..end_idx].trim();
                // Parse with unit conversion
                if speed_str.contains("MiB") {
                    let value: f64 = speed_str.replace("MiB", "").trim().parse().unwrap_or(0.0);
                    Some((value * 1024.0 * 1024.0) as u64)
                } else if speed_str.contains("KiB") {
                    let value: f64 = speed_str.replace("KiB", "").trim().parse().unwrap_or(0.0);
                    Some((value * 1024.0) as u64)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Find the output file after download
    async fn find_output_file(&self) -> Result<PathBuf, DownloaderError> {
        // In a real implementation, we'd parse yt-dlp output to get the exact filename
        // For now, this is a placeholder
        Err(DownloaderError::OutputNotFound)
    }
}

/// Download progress information
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: u64,
    pub progress: f32,
}

/// Download result
#[derive(Debug)]
pub struct DownloadResult {
    pub output_path: PathBuf,
    pub duration_secs: u32,
    pub metadata: Option<serde_json::Value>,
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
}
