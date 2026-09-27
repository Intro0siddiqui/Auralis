//! Download Model
//!
//! Download progress tracking and status management.

use crate::AudioFormat;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// `skip_serializing_if` predicate for a plain `bool`.
///
/// A default-valued flag is noise in every payload it is unset in, and this
/// struct is emitted on every progress tick.
fn is_false(value: &bool) -> bool {
    !*value
}

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
    /// Verified, and being written to its final location.
    ///
    /// The window between "every check passed" and "the file is where it will
    /// stay". It exists because a job that is still `Downloading` across that
    /// window is a job a user can pause or cancel: a cancel would delete a
    /// finished, playable file, and a pause would report `Paused` over a job
    /// whose staging file has just been renamed away, leaving nothing to
    /// resume from. Neither is recoverable from the user's side, and neither
    /// leaves anything in a log a release build can read.
    ///
    /// Non-interruptible by contract — see `downloader::classify_interrupt`.
    Committing,
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
            DownloadStatus::Committing => write!(f, "committing"),
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
            DownloadStatus::Committing => "committing",
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

    /// The file is durable and this job is still doing work that can only
    /// improve it: cover art, tags, the copy in `Download/Auralis/`.
    ///
    /// This exists because the terminal transition was moved to the moment the
    /// rename lands, and everything after it is *not* part of the commit. The
    /// progress emitter ends on the first terminal status it observes, so
    /// without this flag it would emit `download:completed` — and stop — before
    /// the public path or the publish reason existed, silently dropping both.
    /// The alternative, keeping the state `Downloading` until publication
    /// finished, is the DL-02 defect itself: the window a user can cancel
    /// would cover the rename.
    ///
    /// So the file on disk and the event on the wire are given separate
    /// clocks, and this is the one that says which of them has settled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub post_commit: bool,

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
            post_commit: false,
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

        // Only a job that was queued or paused starts downloading here. The
        // unconditional form of this — "if it is not downloading, make it
        // downloading" — also reached backwards over `Committing` and
        // `Completed`, so any progress tick that landed after a terminal
        // transition would silently un-complete a finished download and hand
        // it back to the pause/cancel path that DL-02 exists to close.
        if matches!(self.status, DownloadStatus::Queued | DownloadStatus::Paused) {
            self.status = DownloadStatus::Downloading;
        }
    }

    /// Enter the commit boundary: the transfer is verified and the staging file
    /// is about to become the final file.
    ///
    /// Paired with [`Self::complete`] and both inside one hold of the
    /// downloader's commit gate, so no interrupt can land between them. Setting
    /// this on its own would be a state nothing could act on; that is the
    /// point of it — a request that arrives here is refused rather than obeyed.
    pub fn begin_commit(&mut self) {
        self.status = DownloadStatus::Committing;
        self.updated_at = Utc::now();
    }

    /// Mark as completed
    pub fn complete(&mut self, output_path: String) {
        self.status = DownloadStatus::Completed;
        self.progress = 1.0;
        self.output_path = Some(output_path);
        self.completed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// The file is committed; work that can only improve it has started.
    ///
    /// Paired with [`Self::finish_post_commit`]. Kept as two calls rather than
    /// set-and-unset inline at the call sites so the two halves cannot end up
    /// on opposite sides of a `return`.
    pub fn note_post_commit(&mut self) {
        self.post_commit = true;
        self.updated_at = Utc::now();
    }

    /// Everything that could improve the committed file has finished, so the
    /// progress record is now the whole story.
    pub fn finish_post_commit(&mut self) {
        self.post_commit = false;
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
    fn a_committed_job_is_terminal_before_the_work_that_improves_it() {
        // The shape DL-02 requires. The file exists from the rename, so the
        // state says so from the rename; cover art, tags and the public copy are
        // then reported separately by `post_commit`, which is what keeps the
        // progress emitter from ending the event stream with a record that
        // cannot yet say where the file went.
        let mut d = DownloadProgress::new(
            "https://youtu.be/x".to_string(),
            "Song".to_string(),
            AudioFormat::M4a,
        );
        assert!(!d.post_commit, "no post-commit work before there is a file");

        d.begin_commit();
        assert_eq!(d.status, DownloadStatus::Committing);
        assert!(!d.post_commit, "still nothing to report as settled");

        d.complete("/data/data/com.auralis.v2/files/downloads/Song.m4a".to_string());
        d.note_post_commit();
        assert_eq!(d.status, DownloadStatus::Completed);
        assert!(d.post_commit, "durable, but the public copy has not landed");

        d.finish_post_commit();
        assert_eq!(d.status, DownloadStatus::Completed);
        assert!(!d.post_commit, "now the record is the whole story");
    }

    #[test]
    fn a_progress_tick_cannot_resurrect_a_finished_download() {
        // The unconditional "if it is not downloading, make it downloading" in
        // `update` reached backwards over every terminal state. One stray tick
        // after the commit would hand a finished, playable download back to the
        // pause/cancel path — the exact window DL-02 closes.
        let mut d = DownloadProgress::new(
            "https://youtu.be/x".to_string(),
            "Song".to_string(),
            AudioFormat::M4a,
        );
        d.complete("/tmp/Song.m4a".to_string());
        d.update(1024, Some(2048), 512);
        assert_eq!(
            d.status,
            DownloadStatus::Completed,
            "a completed download stays completed"
        );

        let mut committing = DownloadProgress::new(
            "https://youtu.be/y".to_string(),
            "Other".to_string(),
            AudioFormat::M4a,
        );
        committing.begin_commit();
        committing.update(1024, Some(2048), 512);
        assert_eq!(
            committing.status,
            DownloadStatus::Committing,
            "and a job inside the commit boundary is not knocked out of it"
        );

        // A queued or paused job still starts on its first byte, which is the
        // whole reason that assignment exists.
        let mut queued = DownloadProgress::new(
            "https://youtu.be/z".to_string(),
            "Third".to_string(),
            AudioFormat::M4a,
        );
        queued.update(10, Some(100), 10);
        assert_eq!(queued.status, DownloadStatus::Downloading);
    }

    #[test]
    fn post_commit_reaches_the_wire_only_while_it_is_true() {
        // The emitter reads this field off the serialized payload, so a
        // `skip_serializing_if` that dropped the `true` case would make every
        // completed download look settled before its public copy existed.
        let mut d = DownloadProgress::new(
            "https://youtu.be/x".to_string(),
            "Song".to_string(),
            AudioFormat::M4a,
        );
        let before = serde_json::to_value(&d).expect("serialize");
        assert!(
            before.get("post_commit").is_none(),
            "an unset flag must not appear in the payload"
        );

        d.complete("/tmp/Song.m4a".to_string());
        d.note_post_commit();
        let during = serde_json::to_value(&d).expect("serialize");
        assert_eq!(
            during.get("post_commit").and_then(|v| v.as_bool()),
            Some(true),
            "a set flag must reach the emitter, or it ends the event early"
        );
    }

    #[test]
    fn committing_serializes_and_compares_as_its_own_state() {
        // The wire value is what the frontend branches on. A new variant that
        // serialized as something else would render as `Unknown` in the row.
        let mut d = DownloadProgress::new(
            "https://youtu.be/x".to_string(),
            "Song".to_string(),
            AudioFormat::M4a,
        );
        d.begin_commit();
        assert_eq!(
            serde_json::to_value(&d)
                .expect("serialize")
                .get("status")
                .and_then(|v| v.as_str()),
            Some("committing")
        );
        assert_eq!(d.status.to_string(), "committing");
        assert!(d.status == "committing");
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
