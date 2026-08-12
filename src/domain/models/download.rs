//! Download Model
//!
//! Download progress tracking and status management.

use crate::AudioFormat;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Download job status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DownloadStatus {
    /// Download is queued but not started
    #[default]
    Queued,
    /// Currently downloading
    Downloading,
    /// Paused by user
    Paused,
    /// Completed successfully
    Completed,
    /// Failed with error
    Failed,
    /// Cancelled by user
    Cancelled,
}

impl fmt::Display for DownloadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadStatus::Queued => write!(f, "queued"),
            DownloadStatus::Downloading => write!(f, "downloading"),
            DownloadStatus::Paused => write!(f, "paused"),
            DownloadStatus::Completed => write!(f, "completed"),
            DownloadStatus::Failed => write!(f, "failed"),
            DownloadStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl PartialEq<&str> for DownloadStatus {
    fn eq(&self, other: &&str) -> bool {
        let self_str = match self {
            DownloadStatus::Queued => "queued",
            DownloadStatus::Downloading => "downloading",
            DownloadStatus::Paused => "paused",
            DownloadStatus::Completed => "completed",
            DownloadStatus::Failed => "failed",
            DownloadStatus::Cancelled => "cancelled",
        };
        self_str == *other
    }
}

/// Download progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    /// Unique download identifier
    pub id: Uuid,

    /// Source URL
    pub url: String,

    /// Source platform (youtube, instagram, etc.)
    pub platform: String,

    /// Track title (may be updated after metadata extraction)
    pub title: String,

    /// Current status
    pub status: DownloadStatus,

    /// Progress percentage (0.0 to 1.0)
    pub progress: f32,

    /// Downloaded bytes
    pub downloaded_bytes: u64,

    /// Total bytes (if known)
    pub total_bytes: Option<u64>,

    /// Download speed in bytes per second
    pub speed_bps: u64,

    /// Estimated remaining time in seconds
    pub eta_secs: Option<u32>,

    /// Target audio format
    pub format: AudioFormat,

    /// Output file path (set when completed)
    pub output_path: Option<String>,

    /// Error message (if failed)
    pub error: Option<String>,

    /// When the download was started
    pub started_at: DateTime<Utc>,

    /// When the download was last updated
    pub updated_at: DateTime<Utc>,

    /// When the download completed (or failed/cancelled)
    pub completed_at: Option<DateTime<Utc>>,
}

impl DownloadProgress {
    /// Create a new download progress tracker
    pub fn new(url: String, title: String, format: AudioFormat) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            url: url.clone(),
            platform: Self::detect_platform(&url),
            title,
            status: DownloadStatus::Queued,
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes: None,
            speed_bps: 0,
            eta_secs: None,
            format,
            output_path: None,
            error: None,
            started_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    /// Detect platform from URL
    fn detect_platform(url: &str) -> String {
        if url.contains("youtube.com") || url.contains("youtu.be") {
            "youtube".to_string()
        } else if url.contains("instagram.com") {
            "instagram".to_string()
        } else {
            "unknown".to_string()
        }
    }

    /// Update progress
    pub fn update(&mut self, downloaded_bytes: u64, total_bytes: Option<u64>, speed_bps: u64) {
        self.downloaded_bytes = downloaded_bytes;
        self.total_bytes = total_bytes;
        self.speed_bps = speed_bps;
        self.updated_at = Utc::now();

        if let Some(total) = total_bytes {
            if total > 0 {
                self.progress = downloaded_bytes as f32 / total as f32;
                let remaining = total.saturating_sub(downloaded_bytes);
                self.eta_secs = (remaining as u32).checked_div(speed_bps as u32);
            }
        }

        if self.status != DownloadStatus::Downloading {
            self.status = DownloadStatus::Downloading;
        }
    }

    /// Mark as completed
    pub fn complete(&mut self, output_path: String) {
        self.status = DownloadStatus::Completed;
        self.progress = 1.0;
        self.output_path = Some(output_path);
        self.completed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Mark as failed
    pub fn fail(&mut self, error: String) {
        self.status = DownloadStatus::Failed;
        self.error = Some(error);
        self.completed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Mark as paused
    pub fn pause(&mut self) {
        if self.status == DownloadStatus::Downloading {
            self.status = DownloadStatus::Paused;
            self.updated_at = Utc::now();
        }
    }

    /// Mark as cancelled
    pub fn cancel(&mut self) {
        self.status = DownloadStatus::Cancelled;
        self.completed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Get formatted speed
    pub fn formatted_speed(&self) -> String {
        format_speed(self.speed_bps)
    }

    /// Get formatted size
    pub fn formatted_size(&self) -> String {
        let downloaded = format_size(self.downloaded_bytes);
        match self.total_bytes {
            Some(total) => format!("{} / {}", downloaded, format_size(total)),
            None => downloaded,
        }
    }

    /// Get formatted ETA
    pub fn formatted_eta(&self) -> String {
        match self.eta_secs {
            Some(secs) => {
                let minutes = secs / 60;
                let seconds = secs % 60;
                format!("{}:{:02}", minutes, seconds)
            }
            None => "--:--".to_string(),
        }
    }
}

/// Format bytes as human-readable size
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format bytes per second as human-readable speed
pub fn format_speed(bytes_per_sec: u64) -> String {
    format!("{}/s", format_size(bytes_per_sec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_creation() {
        let download = DownloadProgress::new(
            "https://youtube.com/watch?v=test".to_string(),
            "Test Song".to_string(),
            AudioFormat::Mp3,
        );

        assert_eq!(download.platform, "youtube");
        assert_eq!(download.status, DownloadStatus::Queued);
        assert_eq!(download.progress, 0.0);
    }

    #[test]
    fn test_progress_update() {
        let mut download = DownloadProgress::new(
            "https://youtube.com/watch?v=test".to_string(),
            "Test Song".to_string(),
            AudioFormat::Mp3,
        );

        download.update(50_000_000, Some(100_000_000), 10_000_000);

        assert_eq!(download.progress, 0.5);
        assert_eq!(download.status, DownloadStatus::Downloading);
        assert_eq!(download.eta_secs, Some(5));
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1_500_000), "1.4 MB");
        assert_eq!(format_size(1_500_000_000), "1.4 GB");
    }
}
