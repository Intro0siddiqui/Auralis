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
use tracing::{debug, error, info, warn};
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
    /// Optional HTTP headers to send with the googlevideo request (UA/Referer
    /// matched to the InnerTube client that generated the URL). If absent,
    /// sane YouTube defaults are injected.
    pub headers: Option<HashMap<String, String>>,
}

/// Per-job bookkeeping required to (re)start and resume a download.
#[derive(Clone)]
struct DownloadJob {
    stream_url: String,
    output_path: PathBuf,
    thumbnail: Option<String>,
    headers: Option<HashMap<String, String>>,
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

/// Replace filesystem-unsafe characters so titles produce valid filenames.
/// Strips path separators, control chars, "..", reserved Windows names, and
/// limits length to 200 chars. Never returns empty or "." / "..".
fn sanitize_filename(name: &str) -> String {
    // Take basename only — strip any directory components
    let base = name.rsplit(&['/', '\\'][..]).next().unwrap_or(name);
    // Remove control chars then replace unsafe chars
    let filtered: String = base.chars().filter(|c| !c.is_control()).collect();
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
        trimmed = trimmed.trim_end_matches('.').to_string();
        if trimmed.is_empty() {
            return "audio_track".to_string();
        }
    }
    trimmed
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
        let host = req.stream_url.split('/').nth(2).unwrap_or("unknown");
        let has_headers = req.headers.is_some();
        let header_keys = req
            .headers
            .as_ref()
            .map(|h| h.keys().cloned().collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        info!(url = %req.stream_url, title = %req.title, platform = %req.platform, ext = %req.ext, total_bytes = ?req.total_bytes, host = %host, has_headers = %has_headers, headers = %header_keys, "Starting download");
        debug!(url = %req.stream_url, headers = ?req.headers, "Download request headers");

        if !req.stream_url.starts_with("https://") && !req.stream_url.starts_with("http://") {
            warn!(url = %req.stream_url, "Rejected non-http(s) URL");
            return Err(DownloaderError::InvalidUrl(req.stream_url));
        }

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
                    headers: req.headers,
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
                error!(download_id = %id, url_host = %url_host, url = %job.stream_url, start_byte = start_byte, error = %e, "Download failed — will mark as failed with diagnostic");
                warn!(download_id = %id, error = %e, "DIAGNOSTIC download_failed id={} host={} url={} error={}", id, url_host, job.stream_url, e);
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
    /// On failure the returned `DownloaderError` string is intentionally
    /// verbose (status + body snippet + host + byte counts) so it surfaces
    /// in the `download:completed` event's `error` field, the toast, and
    /// `adb logcat` — users diagnose without devtools.
    async fn run_stream(
        id: Uuid,
        job: &DownloadJob,
        start_byte: u64,
        active: Arc<RwLock<HashMap<Uuid, DownloadProgress>>>,
    ) -> Result<(), DownloaderError> {
        let host = job.stream_url.split('/').nth(2).unwrap_or("unknown");
        let url_snip = if job.stream_url.len() > 160 {
            format!("{}…", &job.stream_url[..160])
        } else {
            job.stream_url.clone()
        };
        // Build client: use supplied UA if present, otherwise sane default.
        let ua = job
            .headers
            .as_ref()
            .and_then(|h| h.get("User-Agent").or_else(|| h.get("user-agent")).cloned())
            .unwrap_or_else(|| "Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UD1A.230803.041) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36".to_string());

        debug!(download_id = %id, host = %host, ua = %ua, start_byte = start_byte, "Building HTTP client for stream");
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

        let mut req = client.get(&job.stream_url);
        // Inject headers that googlevideo validates: Referer/Origin/Accept.
        // Caller (youtube.js) supplies a client-matched UA; we add the rest.
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
        if let Some(h) = &job.headers {
            for (k, v) in h {
                if k.eq_ignore_ascii_case("user-agent") {
                    continue;
                }
                injected.insert(k.clone(), v.clone());
            }
        }
        let injected_keys = injected.keys().cloned().collect::<Vec<_>>().join(",");
        info!(download_id = %id, host = %host, injected = %injected_keys, start_byte = start_byte, "Sending GET for stream");
        for (k, v) in injected {
            req = req.header(k, v);
        }
        if start_byte > 0 {
            req = req.header("Range", format!("bytes={}-", start_byte));
        }
        let mut res = tokio::time::timeout(Duration::from_secs(30), req.send())
            .await
            .map_err(|_| {
                let msg = format!("request timed out after 30s [{host}] start_byte={start_byte} url={url_snip}");
                error!(download_id = %id, host = %host, start_byte = start_byte, "Request timeout (30s)");
                warn!(download_id = %id, "DIAGNOSTIC download_timeout id={} host={} start_byte={} url={} ua={}", id, host, start_byte, url_snip, ua);
                DownloaderError::HttpError(msg)
            })?
            .map_err(|e| {
                let msg = format!("request failed [{host}] start_byte={start_byte}: {e} (url={url_snip})");
                error!(download_id = %id, host = %host, error = %e, "Request failed");
                DownloaderError::HttpError(msg)
            })?;

        let status = res.status();
        // Use reqwest StatusCode for correctness; 206 = PartialContent
        let resuming = start_byte > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        if start_byte > 0 && status == reqwest::StatusCode::OK {
            // Server ignored Range — must truncate and restart from 0, otherwise
            // `downloaded = start_byte` overcounts and file contains duplicate prefix
            warn!(download_id = %id, start_byte = start_byte, "Range request got 200 not 206 — truncating file and resetting to 0 (server does not support resume)");
            {
                let mut guard = active.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    state.downloaded_bytes = 0;
                    state.progress = 0.0;
                    state.updated_at = Utc::now();
                }
            }
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

        info!(download_id = %id, host = %host, status = %status, content_type = %ct, content_length = %cl_hdr, total = ?total, resuming = resuming, "Received response headers");

        if !status.is_success() && status.as_u16() != 206 {
            let body_snip = match tokio::time::timeout(Duration::from_secs(5), res.text()).await {
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
                403 => " — 403 Forbidden: googlevideo rejected UA/Referer/Origin/PO-token or URL expired [rr1---sn-gwpa-cived] — 2026 Jio sn-gwpa-cived now gates TV too (try re-resolving with ANDROID+pot or WEB_SAFARI; set youtube_po_token via BgUtils mint or Settings cookie)",
                404 => " — 404: URL expired or invalid (re-resolve the video)",
                416 => " — 416 Range Not Satisfiable: resume offset beyond file size (will retry from 0)",
                429 => " — 429 Too Many Requests: rate-limited, retry later",
                500..=599 => " — server error, retry later",
                _ => "",
            };
            let msg = format!("HTTP {status} [{host}]{hint} body: {body_snip} (url={url_snip}, start_byte={start_byte}, ct={ct})");
            error!(download_id = %id, host = %host, status = %status, body = %body_snip, "HTTP error response");
            warn!(download_id = %id, "DIAGNOSTIC download_http_error id={} host={} status={} ct={} hint={} body={} url={}", id, host, status, ct, hint, body_snip, url_snip);
            return Err(DownloaderError::HttpError(msg));
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
                .await
                .map_err(|e| {
                    error!(download_id = %id, path = ?job.output_path, error = %e, "Failed to open file for append (resume)");
                    DownloaderError::IoError(e)
                })?
        } else {
            tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&job.output_path)
                .await
                .map_err(|e| {
                    error!(download_id = %id, path = ?job.output_path, error = %e, "Failed to create/truncate output file");
                    DownloaderError::IoError(e)
                })?
        };

        if !resuming {
            if let Err(e) = file.seek(SeekFrom::Start(0)).await {
                warn!(download_id = %id, error = %e, "Seek to start failed (non-fatal)");
            }
        }

        let start_instant = Instant::now();
        // If server returned 200 to a Range request, we truncated the file —
        // downloaded must start at 0, not start_byte (otherwise overcount)
        let effective_start = if resuming { start_byte } else { 0 };
        let mut downloaded = effective_start;

        loop {
            let chunk_opt = tokio::time::timeout(Duration::from_secs(30), res.chunk())
                .await
                .map_err(|_| {
                    let elapsed = start_instant.elapsed().as_secs();
                    let msg = format!("stream stalled: no data for 30s [{host}] downloaded={downloaded}/{:?} elapsed={elapsed}s (url={url_snip})", total);
                    error!(download_id = %id, host = %host, downloaded = downloaded, total = ?total, elapsed = elapsed, "Stream stalled (30s timeout)");
                    warn!(download_id = %id, "DIAGNOSTIC download_stalled id={} host={} downloaded={}/{:?} elapsed={}s url={}", id, host, downloaded, total, elapsed, url_snip);
                    DownloaderError::HttpError(msg)
                })?
                .map_err(|e| {
                    let msg = format!("stream read error [{host}] downloaded={downloaded}/{:?}: {e} (url={url_snip})", total);
                    error!(download_id = %id, host = %host, error = %e, downloaded = downloaded, "Stream read error");
                    DownloaderError::HttpError(msg)
                })?;

            let Some(chunk) = chunk_opt else { break };
            if chunk.is_empty() {
                continue;
            }
            if let Err(e) = file.write_all(&chunk).await {
                error!(download_id = %id, path = ?job.output_path, error = %e, chunk_len = chunk.len(), "File write failed");
                warn!(download_id = %id, "DIAGNOSTIC download_io_error id={} path={:?} error={} downloaded={}", id, job.output_path, e, downloaded);
                return Err(DownloaderError::DownloadFailed(format!(
                    "failed to write {} bytes to {:?}: {e} (downloaded={}/{:?})",
                    chunk.len(),
                    job.output_path,
                    downloaded,
                    total
                )));
            }
            downloaded += chunk.len() as u64;

            let elapsed = start_instant.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                ((downloaded - effective_start) as f64 / elapsed) as u64
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

        // Android: also publish a copy to the user-visible Download/Auralis/ via MediaStore
        // so it appears in the system Files app like a browser download. Non-fatal;
        // pause/resume continues to use the sandboxed file.
        // Dual-save is gated by `use_system_downloads` (default true per settings.rs
        // `default_true`). We evaluate the persisted setting at publish time so the
        // default `true` path always publishes; only an explicit `false` skips it.
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
                    // publish_to_downloads already warns with source/error, add call-site context.
                    // Common causes: JNI env unavailable, is_pending insert failure, empty source.
                    warn!(download_id = %id, src = %job.output_path.display(), "MediaStore publish returned None — keeping internal path (see prior warn for root cause; no permission needed on API 29+ via MediaStore)");
                }
            }
        }

        info!(download_id = %id, path = ?job.output_path, download_dir = %job.output_path.display(), "Download complete (internal retained; public copy was attempted above on Android when enabled)");
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
