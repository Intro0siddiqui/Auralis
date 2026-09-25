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
use lofty::file::{AudioFile, FileType};
use lofty::probe::Probe;
use std::collections::HashMap;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Remove a staging `.part` file, logging any error at debug level.
pub(crate) async fn cleanup_staging_file(path: &Path) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        debug!(path = %path.display(), error = %e, "Failed to remove staging file during cleanup");
    } else {
        debug!(path = %path.display(), "Cleaned up staging file");
    }
}

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
    /// Optional HTTP headers to send with the googlevideo request (UA/Referer
    /// matched to the InnerTube client that generated the URL). If absent,
    /// sane YouTube defaults are injected.
    pub headers: Option<HashMap<String, String>>,
    /// Expected duration in seconds if known up-front from metadata.
    pub expected_duration_secs: Option<u32>,
}

/// Per-job bookkeeping required to (re)start and resume a download.
#[derive(Clone)]
struct DownloadJob {
    stream_url: String,
    output_path: PathBuf,
    staging_path: PathBuf,
    thumbnail: Option<String>,
    headers: Option<HashMap<String, String>>,
    expected_duration_secs: Option<u32>,
    total_bytes: Option<u64>,
    format: AudioFormat,
    ext: String,
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

const ALLOWED_EXTS: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "ogg", "opus", "wav", "webm", "mp4", "mov", "oga",
];

/// Whitelist and sanitize an extension string. Returns a safe extension from
/// the allow-list; falls back to the trusted `fallback` (AudioFormat) or "mp3".
fn sanitize_ext(raw: &str, fallback: &str) -> String {
    let t = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    // must be purely alphanumeric and on allow-list — any slash, dot, or
    // control char causes fallback (prevents traversal like "../../etc")
    let is_clean = !t.is_empty() && t.len() <= 8 && t.chars().all(|c| c.is_ascii_alphanumeric());
    if is_clean && ALLOWED_EXTS.contains(&t.as_str()) {
        return t;
    }
    let fb = fallback.trim().trim_start_matches('.').to_ascii_lowercase();
    let fb_clean: String = fb.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if ALLOWED_EXTS.contains(&fb_clean.as_str()) {
        fb_clean
    } else {
        "mp3".to_string()
    }
}

/// Helper to extract an integer query parameter from a URL (e.g. `clen=15234567`).
fn extract_url_param_u64(url: &str, param: &str) -> Option<u64> {
    let key = format!("{param}=");
    let start = url.find(&key)? + key.len();
    let val_str = &url[start..];
    let end = val_str.find('&').unwrap_or(val_str.len());
    val_str[..end].parse::<u64>().ok()
}

/// Slack allowed when comparing received bytes against the advertised object
/// size. A few tens of KiB absorbs container/manifest rounding differences
/// between what the resolver reports and what the edge serves.
const COMPLETE_TOLERANCE_BYTES: u64 = 64 * 1024;

/// Minimum plausible audio bitrate (≈64 kbps) used to sanity-check an
/// advertised object size against the known track duration. A 4-minute track
/// that "totals" 1 MB is a truncated response window, not a 1-minute song.
const MIN_PLAUSIBLE_BYTES_PER_SEC: u64 = 8_000;

/// Tracks shorter than this are exempt from the bitrate plausibility check
/// (very short clips legitimately have tiny byte counts and rounding noise).
const MIN_DURATION_FOR_BITRATE_CHECK: u32 = 20;

/// Bytes/second floor used **only** when neither the resolver nor the server
/// gave a size: ≈32 kbps. Used to catch "the stream just stopped" truncations.
const MIN_BYTES_PER_SEC_FALLBACK: u64 = 4_000;

/// Remove query parameters that cap the response window (`range`, `range2`).
///
/// Throttled googlevideo URLs can carry `&range=0-1048575`. The edge then serves
/// only that window and reports it as the whole object, so a partial download
/// is indistinguishable from a complete one. Dropping the parameter makes the
/// server return the full resource.
fn strip_response_range_params(url: &str) -> String {
    let mut removed: Vec<String> = Vec::new();
    let filtered = url
        .split('&')
        .filter(|part| {
            let key = part.split('=').next().unwrap_or("");
            let is_cap = key.eq_ignore_ascii_case("range") || key.eq_ignore_ascii_case("range2");
            if is_cap {
                removed.push(key.to_string());
            }
            !is_cap
        })
        .collect::<Vec<_>>()
        .join("&");
    if removed.is_empty() {
        return url.to_string();
    }
    info!(
        removed = ?removed,
        "Stripped response-capping parameter(s) from stream URL"
    );
    filtered
}

/// Parse the total object length from a `Content-Range` header
/// (`bytes 0-99/1234` or the unsatisfied form `bytes */1234`).
/// Returns `None` when the total is `*` or unparsable.
fn parse_content_range_total(value: &str) -> Option<u64> {
    let total = value.rsplit('/').next()?.trim();
    if total.is_empty() || total == "*" {
        return None;
    }
    total.parse::<u64>().ok()
}

/// Apply the headers googlevideo validates (`Referer`/`Origin`/`Accept`) plus any
/// client-matched headers produced by the frontend resolver. The User-Agent is
/// configured on the `reqwest` client itself, so it is skipped here.
fn inject_stream_headers(
    mut req: reqwest::RequestBuilder,
    job_headers: Option<&HashMap<String, String>>,
) -> reqwest::RequestBuilder {
    let mut injected: HashMap<String, String> = HashMap::new();
    injected.insert(
        "Referer".to_string(),
        "https://www.youtube.com/".to_string(),
    );
    injected.insert("Origin".to_string(), "https://www.youtube.com".to_string());
    injected.insert("Accept".to_string(), "*/*".to_string());
    injected.insert("Accept-Language".to_string(), "en-US,en;q=0.9".to_string());
    injected.insert("Sec-Fetch-Mode".to_string(), "no-cors".to_string());
    injected.insert("Connection".to_string(), "keep-alive".to_string());
    if let Some(h) = job_headers {
        for (k, v) in h {
            if k.eq_ignore_ascii_case("user-agent") {
                continue;
            }
            injected.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in injected {
        req = req.header(k, v);
    }
    req
}

/// Ask the server for the authoritative object size with a 1-byte ranged GET.
///
/// `Content-Range: bytes 0-0/TOTAL` (or a full `Content-Length` when the server
/// ignores `Range`) yields the real size. This is the number completeness is
/// judged against when the resolver-advertised size looks implausible, which is
/// the failure mode where a half file used to pass validation and be renamed.
async fn probe_total_bytes(
    client: &reqwest::Client,
    url: &str,
    job_headers: Option<&HashMap<String, String>>,
) -> Option<u64> {
    let req = inject_stream_headers(client.get(url), job_headers).header("Range", "bytes=0-0");
    let res = match tokio::time::timeout(Duration::from_secs(15), req.send()).await {
        Ok(Ok(r)) => r,
        _ => return None,
    };
    if let Some(cr) = res
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(total) = parse_content_range_total(cr) {
            return Some(total);
        }
    }
    if res.status().is_success() {
        return res.content_length().filter(|c| *c > 0);
    }
    None
}

/// Helper to extract a float query parameter from a URL (e.g. `dur=245.123`).
fn extract_url_param_f64(url: &str, param: &str) -> Option<f64> {
    let key = format!("{param}=");
    let start = url.find(&key)? + key.len();
    let val_str = &url[start..];
    let end = val_str.find('&').unwrap_or(val_str.len());
    val_str[..end].parse::<f64>().ok()
}

/// Replace filesystem-unsafe characters so titles produce valid filenames.
/// Strips path separators, control chars, "..", reserved Windows names, and
/// limits length to 200 chars. Never returns empty or "." / "..".
fn sanitize_filename(name: &str) -> String {
    // Replace control chars and map path separators/unsafe chars to '_'
    let filtered: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = filtered
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut trimmed = cleaned.trim().trim_matches('.').to_string();
    // Collapse any remaining ".." to avoid traversal
    while trimmed.contains("..") {
        trimmed = trimmed.replace("..", "_");
    }
    // Remove any lingering path separators (already mapped to _ but be safe)
    trimmed = trimmed.replace(['/', '\\'], "_");
    // Collapse consecutive underscores
    while trimmed.contains("__") {
        trimmed = trimmed.replace("__", "_");
    }
    trimmed = trimmed
        .trim_matches(|c| c == '.' || c == '_' || c == ' ')
        .to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return "audio_track".to_string();
    }
    // Windows reserved device names
    let lower = trimmed.to_ascii_lowercase();
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if RESERVED.contains(&lower.as_str()) {
        return format!("{}_{}", trimmed, "track");
    }
    if trimmed.len() > 200 {
        trimmed.truncate(200);
        trimmed = trimmed.trim_end_matches(['.', '_', ' ']).to_string();
        if trimmed.is_empty() {
            return "audio_track".to_string();
        }
    }
    trimmed
}

/// Try the Opus/WebM fallback (Symphonia probe with EBML sniffing) for files
/// lofty/rodio cannot handle — e.g. Opus-in-WebM mislabeled as `.m4a`
/// (`https://d.uguu.se/jXSTGTDj.m4a`: EBML `1A 45 DF A3`, `google/video-file`,
/// lofty guesses `Mpeg` and fails, rodio reports "format not recognized").
/// Returns `Some(duration)` when the Symphonia probe yields a usable duration
/// within the ±5s expected-duration tolerance.
fn try_opus_fallback(path: &Path, expected_duration_secs: Option<u32>) -> Option<u32> {
    let meta = super::opus::extract_opus_metadata(path).ok()?;
    if meta.duration_secs == 0 {
        return None;
    }
    if let Some(expected) = expected_duration_secs {
        if expected > 0 && (meta.duration_secs as i64 - expected as i64).abs() > 5 {
            return None;
        }
    }
    Some(meta.duration_secs)
}

/// Check whether a file starts with the EBML header (`1A 45 DF A3`)
/// identifying a WebM/Matroska container (usually Opus audio from YouTube).
fn is_ebml_container(path: &Path) -> bool {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut header = [0u8; 4];
    file.read_exact(&mut header).is_ok() && header == [0x1a, 0x45, 0xdf, 0xa3]
}

/// Validate downloaded audio file integrity using lofty.
/// Checks that the file is non-empty, contains valid audio headers/properties,
/// and that decoded duration matches expected duration within ±5s tolerance (if expected is known).
pub fn validate_audio_file(
    path: &Path,
    expected_duration_secs: Option<u32>,
    ext: &str,
    format: AudioFormat,
) -> Result<u32, String> {
    if !path.exists() {
        return Err(format!("Staging file does not exist: {}", path.display()));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read metadata for {}: {e}", path.display()))?;
    let file_size = metadata.len();
    if file_size == 0 {
        return Err("Staging file is empty (0 bytes)".to_string());
    }

    // Fast path: EBML/WebM container (Opus audio mislabeled as .m4a/.mp3, …).
    // lofty misdetects these bytes as `Mpeg` and rodio cannot decode Opus at
    // all, so consult the Symphonia Opus probe first — mirroring
    // `player.rs::create_decoder` + `metadata.rs::extract_with_size`.
    if is_ebml_container(path) {
        if let Some(dur) = try_opus_fallback(path, expected_duration_secs) {
            return Ok(dur);
        }
    }

    let mut probe =
        Probe::open(path).map_err(|e| format!("Failed to open file for lofty probe: {e}"))?;

    if probe.file_type().is_none() {
        probe = probe
            .guess_file_type()
            .map_err(|e| format!("Failed to guess audio file type: {e}"))?;
    }

    if probe.file_type().is_none() {
        if let Some(ft) = FileType::from_ext(ext) {
            probe = probe.set_file_type(ft);
        } else if let Some(ft) = FileType::from_ext(format.extension()) {
            probe = probe.set_file_type(ft);
        }
    }

    let tagged_file = match probe.read() {
        Ok(tf) => tf,
        Err(e) => {
            // lofty cannot parse WebM/Opus (no WebM FileType; EBML bytes guess
            // as Mpeg) — fall back to the Symphonia Opus probe before failing.
            if let Some(dur) = try_opus_fallback(path, expected_duration_secs) {
                return Ok(dur);
            }
            return Err(format!("Corrupt or unreadable audio headers: {e}"));
        }
    };

    let duration_secs = tagged_file.properties().duration().as_secs() as u32;
    if duration_secs == 0 {
        if expected_duration_secs.is_some() {
            return Err(
                "Decoded duration is 0s — file has unreadable atom index tables or is truncated"
                    .to_string(),
            );
        }
        // No expected duration — validate file size and audio properties
        let props = tagged_file.properties();
        let sample_rate = props.sample_rate().unwrap_or(0);
        let channels = props.channels().unwrap_or(0);
        if file_size <= 10_240 || sample_rate == 0 || channels == 0 {
            return Err(format!(
                "Decoded duration is 0s — file has unreadable atom index tables or is truncated (size={} bytes, sample_rate={}, channels={})",
                file_size, sample_rate, channels
            ));
        }
        return Err(
            "Decoded duration is 0s — file has unreadable atom index tables or is truncated"
                .to_string(),
        );
    }

    if let Some(expected) = expected_duration_secs {
        if expected > 0 {
            let diff = (duration_secs as i64 - expected as i64).abs();
            if diff > 5 {
                return Err(format!(
                    "Decoded duration ({}s) differs by > 5s from expected duration ({}s, diff={}s)",
                    duration_secs, expected, diff
                ));
            }
        }
    }

    // Dry-run decoder probe using rodio::Decoder with 64 KB BufReader
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open file for decoder probe: {e}"))?;
    let probe_res = if !ext.is_empty() {
        if let Ok(cloned_file) = file.try_clone() {
            let reader = BufReader::with_capacity(64 * 1024, cloned_file);
            match rodio::Decoder::builder()
                .with_data(reader)
                .with_hint(ext)
                .build()
            {
                Ok(decoder) => Ok(decoder),
                Err(_) => {
                    let mut f = file;
                    let _ = f.seek(SeekFrom::Start(0));
                    rodio::Decoder::new(BufReader::with_capacity(64 * 1024, f))
                }
            }
        } else {
            let mut f = file;
            let _ = f.seek(SeekFrom::Start(0));
            rodio::Decoder::new(BufReader::with_capacity(64 * 1024, f))
        }
    } else {
        let mut f = file;
        let _ = f.seek(SeekFrom::Start(0));
        rodio::Decoder::new(BufReader::with_capacity(64 * 1024, f))
    };

    if let Err(e) = probe_res {
        // rodio has no Opus decoder (WebM/Opus always fails here) — accept the
        // file when the native OpusSource probe decodes its metadata.
        if let Some(dur) = try_opus_fallback(path, expected_duration_secs) {
            return Ok(dur);
        }
        return Err(format!("Decoder probe failed for {}: {e}", path.display()));
    }

    Ok(duration_secs)
}

/// Asynchronously validate downloaded audio file integrity offloaded to tokio::task::spawn_blocking.
///
/// Prevents synchronous file I/O, lofty probe, and rodio decoder probing from blocking
/// Tokio executor worker threads.
pub async fn validate_audio_file_async(
    path: &Path,
    expected_duration_secs: Option<u32>,
    ext: &str,
    format: AudioFormat,
) -> Result<u32, String> {
    let path_buf = path.to_path_buf();
    let ext_owned = ext.to_string();
    tokio::task::spawn_blocking(move || {
        validate_audio_file(&path_buf, expected_duration_secs, &ext_owned, format)
    })
    .await
    .map_err(|e| format!("Task join error for validate_audio_file: {e}"))?
}

impl Downloader {
    /// Create a new downloader that writes files into `output_dir`.
    pub fn new(output_dir: PathBuf) -> Self {
        // Ensure staging directory exists
        let tmp_dir = output_dir.join(".tmp");
        let _ = std::fs::create_dir_all(&tmp_dir);

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
        let host = req.stream_url.split('/').nth(2).unwrap_or("unknown");
        let has_headers = req.headers.is_some();
        let header_keys = req
            .headers
            .as_ref()
            .map(|h| h.keys().cloned().collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        info!(
            url = %req.stream_url,
            title = %req.title,
            platform = %req.platform,
            ext = %req.ext,
            total_bytes = ?req.total_bytes,
            expected_duration = ?req.expected_duration_secs,
            host = %host,
            has_headers = %has_headers,
            headers = %header_keys,
            "Starting download"
        );
        debug!(url = %req.stream_url, headers = ?req.headers, "Download request headers");

        if !req.stream_url.starts_with("https://") && !req.stream_url.starts_with("http://") {
            warn!(url = %req.stream_url, "Rejected non-http(s) URL");
            return Err(DownloaderError::InvalidUrl(req.stream_url));
        }

        // Clean up completed/failed/cancelled records to prevent memory growth
        self.cleanup().await;

        let id = Uuid::new_v4();
        let fallback_ext = req.format.extension().to_string();
        let ext = if req.ext.is_empty() {
            sanitize_ext(&fallback_ext, &fallback_ext)
        } else {
            sanitize_ext(&req.ext, &fallback_ext)
        };
        let mut filename = format!("{}.{}", sanitize_filename(&req.title), ext);
        let mut path = self.output_dir.join(&filename);
        // Deduplicate with short UUID suffix if file already exists; keeps
        // sidecar `<audio>.jpg` working via with_extension("jpg")
        if path.exists() {
            let stem = sanitize_filename(&req.title);
            let short = id.to_string().chars().take(8).collect::<String>();
            filename = format!("{}_{}.{}", stem, short, ext);
            path = self.output_dir.join(&filename);
        }

        let tmp_dir = self.output_dir.join(".tmp");
        let staging_path = tmp_dir.join(format!("{}.part", id));
        let _ = tokio::fs::create_dir_all(&tmp_dir).await;

        let mut progress =
            DownloadProgress::with_id(id, req.stream_url.clone(), req.title.clone(), req.format);
        progress.platform = req.platform.clone();
        progress.total_bytes = req.total_bytes;
        progress.expected_duration_secs = req.expected_duration_secs;
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
                    staging_path,
                    thumbnail: req.thumbnail,
                    headers: req.headers,
                    expected_duration_secs: req.expected_duration_secs,
                    total_bytes: req.total_bytes,
                    format: req.format,
                    ext,
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
                let url_host = job.stream_url.split('/').nth(2).unwrap_or("unknown");
                error!(download_id = %id, url_host = %url_host, url = %job.stream_url, start_byte = start_byte, error = %e, "Download failed — cleaning staging file and marking failed");
                warn!(download_id = %id, error = %e, "DIAGNOSTIC download_failed id={} host={} url={} error={}", id, url_host, job.stream_url, e);

                // Clean up staging file and output file on unrecoverable failure
                let _ = tokio::fs::remove_file(&job.staging_path).await;
                let _ = tokio::fs::remove_file(&job.output_path).await;
                let _ = tokio::fs::remove_file(job.output_path.with_extension("jpg")).await;

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

    /// Stream `job.stream_url` to `job.staging_path`, verifying integrity and atomically
    /// moving to `job.output_path`.
    async fn run_stream(
        id: Uuid,
        job: &DownloadJob,
        initial_start_byte: u64,
        active: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    ) -> Result<(), DownloaderError> {
        let host = job.stream_url.split('/').nth(2).unwrap_or("unknown");
        let url_snip = if job.stream_url.len() > 160 {
            format!("{}…", &job.stream_url[..160])
        } else {
            job.stream_url.clone()
        };

        // Ensure staging and output directories exist
        if let Some(parent) = job.staging_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(DownloaderError::IoError)?;
        }
        if let Some(parent) = job.output_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(DownloaderError::IoError)?;
        }

        // Build HTTP client: use supplied UA if present, otherwise sane default.
        let ua = job
            .headers
            .as_ref()
            .and_then(|h| h.get("User-Agent").or_else(|| h.get("user-agent")).cloned())
            .unwrap_or_else(|| "Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UD1A.230803.041) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36".to_string());

        debug!(download_id = %id, host = %host, ua = %ua, "Building HTTP client for stream");
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .user_agent(ua.clone())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| {
                error!(download_id = %id, error = %e, "Failed to build HTTP client");
                DownloaderError::HttpError(format!("failed to build HTTP client: {e}"))
            })?;

        const MAX_CONSECUTIVE_ERRORS: usize = 5;

        // A resolved googlevideo URL can still carry a `range=start-end` window.
        // Request the full object so the byte accounting below is judged against
        // the real file size instead of a capped window.
        let request_url = strip_response_range_params(&job.stream_url);

        // Why the stream loop stopped. Surfaced in failure messages so a
        // truncated download is explainable from the app UI alone (Android
        // release builds do not reach logcat).
        let mut end_reason = "unknown";

        let mut consecutive_errors: usize = 0;
        let mut total_bytes: Option<u64> = job
            .total_bytes
            .or_else(|| extract_url_param_u64(&request_url, "clen"));
        let expected_duration_secs: Option<u32> = job
            .expected_duration_secs
            .or_else(|| extract_url_param_f64(&job.stream_url, "dur").map(|d| d.round() as u32));
        let mut current_downloaded: u64 = initial_start_byte;

        // Check if staging file already exists on disk and has bytes for resuming
        if initial_start_byte > 0 && job.staging_path.exists() {
            if let Ok(meta) = tokio::fs::metadata(&job.staging_path).await {
                current_downloaded = meta.len();
            }
        }

        let overall_start = Instant::now();
        let mut chunk_iteration: usize = 0;

        loop {
            chunk_iteration += 1;

            // Check if full stream byte length is already reached
            if let Some(total) = total_bytes {
                if total > 0 && current_downloaded >= total {
                    info!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        total = total,
                        "Full stream byte length reached (downloaded >= total) — proceeding to validation"
                    );
                    end_reason = "all-advertised-bytes-received";
                    break;
                }
            }

            if consecutive_errors > 0 {
                let backoff_ms = 500 * (1 << (consecutive_errors - 1).min(5));
                info!(
                    download_id = %id,
                    consecutive_errors = consecutive_errors,
                    max_consecutive_errors = MAX_CONSECUTIVE_ERRORS,
                    backoff_ms = backoff_ms,
                    downloaded = current_downloaded,
                    "Stream retry after error with exponential backoff"
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }

            // Inject headers that googlevideo validates: Referer/Origin/Accept.
            let mut req = inject_stream_headers(client.get(&request_url), job.headers.as_ref());

            if current_downloaded > 0 {
                req = req.header("Range", format!("bytes={}-", current_downloaded));
            }

            info!(
                download_id = %id,
                host = %host,
                chunk_iteration = chunk_iteration,
                start_byte = current_downloaded,
                total_bytes = ?total_bytes,
                "Sending GET for stream range"
            );

            let send_res = tokio::time::timeout(Duration::from_secs(30), req.send()).await;
            let mut res = match send_res {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    let msg = format!("request failed [{host}] start_byte={current_downloaded}: {e} (url={url_snip})");
                    warn!(download_id = %id, host = %host, error = %e, consecutive_errors = consecutive_errors, "Request send error");
                    consecutive_errors += 1;
                    if consecutive_errors < MAX_CONSECUTIVE_ERRORS {
                        continue;
                    }
                    return Err(DownloaderError::HttpError(msg));
                }
                Err(_) => {
                    let msg = format!("request timed out after 30s [{host}] start_byte={current_downloaded} url={url_snip}");
                    warn!(download_id = %id, host = %host, consecutive_errors = consecutive_errors, "Request send timed out");
                    consecutive_errors += 1;
                    if consecutive_errors < MAX_CONSECUTIVE_ERRORS {
                        continue;
                    }
                    return Err(DownloaderError::HttpError(msg));
                }
            };

            let status = res.status();
            let resuming = current_downloaded > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;

            if current_downloaded > 0 && status == reqwest::StatusCode::OK {
                // Server ignored Range — must truncate and restart from 0
                warn!(
                    download_id = %id,
                    start_byte = current_downloaded,
                    "Range request got 200 not 206 — resetting downloaded to 0 and truncating staging file"
                );
                current_downloaded = 0;
                {
                    let mut guard = active.write().await;
                    if let Some(state) = guard.get_mut(&id) {
                        state.downloaded_bytes = 0;
                        state.progress = 0.0;
                        state.updated_at = Utc::now();
                    }
                }
            }

            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                // A 416 normally carries `Content-Range: bytes */TOTAL`, which is
                // the authoritative object size. Treat it as "finished" ONLY when
                // our byte count actually reached that total: a decodable
                // container header is NOT evidence that the audio bytes arrived.
                if let Some(cr) = res
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                {
                    if let Some(server_total) = parse_content_range_total(cr) {
                        total_bytes =
                            Some(total_bytes.map_or(server_total, |cur| cur.max(server_total)));
                    }
                }
                let reached_total = total_bytes
                    .is_some_and(|t| t > 0 && current_downloaded + COMPLETE_TOLERANCE_BYTES >= t);
                if reached_total {
                    info!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        total = ?total_bytes,
                        "416 Range Not Satisfiable but all advertised bytes are present — stream complete"
                    );
                    end_reason = "416-all-advertised-bytes-present";
                    break;
                }
                warn!(
                    download_id = %id,
                    start_byte = current_downloaded,
                    total = ?total_bytes,
                    "416 Range Not Satisfiable with bytes still missing — discarding staging file and restarting from 0"
                );
                end_reason = "416-bytes-missing-restart";
                let bytes_before_reset = current_downloaded;
                current_downloaded = 0;
                let _ = tokio::fs::remove_file(&job.staging_path).await;
                consecutive_errors += 1;
                if consecutive_errors < MAX_CONSECUTIVE_ERRORS {
                    continue;
                }
                return Err(DownloaderError::HttpError(format!(
                    "HTTP 416 Range Not Satisfiable [{host}] — received {bytes_before_reset} of {:?} bytes after {MAX_CONSECUTIVE_ERRORS} attempts",
                    total_bytes
                )));
            }

            let ct = res
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .to_string();
            let cl_hdr = res
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .to_string();

            let response_total: Option<u64> = res
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.rsplit('/').next())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .or_else(|| {
                    if resuming {
                        res.content_length().map(|cl| cl + current_downloaded)
                    } else {
                        res.content_length()
                    }
                });

            // Update total_bytes, but NEVER shrink a larger known total with a partial chunk length
            if let Some(resp_tot) = response_total {
                total_bytes = Some(total_bytes.map_or(resp_tot, |cur| cur.max(resp_tot)));
            }

            info!(
                download_id = %id,
                host = %host,
                status = %status,
                content_type = %ct,
                content_length = %cl_hdr,
                total = ?total_bytes,
                resuming = resuming,
                "Received response headers"
            );

            if !status.is_success() && status.as_u16() != 206 {
                let body_snip = match tokio::time::timeout(Duration::from_secs(5), res.text()).await
                {
                    Ok(Ok(t)) => {
                        let s = t.chars().take(500).collect::<String>().replace('\n', " ");
                        if s.is_empty() {
                            "(empty body)".to_string()
                        } else {
                            s
                        }
                    }
                    Ok(Err(e)) => format!("(failed to read body: {e})"),
                    Err(_) => "(body read timed out)".to_string(),
                };
                let hint = match status.as_u16() {
                    403 => " — 403 Forbidden: googlevideo rejected UA/Referer/Origin/PO-token or URL expired [rr1---sn-gwpa-cived]",
                    404 => " — 404: URL expired or invalid (re-resolve the video)",
                    416 => " — 416 Range Not Satisfiable: resume offset beyond file size",
                    429 => " — 429 Too Many Requests: rate-limited, retry later",
                    500..=599 => " — server error, retry later",
                    _ => "",
                };
                let msg = format!("HTTP {status} [{host}]{hint} body: {body_snip} (url={url_snip}, start_byte={current_downloaded}, ct={ct})");
                error!(download_id = %id, host = %host, status = %status, body = %body_snip, "HTTP error response");
                warn!(download_id = %id, "DIAGNOSTIC download_http_error id={} host={} status={} ct={} hint={} body={} url={}", id, host, status, ct, hint, body_snip, url_snip);

                if status.as_u16() == 403 || status.as_u16() == 404 {
                    return Err(DownloaderError::HttpError(msg));
                }

                consecutive_errors += 1;
                if consecutive_errors < MAX_CONSECUTIVE_ERRORS {
                    continue;
                }
                return Err(DownloaderError::HttpError(msg));
            }

            // Sanity-check the advertised size against the known track duration.
            // A 4-minute song that "totals" 1 MB is a truncated response window
            // (e.g. a URL still carrying a `range=` cap), not a 1-minute track —
            // re-ask the server for the real object size so completeness is
            // judged against the truth.
            if let (Some(total), Some(dur)) = (total_bytes, expected_duration_secs) {
                let min_plausible = (dur as u64).saturating_mul(MIN_PLAUSIBLE_BYTES_PER_SEC);
                if dur >= MIN_DURATION_FOR_BITRATE_CHECK && total < min_plausible {
                    warn!(
                        download_id = %id,
                        advertised_total = total,
                        expected_duration = dur,
                        min_plausible = min_plausible,
                        "Advertised object size is implausibly small for the track duration — probing server for the real size"
                    );
                    if let Some(probed) =
                        probe_total_bytes(&client, &request_url, job.headers.as_ref()).await
                    {
                        info!(download_id = %id, probed_total = probed, "Probed authoritative object size");
                        total_bytes = Some(total_bytes.map_or(probed, |cur| cur.max(probed)));
                    }
                }
            }

            {
                let mut guard = active.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    state.status = DownloadStatus::Downloading;
                    if total_bytes.is_some() {
                        state.total_bytes = total_bytes;
                    }
                    state.updated_at = Utc::now();
                }
            }

            let mut file = if current_downloaded > 0 {
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&job.staging_path)
                    .await
                    .map_err(|e| {
                        error!(download_id = %id, path = ?job.staging_path, error = %e, "Failed to open staging file for append (resume)");
                        DownloaderError::IoError(e)
                    })?
            } else {
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&job.staging_path)
                    .await
                    .map_err(|e| {
                        error!(download_id = %id, path = ?job.staging_path, error = %e, "Failed to create/truncate staging file");
                        DownloaderError::IoError(e)
                    })?
            };

            let stream_start_instant = Instant::now();
            let mut stream_interrupted = false;
            let mut bytes_in_this_request: u64 = 0;

            loop {
                let chunk_opt =
                    match tokio::time::timeout(Duration::from_secs(30), res.chunk()).await {
                        Ok(Ok(c)) => c,
                        Ok(Err(e)) => {
                            warn!(
                                download_id = %id,
                                host = %host,
                                error = %e,
                                downloaded = current_downloaded,
                                total = ?total_bytes,
                                "Stream read error — will trigger range resume"
                            );
                            stream_interrupted = true;
                            break;
                        }
                        Err(_) => {
                            let elapsed = stream_start_instant.elapsed().as_secs();
                            warn!(
                                download_id = %id,
                                host = %host,
                                downloaded = current_downloaded,
                                total = ?total_bytes,
                                elapsed = elapsed,
                                "Stream stalled (30s timeout) — will trigger range resume"
                            );
                            stream_interrupted = true;
                            break;
                        }
                    };

                let Some(chunk) = chunk_opt else {
                    // Stream reached EOF for this HTTP response
                    break;
                };

                if chunk.is_empty() {
                    continue;
                }

                if let Err(e) = file.write_all(&chunk).await {
                    error!(download_id = %id, path = ?job.staging_path, error = %e, chunk_len = chunk.len(), "Staging file write failed");
                    return Err(DownloaderError::DownloadFailed(format!(
                        "failed to write {} bytes to staging file {:?}: {e}",
                        chunk.len(),
                        job.staging_path
                    )));
                }

                bytes_in_this_request += chunk.len() as u64;
                current_downloaded += chunk.len() as u64;

                let elapsed_total = overall_start.elapsed().as_secs_f64();
                let speed = if elapsed_total > 0.0 {
                    (current_downloaded as f64 / elapsed_total) as u64
                } else {
                    0
                };

                let mut guard = active.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    state.downloaded_bytes = current_downloaded;
                    if let Some(t) = total_bytes {
                        if t > 0 {
                            state.progress = ((current_downloaded as f32) / (t as f32)).min(1.0);
                            let remaining = t.saturating_sub(current_downloaded);
                            state.eta_secs = (remaining as u32).checked_div(speed as u32);
                        }
                    }
                    state.speed_bps = speed;
                    state.updated_at = Utc::now();
                }
            }

            let _ = file.flush().await;

            if bytes_in_this_request > 0 {
                consecutive_errors = 0; // Successful progress made!
            } else if stream_interrupted {
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    return Err(DownloaderError::HttpError(format!(
                        "Stream interrupted and max consecutive errors ({MAX_CONSECUTIVE_ERRORS}) reached [{host}] ({current_downloaded}/{:?} bytes)",
                        total_bytes
                    )));
                }
                continue;
            } else if total_bytes.is_some_and(|t| t > current_downloaded) {
                // The server closed the connection cleanly but still owes us
                // bytes. That is NOT the end of the stream — resume via Range
                // until the retry budget is spent, then fail honestly.
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    return Err(DownloaderError::DownloadFailed(format!(
                        "Incomplete download: server ended the stream at {current_downloaded} of {} bytes after {MAX_CONSECUTIVE_ERRORS} resume attempts (end_reason={end_reason}, host={host}) — file not saved",
                        total_bytes.unwrap_or(0)
                    )));
                }
                warn!(
                    download_id = %id,
                    downloaded = current_downloaded,
                    total = ?total_bytes,
                    consecutive_errors = consecutive_errors,
                    "Stream ended early with bytes still outstanding — resuming via Range"
                );
                continue;
            }

            // Check if full stream has been reached:
            if let Some(total) = total_bytes {
                if total > 0 && current_downloaded >= total {
                    info!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        total = total,
                        "Stream finished with all expected bytes ({current_downloaded}/{total}) — proceeding to validation"
                    );
                    end_reason = "all-advertised-bytes-received";
                    break;
                } else {
                    // Googlevideo chunk cutoff — recursively continue range request for the next chunk
                    info!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        total = total,
                        "Chunk completed; continuing range request for remaining bytes ({current_downloaded}/{total})"
                    );
                    continue;
                }
            } else {
                // total_bytes is unknown
                if bytes_in_this_request > 0 {
                    // Try to request next range to see if more bytes exist
                    info!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        "Chunk completed with unknown total_bytes; probing next range for remaining bytes"
                    );
                    continue;
                } else {
                    // Zero bytes in this request and clean EOF -> stream is finished!
                    info!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        "Stream reached clean EOF with no additional bytes — proceeding to validation"
                    );
                    end_reason = "clean-eof-size-unknown";
                    break;
                }
            }
        }

        // ---- Completeness gate -------------------------------------------------
        // A decodable container header is NOT proof that the audio bytes
        // arrived: YouTube's m4a carries the full duration in its front `moov`
        // atom, so a half-downloaded file still "validates" and used to be
        // renamed + reported as completed (and played as silence afterwards).
        // Decide completeness from the byte count, never from metadata alone.
        let staged_bytes = tokio::fs::metadata(&job.staging_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let short_by_advertised =
            total_bytes.is_some_and(|t| t > 0 && staged_bytes + COMPLETE_TOLERANCE_BYTES < t);
        let short_by_duration = match (total_bytes, expected_duration_secs) {
            // No size from resolver or server: fall back to a conservative
            // bytes-per-second floor so "it just stopped" is still caught.
            (None, Some(dur)) if dur >= MIN_DURATION_FOR_BITRATE_CHECK => {
                staged_bytes < (dur as u64).saturating_mul(MIN_BYTES_PER_SEC_FALLBACK)
            }
            _ => false,
        };
        if short_by_advertised || short_by_duration {
            let msg = format!(
                "Incomplete download: received {staged_bytes} bytes of {} expected \
                 (track duration {expected_duration_secs:?}s, end_reason={end_reason}, host={host}). \
                 The file was NOT saved — retrying will resume from byte {staged_bytes}.",
                total_bytes.unwrap_or(0)
            );
            error!(
                download_id = %id,
                downloaded = staged_bytes,
                total = ?total_bytes,
                expected_duration = ?expected_duration_secs,
                end_reason = %end_reason,
                "Refusing to mark a truncated download as complete"
            );
            cleanup_staging_file(&job.staging_path).await;
            return Err(DownloaderError::DownloadFailed(msg));
        }
        info!(
            download_id = %id,
            downloaded = staged_bytes,
            total = ?total_bytes,
            end_reason = %end_reason,
            "Byte accounting complete — proceeding to audio validation"
        );

        // Post-Download Audio Stream Integrity & Duration Validation
        info!(
            download_id = %id,
            staging_path = %job.staging_path.display(),
            expected_duration = ?expected_duration_secs,
            downloaded = staged_bytes,
            end_reason = %end_reason,
            "Validating audio stream integrity with lofty"
        );

        match validate_audio_file_async(
            &job.staging_path,
            expected_duration_secs,
            &job.ext,
            job.format,
        )
        .await
        {
            Ok(decoded_duration) => {
                info!(
                    download_id = %id,
                    decoded_duration = decoded_duration,
                    "Audio stream validation succeeded"
                );
            }
            Err(val_err) => {
                warn!(
                    download_id = %id,
                    error = %val_err,
                    "Audio stream validation failed on staging file"
                );
                cleanup_staging_file(&job.staging_path).await;
                return Err(DownloaderError::DownloadFailed(format!(
                    "Audio stream integrity validation failed: {val_err}"
                )));
            }
        }

        // All validation passed! Now perform atomic rename to destination
        info!(
            download_id = %id,
            staging = %job.staging_path.display(),
            destination = %job.output_path.display(),
            "Atomically moving verified staging file to final destination"
        );

        if let Err(e) = tokio::fs::rename(&job.staging_path, &job.output_path).await {
            warn!(
                download_id = %id,
                error = %e,
                "tokio::fs::rename failed, falling back to copy + remove"
            );
            tokio::fs::copy(&job.staging_path, &job.output_path)
                .await
                .map_err(DownloaderError::IoError)?;
            let _ = tokio::fs::remove_file(&job.staging_path).await;
        }

        // Save thumbnail sidecar if requested
        if let Some(thumb) = &job.thumbnail {
            Self::save_thumbnail(&client, thumb, &job.output_path).await;
        }

        // MediaStore Publishing on Android
        #[cfg(target_os = "android")]
        {
            let should_publish = std::panic::catch_unwind(|| {
                crate::domain::models::Settings::load()
                    .map(|s| s.downloads.use_system_downloads)
                    .unwrap_or(true)
            })
            .unwrap_or(true);
            info!(download_id = %id, should_publish = should_publish, src = %job.output_path.display(), "MediaStore publish check (use_system_downloads, default true)");
            if !should_publish {
                info!(download_id = %id, "Skipping MediaStore publish per use_system_downloads=false");
            } else {
                let public = crate::infrastructure::media::android_downloads::publish_to_downloads(
                    &job.output_path,
                );
                if let Some(pub_path) = public {
                    let mut guard = active.write().await;
                    if let Some(state) = guard.get_mut(&id) {
                        // Keep internal path for library scan dedup, but surface public
                        // path so `download:completed` shows the Files-visible location.
                        state.output_path = Some(pub_path.clone());
                    }
                    info!(download_id = %id, public = %pub_path, internal = %job.output_path.display(), "Published download to Download/Auralis");
                } else {
                    warn!(download_id = %id, src = %job.output_path.display(), "MediaStore publish returned None — keeping internal path");
                }
            }
        }

        // Mark completed in active downloads
        {
            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.complete(job.output_path.to_string_lossy().to_string());
            }
        }

        info!(
            download_id = %id,
            path = %job.output_path.display(),
            "Download complete and verified"
        );

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
    /// staging file to the last fully-written byte.
    pub async fn pause(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Pausing download");

        if let Some(handle) = self.tasks.write().await.remove(&id) {
            handle.abort();
        }

        let staging_path = {
            let jobs = self.jobs.read().await;
            jobs.get(&id).map(|j| j.staging_path.clone())
        };

        let downloaded = {
            let downloads = self.active_downloads.read().await;
            downloads.get(&id).map(|s| s.downloaded_bytes)
        };

        if let (Some(path), Some(bytes)) = (staging_path, downloaded) {
            if let Ok(f) = tokio::fs::OpenOptions::new().write(true).open(&path).await {
                let _ = f.set_len(bytes).await;
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

    /// Cancel a download, killing its task and removing any staging and partial files.
    pub async fn cancel(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Cancelling download");

        if let Some(handle) = self.tasks.write().await.remove(&id) {
            handle.abort();
        }

        let paths = {
            let jobs = self.jobs.read().await;
            jobs.get(&id)
                .map(|j| (j.staging_path.clone(), j.output_path.clone()))
        };

        if let Some((staging, output)) = paths {
            let _ = tokio::fs::remove_file(&staging).await;
            let _ = tokio::fs::remove_file(&output).await;
            let _ = tokio::fs::remove_file(output.with_extension("jpg")).await;
        }

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            state.cancel();
        }

        Ok(())
    }

    /// Prune finished, failed, and cancelled download records from `jobs` and `active_downloads`.
    /// Removes records updated more than `max_age` ago (default 10 minutes) and ensures
    /// at most `max_retained` (default 50) finished/terminal records are kept.
    pub async fn cleanup(&self) {
        self.prune_finished(Duration::from_secs(10 * 60), 50).await;
    }

    /// Prune finished download records with custom age and retention limits.
    pub async fn prune_finished(&self, max_age: Duration, max_retained: usize) {
        let now = Utc::now();
        let max_age_chrono =
            chrono::Duration::from_std(max_age).unwrap_or_else(|_| chrono::Duration::minutes(10));

        let mut to_remove = Vec::new();

        {
            let downloads = self.active_downloads.read().await;
            let mut terminal_records: Vec<(Uuid, chrono::DateTime<Utc>)> = downloads
                .iter()
                .filter(|(_, p)| {
                    matches!(
                        p.status,
                        DownloadStatus::Completed
                            | DownloadStatus::Failed
                            | DownloadStatus::Cancelled
                    )
                })
                .map(|(&id, p)| (id, p.updated_at))
                .collect();

            // 1. Records older than max_age
            for (id, updated_at) in &terminal_records {
                if now.signed_duration_since(*updated_at) > max_age_chrono {
                    to_remove.push(*id);
                }
            }

            // 2. If retained terminal records exceed max_retained, remove oldest
            terminal_records.retain(|(id, _)| !to_remove.contains(id));
            if terminal_records.len() > max_retained {
                // Sort descending by updated_at (newest first)
                terminal_records.sort_by_key(|a| std::cmp::Reverse(a.1));
                for (id, _) in terminal_records.iter().skip(max_retained) {
                    to_remove.push(*id);
                }
            }
        }

        if !to_remove.is_empty() {
            debug!(count = to_remove.len(), "Pruning finished download records");
            let mut downloads = self.active_downloads.write().await;
            let mut jobs = self.jobs.write().await;
            let mut tasks = self.tasks.write().await;

            for id in to_remove {
                downloads.remove(&id);
                jobs.remove(&id);
                tasks.remove(&id);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_downloader_cleanup_and_prune() {
        let dir = std::env::temp_dir().join(format!("auralis_dl_test_{}", Uuid::new_v4()));
        let downloader = Downloader::new(dir.clone());

        let id1 = Uuid::new_v4();
        let mut p1 = DownloadProgress::new(
            "https://example.com/1".into(),
            "Track 1".into(),
            AudioFormat::Mp3,
        );
        p1.status = DownloadStatus::Completed;
        p1.updated_at = Utc::now() - chrono::Duration::minutes(15); // > 10 min old

        let id2 = Uuid::new_v4();
        let mut p2 = DownloadProgress::new(
            "https://example.com/2".into(),
            "Track 2".into(),
            AudioFormat::Mp3,
        );
        p2.status = DownloadStatus::Failed;
        p2.updated_at = Utc::now() - chrono::Duration::seconds(30); // recent

        let id3 = Uuid::new_v4();
        let mut p3 = DownloadProgress::new(
            "https://example.com/3".into(),
            "Track 3".into(),
            AudioFormat::Mp3,
        );
        p3.status = DownloadStatus::Downloading; // in progress
        p3.updated_at = Utc::now() - chrono::Duration::minutes(20);

        {
            let mut active = downloader.active_downloads.write().await;
            active.insert(id1, p1);
            active.insert(id2, p2);
            active.insert(id3, p3);
        }

        // Run default cleanup (10 min threshold, 50 cap)
        downloader.cleanup().await;

        {
            let active = downloader.active_downloads.read().await;
            assert!(
                !active.contains_key(&id1),
                "Old completed download should be pruned"
            );
            assert!(
                active.contains_key(&id2),
                "Recent failed download should be retained"
            );
            assert!(
                active.contains_key(&id3),
                "Active downloading track should not be pruned"
            );
        }

        // Test max_retained limit
        for i in 0..10 {
            let id = Uuid::new_v4();
            let mut p = DownloadProgress::new(
                format!("https://example.com/{i}"),
                format!("Track {i}"),
                AudioFormat::Mp3,
            );
            p.status = DownloadStatus::Completed;
            p.updated_at = Utc::now() - chrono::Duration::seconds(i as i64);
            downloader.active_downloads.write().await.insert(id, p);
        }

        // Prune with max_retained = 3
        downloader
            .prune_finished(Duration::from_secs(3600), 3)
            .await;

        {
            let active = downloader.active_downloads.read().await;
            let completed_count = active
                .values()
                .filter(|p| p.status == DownloadStatus::Completed)
                .count();
            assert_eq!(
                completed_count, 3,
                "Should retain exactly 3 completed records"
            );
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("Valid Name"), "Valid Name");
        assert_eq!(sanitize_filename("Slash/In/Name"), "Slash_In_Name");
        assert_eq!(sanitize_filename("Back\\Slash"), "Back_Slash");
        assert_eq!(sanitize_filename("../../etc/passwd"), "etc_passwd");
        assert_eq!(sanitize_filename("AUX"), "AUX_track");
        assert_eq!(sanitize_filename("COM1"), "COM1_track");
    }

    #[test]
    fn test_sanitize_ext() {
        assert_eq!(sanitize_ext("mp3", "mp3"), "mp3");
        assert_eq!(sanitize_ext("m4a", "m4a"), "m4a");
        assert_eq!(sanitize_ext("..exe", "mp3"), "mp3");
        assert_eq!(sanitize_ext("unknown", "flac"), "flac");
    }

    #[test]
    fn test_validate_audio_file_nonexistent() {
        let path = PathBuf::from("/nonexistent/path/audio.mp3");
        let res = validate_audio_file(&path, Some(100), "mp3", AudioFormat::Mp3);
        assert!(res.is_err());
    }

    #[test]
    fn test_validate_audio_file_empty() {
        let dir = std::env::temp_dir().join(format!("auralis_test_{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let empty_path = dir.join("empty.mp3");
        std::fs::write(&empty_path, b"").unwrap();

        let res = validate_audio_file(&empty_path, None, "mp3", AudioFormat::Mp3);
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_validate_audio_file_valid_wav() {
        let dir = std::env::temp_dir().join(format!("auralis_test_{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("test.part");

        // 1s 8000Hz 8-bit mono WAV = 8044 bytes
        let sample_rate: u32 = 8000;
        let num_samples: u32 = 8000;
        let mut data = Vec::with_capacity(44 + num_samples as usize);
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + num_samples).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes()); // Mono
        data.extend_from_slice(&sample_rate.to_le_bytes());
        data.extend_from_slice(&sample_rate.to_le_bytes()); // Byte rate
        data.extend_from_slice(&1u16.to_le_bytes()); // Block align
        data.extend_from_slice(&8u16.to_le_bytes()); // Bits per sample
        data.extend_from_slice(b"data");
        data.extend_from_slice(&num_samples.to_le_bytes());
        data.resize(44 + num_samples as usize, 0x80);

        std::fs::write(&wav_path, &data).unwrap();

        // Valid with expected duration 1s
        let res = validate_audio_file(&wav_path, Some(1), "wav", AudioFormat::Wav);
        assert!(
            res.is_ok(),
            "Expected valid WAV to pass validation: {:?}",
            res
        );
        assert_eq!(res.unwrap(), 1);

        // Fails if expected duration is 60s (diff > 5s)
        let res_fail = validate_audio_file(&wav_path, Some(60), "wav", AudioFormat::Wav);
        assert!(res_fail.is_err(), "Expected duration mismatch to fail");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_validate_audio_file_webm_opus_mislabeled_m4a() {
        // Regression: Opus-in-WebM mislabeled as `.m4a`
        // (EBML `1A 45 DF A3`, e.g. https://d.uguu.se/jXSTGTDj.m4a which is
        // byte-identical to scratch/sample.m4a). lofty guesses `Mpeg` and
        // rodio reports "format not recognized", so validation must fall back
        // to the Symphonia Opus probe instead of rejecting the download.
        let sample_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch/sample.m4a");
        if !sample_path.exists() {
            eprintln!("scratch/sample.m4a not found, skipping test");
            return;
        }
        assert!(is_ebml_container(&sample_path));
        let res = validate_audio_file(&sample_path, None, "m4a", AudioFormat::M4a);
        assert!(
            res.is_ok(),
            "Expected WebM/Opus mislabeled as .m4a to pass validation: {:?}",
            res
        );
        assert!(res.unwrap() > 0, "Duration should be non-zero");
    }

    #[tokio::test]
    async fn test_validate_audio_file_async_performance_baseline() {
        let dir = std::env::temp_dir().join(format!("auralis_perf_test_{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("test.part");

        // 1s 8000Hz 8-bit mono WAV = 8044 bytes
        let sample_rate: u32 = 8000;
        let num_samples: u32 = 8000;
        let mut data = Vec::with_capacity(44 + num_samples as usize);
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + num_samples).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes()); // Mono
        data.extend_from_slice(&sample_rate.to_le_bytes());
        data.extend_from_slice(&sample_rate.to_le_bytes()); // Byte rate
        data.extend_from_slice(&1u16.to_le_bytes()); // Block align
        data.extend_from_slice(&8u16.to_le_bytes()); // Bits per sample
        data.extend_from_slice(b"data");
        data.extend_from_slice(&num_samples.to_le_bytes());
        data.resize(44 + num_samples as usize, 0x80);

        std::fs::write(&wav_path, &data).unwrap();

        let start_sync = std::time::Instant::now();
        let sync_res = validate_audio_file(&wav_path, Some(1), "wav", AudioFormat::Wav);
        let sync_elapsed = start_sync.elapsed();
        assert!(sync_res.is_ok());

        let start_async = std::time::Instant::now();
        let async_res =
            validate_audio_file_async(&wav_path, Some(1), "wav", AudioFormat::Wav).await;
        let async_elapsed = start_async.elapsed();
        assert!(async_res.is_ok());

        println!(
            "validate_audio_file sync time: {:?}, async spawn_blocking time: {:?}",
            sync_elapsed, async_elapsed
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
