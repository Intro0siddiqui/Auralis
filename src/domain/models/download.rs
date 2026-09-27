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

    /// Optional expected duration in seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_duration_secs: Option<u32>,

    /// Validated decoded duration in seconds (set when complete)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,

    /// Error message (if failed)
    pub error: Option<String>,
    /// Why the public copy did not land, when it did not.
    ///
    /// Separate from `error`, which reports whether the *download* failed. A
    /// download can succeed completely and still be invisible to the user, and
    /// that combination is the one no other surface reported: the row said
    /// "completed" while the file existed only in app-private storage. The
    /// reason is already built inside `publish_to_downloads` (it carries the
    /// display name, the API level and the JNI error) and was being discarded at
    /// the `None` boundary, leaving the failure visible but not diagnosable.
    ///
    /// `None` when there is nothing to report — the public copy succeeded, or
    /// this is not Android, so no public copy was attempted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_error: Option<String>,

    /// When the download was started
    pub started_at: DateTime<Utc>,

    /// When the download was last updated
    pub updated_at: DateTime<Utc>,

    /// When the download completed (or failed/cancelled)
    pub completed_at: Option<DateTime<Utc>>,
}

impl DownloadProgress {
    /// Create a new download progress tracker with a generated identifier.
    pub fn new(url: String, title: String, format: AudioFormat) -> Self {
        Self::with_id(Uuid::new_v4(), url, title, format)
    }

    /// Create a new download progress tracker with a caller-supplied identifier.
    ///
    /// The downloader uses this form so the serialized progress ID is exactly
    /// the ID used as the key for its job/task maps.
    pub fn with_id(id: Uuid, url: String, title: String, format: AudioFormat) -> Self {
        let now = Utc::now();
        Self {
            id,
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
            expected_duration_secs: None,
            duration_secs: None,
            error: None,
            publish_error: None,
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

    /// Record why the public copy did not land, on an otherwise completed
    /// download. Kept separate from [`Self::fail`] because the transfer itself
    /// succeeded — collapsing the two would report a successful download as a
    /// failure and hide the real problem behind a red row.
    pub fn note_publish_error(&mut self, reason: String) {
        self.publish_error = Some(reason);
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
    fn publish_failure_does_not_make_a_completed_download_look_failed() {
        // The bug this exists for: a download that transferred perfectly and
        // then could not be published to Download/Auralis. Reporting that as a
        // failure would be a lie about the transfer AND would hide the real
        // problem behind a red row, so the reason rides on its own field.
        let mut d = DownloadProgress::new(
            "https://youtu.be/hsXKOsnptw4".to_string(),
            "GO gyal".to_string(),
            AudioFormat::M4a,
        );
        assert!(d.publish_error.is_none(), "no reason before a publish");

        d.complete("/data/data/com.auralis.v2/files/downloads/GO gyal.mp4".to_string());
        d.note_publish_error("ContentResolver.insert returned no row id".to_string());

        assert_eq!(d.status, DownloadStatus::Completed, "transfer succeeded");
        assert_eq!(d.progress, 1.0);
        assert!(d.error.is_none(), "not a download error");
        assert_eq!(
            d.publish_error.as_deref(),
            Some("ContentResolver.insert returned no row id"),
            "the reason must survive to the event"
        );
    }

    #[test]
    fn publish_error_is_absent_from_the_wire_when_unset() {
        // The frontend branches on this field being absent, so `skip_serializing_if`
        // has to hold or every progress event grows a null nobody reads.
        let d = DownloadProgress::new(
            "https://youtu.be/x".to_string(),
            "t".to_string(),
            AudioFormat::M4a,
        );
        let json = serde_json::to_value(&d).expect("serialize");
        assert!(
            json.get("publish_error").is_none(),
            "unset publish_error must not appear in the payload"
        );

        let mut d2 = d;
        d2.note_publish_error("boom".to_string());
        let json2 = serde_json::to_value(&d2).expect("serialize");
        assert_eq!(
            json2.get("publish_error").and_then(|v| v.as_str()),
            Some("boom"),
            "a set publish_error must reach the frontend"
        );
    }

    #[test]
    fn test_download_creation_with_caller_id() {
        let id = Uuid::new_v4();
        let download = DownloadProgress::with_id(
            id,
            "https://youtube.com/watch?v=test".to_string(),
            "Test Song".to_string(),
            AudioFormat::Mp3,
        );

        assert_eq!(download.id, id);
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
