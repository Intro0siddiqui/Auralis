//! Media Downloader
//!
//! Streams media from a *resolved* direct audio URL (e.g. produced by the
//! frontend `youtube.js` resolver) to disk using `reqwest`, with pause/resume
//! (via HTTP `Range`) and cancel support.
//!
//! No external binaries (`yt-dlp` / `ffmpeg`) or dedicated Rust YouTube crates
//! are required — resolution of user-facing URLs (YouTube, SoundCloud, …) is
//! the frontend's responsibility; this layer only fetches bytes.

/// Imported rather than spelled out at the call site: the fully qualified path
/// is 86 characters wide, so the one line that calls it sat within a character
/// or two of `max_width` and the two rustfmt versions this project builds with
/// disagreed about where to break it. A short name has no such decision to
/// make, and off Android there is nothing to import at all.
#[cfg(target_os = "android")]
use super::android_downloads::publish_to_downloads;
use super::completeness::verify_decoded_duration;
use super::forensics::{inspect_container, inspect_content, ContainerFacts, ContentFacts, Verdict};
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
use tokio::sync::{Mutex, RwLock};
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
    /// Display title used for the output filename, UI and the tags written
    /// into the finished file.
    pub title: String,
    /// Optional artist, written into the finished file's tags so the library
    /// scanner does not fall back to `Unknown Artist`.
    pub artist: Option<String>,
    /// Optional album, written into the finished file's tags.
    pub album: Option<String>,
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
    title: String,
    artist: Option<String>,
    album: Option<String>,
    /// The final internal path. **Owned by this job alone** — see
    /// [`reserve_output`], which is what made that exclusive, and
    /// [`discard_owned_paths`], which is the only thing allowed to delete it.
    output_path: PathBuf,
    /// The staging file, and simultaneously the reservation that makes
    /// `output_path` this job's alone. Never removed while the job is
    /// resumable: `pause` truncates it and `resume` appends to it.
    staging_path: PathBuf,
    /// Name the public copy carries: the clean title, never the dedup suffix
    /// that only the internal path needs.
    ///
    /// Read only by the Android MediaStore publish path, so on a host build
    /// this is genuinely unread and `dead_code` fires — which fails the `lint`
    /// job, because clippy runs `-D warnings` there while `check-android` is
    /// the only job that would see the read. Gate the lint, not the field:
    /// removing it would leave Android with no clean name to publish under.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    public_name: String,
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
    /// Held across the commit boundary: the move to `Committing`, the rename,
    /// and the move to `Completed`.
    ///
    /// This is what makes the refusal in [`classify_interrupt`] mean what it
    /// says. `pause` and `cancel` take this gate *before* they read the status,
    /// so a job is observed strictly before the commit or strictly after it —
    /// `Committing` is not observable to them at all, because it only exists
    /// inside one hold. Without the gate the table alone is not enough: a
    /// request that read `Downloading` and *then* aborted the task would still
    /// be able to kill a job microseconds before its rename, leaving a stranded
    /// `Committing` record and a file nobody ever reports.
    ///
    /// It is released as soon as the rename is durable, before cover art and
    /// the public copy: those are the slow parts, and an interrupt that had to
    /// queue behind them would look like a hang.
    commit_gate: Arc<Mutex<()>>,
}

/// Downloader errors.
#[derive(Debug, thiserror::Error)]
pub enum DownloaderError {
    #[error("Download not found: {0}")]
    DownloadNotFound(Uuid),

    #[error("Invalid download state: {0}")]
    InvalidState(String),

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

/// Extension of a staging file — which is also what makes it a reservation.
const STAGING_EXT: &str = "part";

/// How many output names one download may try before giving up.
///
/// Eight is not a tuning parameter: the fallback names are all distinct, so this
/// only bounds a pathological case (every candidate already on disk) that would
/// otherwise spin. Failing is the right answer there, because the alternative
/// is sharing a path with a job that already holds it.
const RESERVE_ATTEMPTS: usize = 8;

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

/// A decoded length or a container sample table that covers less than this
/// fraction of the expected track counts as short. Deliberately the same
/// threshold `completeness.rs` uses, so the decoder's gate and the
/// container/content cross-check cannot disagree about what "short" means. The
/// *audible* check below deliberately does not use it — see
/// [`MIN_AUDIBLE_RATIO`].
const MIN_COVERAGE_RATIO: f64 = 0.9;

/// The audible fraction below which an otherwise complete file is still refused.
///
/// `audible_secs` is the position of the last sample above the silence
/// threshold, not a length. It therefore cannot be *required*: a track whose
/// last 10 % is a legitimate fade-out is complete, and demanding audible audio
/// for 90 % of it sent real downloads through four pointless range-top-up
/// rounds. It can still **veto**, because a server-side window keeps serving
/// container and silence long after it stops serving audio — the one shape where
/// the byte count is complete, the sample table describes the full track, and
/// the file is still not the track.
///
/// Both device measurements of that window sit at roughly a third of the track
/// (99 s of 287 s; 75 s of 216 s), so half clears them with room to spare while
/// leaving any plausible silent tail alone. This is an **inference** from those
/// two samples, not a measured constant: the length check above is the
/// load-bearing one, and this only has to tell a fade-out from a window.
const MIN_AUDIBLE_RATIO: f64 = 0.5;

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
pub(crate) fn inject_stream_headers(
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
/// limits length to fit within the filesystem's 255-byte NAME_MAX. Never
/// returns empty or "." / "..".
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
    // Limit stem to 246 bytes (255 NAME_MAX − 8 max ext − 1 dot).
    // Char-based limit allowed CJK titles (~600 bytes) to exceed the OS limit.
    const MAX_STEM_BYTES: usize = 246;
    if trimmed.len() > MAX_STEM_BYTES {
        let mut end = MAX_STEM_BYTES;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed = trimmed[..end].to_string();
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

// ---------------------------------------------------------------------------
// Completeness cross-check (container structure + decoded content)
// ---------------------------------------------------------------------------

/// Everything the completeness gate needs from one on-disk file.
///
/// The three inspections are bundled because they answer a single question —
/// *is this file whole?* — and because every one of them is a full read or a
/// full decode of the file.
struct ForensicReport {
    container: ContainerFacts,
    content: ContentFacts,
    /// `completeness::verify_decoded_duration`'s verdict.
    ///
    /// Kept only for the unparseable-container case, where no structural
    /// evidence exists and the decoder's own number is all that is left. It is
    /// the least trustworthy of the three, so it never gets the last word
    /// anywhere else.
    decoded: Result<Option<u64>, String>,
}

/// What the cross-check concluded about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Acceptance {
    /// Proven whole: keep it.
    Accept,
    /// Not proven whole. `reason` names the single signal that decided, so the
    /// failure shown to the user is attributable rather than vague.
    Reject(&'static str),
}

// The reject reasons are `const`s rather than literals inline in `acceptance`
// for two reasons. They are long enough that an unbreakable literal would push
// its line past `max_width`, and rustfmt gives up on an item it cannot fit —
// which would silently take the whole gate out of the `cargo fmt --check` gate.
// And a test can assert against the very same value production returns, so a
// reworded message cannot quietly stop being the one the test checks.
const REASON_MISSING_BYTES: &str = "the sample table references bytes the file does not contain";
const REASON_SHORT_TABLE: &str = "the container itself describes a shorter track than the \
                                   resolver expected, so the server sent a windowed object \
                                   and reported it as complete";
const REASON_SILENT_TAIL: &str = "the bytes and the decoded length are complete but audio \
                                 stops well before the end, which is the signature of a \
                                 server-side window rather than of a whole file";
const REASON_UNPARSABLE: &str = "the container could not be parsed and the decoder agrees \
                                  it is short";

/// Does this measured length cover the expected track?
///
/// `expected_secs == 0.0` means no expectation was available, and a
/// measurement cannot prove shortness against nothing, so it does not veto.
///
/// What an **absent** measurement means is deliberately not decided here:
/// `forensics::ContentFacts::measured_secs` returns `None` exactly when no
/// decoder could be built, and its own documentation says a caller "must fall
/// back to its other evidence" — so the decision belongs to `acceptance`, which
/// has the other evidence in hand.
fn covers(value: f64, expected_secs: f64) -> bool {
    if expected_secs <= 0.0 {
        return true;
    }
    value >= expected_secs * MIN_COVERAGE_RATIO
}

/// Cross-check the container and the decoded content of one file.
///
/// The **order** is the whole point, and it is what NEW-04 turned over:
///
/// 1. Bytes, from the sample table. Exact, needs no decoding, and
///    `Verdict::Truncated` is a hard no.
/// 2. Container coverage — does the table describe the full track?
/// 3. NOT the decoded length. A short decode is non-evidence, not failure:
///    measured on `yF9nmg_jHNs` (2026-09-29, dev box) — container `Complete`,
///    table 216.3 s, every byte present — the decoder walked 25.2 % of the
///    samples (`measured=54.4s`) and the gate refused a perfect file. The
///    transfer was always whole; the decoder stopped early. Byte accounting
///    (step 1) and table coverage (step 2) prove what a decode cannot
///    disprove, so a short `measured_secs` abstains here unconditionally.
/// 4. Audible position — consulted last, only as a **veto**, and only when
///    the decode actually walked the whole track (`measured_secs` covers).
///    Then a short audible position means observed trailing silence, the
///    signature of a served window. When measured is short too, audible is
///    just where the decode ended and adds no information, so the veto
///    abstains with it.
///
/// Step 4 has to stay in that gated form. A server-side window is the shape
/// where steps 1-2 pass and the decoded stream is the right length — because
/// what arrived after the cutoff is silence. Removing the veto would silently
/// delete the only check that catches it; applying it to a short decode
/// would refuse every file whose decoder stops early, which is the bug this
/// ordering exists to prevent.
fn acceptance(report: &ForensicReport, expected_secs: f64) -> Acceptance {
    match report.container.verdict.as_ref() {
        // The sample table references bytes the file does not have. No amount
        // of decoded audio makes that whole.
        Some(Verdict::Truncated { .. }) => return Acceptance::Reject(REASON_MISSING_BYTES),
        Some(Verdict::Complete) => {}
        // `Verdict::Unknown` and an absent verdict mean the same thing: the box
        // walker read nothing — a file over `forensics::MAX_INSPECT_BYTES`, or a
        // container it does not parse. `forensics` documents that as "keep your
        // previous behaviour", and the previous behaviour is the decoder's
        // verdict, so that is what decides here.
        _ => {
            return match &report.decoded {
                Ok(_) => Acceptance::Accept,
                Err(_) => Acceptance::Reject(REASON_UNPARSABLE),
            }
        }
    }

    // The table is `Some` whenever the verdict is — `inspect_container` only
    // reports one from a structure whose duration it read — so an absent table
    // here is not a measurement question, it is an absence of evidence, and an
    // absence of evidence does not pass a completeness gate.
    if !report
        .container
        .table_secs
        .is_some_and(|table| covers(table, expected_secs))
    {
        return Acceptance::Reject(REASON_SHORT_TABLE);
    }

    // A short decode is non-evidence (see the step-3 note above), so there
    // is no length rejection here at all: byte accounting and table coverage
    // already decided. What remains is the audible veto, gated on the decode
    // having walked the whole track — only then is a short audible position
    // observed silence rather than the place the decoder stopped.
    //
    // `measured_secs() == None` means the sample stream could not be walked
    // at all, and `forensics` is explicit that this "never means empty": the
    // veto abstains with it, and the container's proof stands.
    let decoded_covers = report
        .content
        .measured_secs()
        .is_some_and(|measured| covers(measured, expected_secs));
    if decoded_covers
        && report
            .content
            .audible_secs
            .is_some_and(|audible| audible < expected_secs * MIN_AUDIBLE_RATIO)
    {
        return Acceptance::Reject(REASON_SILENT_TAIL);
    }

    Acceptance::Accept
}

/// Gather container structure, decoded content and the decoder's own verdict in
/// a **single blocking hop**.
///
/// `inspect_container` reads up to 192 MB into memory, and both decoder passes
/// walk the whole stream. Inline in the async download task that is seconds of a
/// Tokio worker held for each of them, and a download is not the only task on
/// that runtime.
///
/// The hop is also what makes the post-append recompute affordable: the facts
/// have to be re-derived after *every* append, so re-deriving them inline
/// would block the runtime once per top-up round instead of once.
///
/// A join failure yields default facts, which read as `Verdict::None` — the
/// unparseable-container path — and a `decoded` of `Err`, so a lost task
/// refuses the file rather than waving it through.
async fn gather_forensics(
    path: &Path,
    ext: &str,
    expected_duration_secs: Option<u32>,
) -> ForensicReport {
    let path_buf = path.to_path_buf();
    let ext_owned = ext.to_string();
    let joined = tokio::task::spawn_blocking(move || ForensicReport {
        container: inspect_container(&path_buf),
        content: inspect_content(&path_buf, &ext_owned),
        decoded: verify_decoded_duration(&path_buf, &ext_owned, expected_duration_secs),
    })
    .await;
    match joined {
        Ok(report) => report,
        Err(join_err) => {
            warn!(error = %join_err, path = %path.display(), "Forensics did not complete");
            ForensicReport {
                container: ContainerFacts::default(),
                content: ContentFacts::default(),
                decoded: Err(format!(
                    "forensic inspection task did not complete: {join_err}"
                )),
            }
        }
    }
}

/// What one completed HTTP response did to the stream and the retry budget.
///
/// Split out as its own function because the defect it fixes (DL-05) was a
/// *hole in the branch structure*, not bad arithmetic, and the branch structure
/// is the only thing a unit test can pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamStep {
    /// Bytes arrived — the retry budget resets.
    Progress,
    /// The read failed or stalled part way through.
    Interrupted,
    /// A clean early end, with bytes provably still outstanding.
    Short { advertised: u64 },
    /// A clean end that delivered nothing and proves nothing is outstanding.
    ///
    /// Still an error. A silent response is not evidence that the object ended,
    /// and treating it as one is the other half of the infinite loop: with an
    /// advertised length of zero, both "all bytes received" (`total > 0`) and
    /// "bytes still outstanding" (`total > current`) are false, so a zero-byte
    /// response fell through every branch and the loop asked again, forever.
    NoProgress,
}

/// Classify what one finished response did.
///
/// The order is load-bearing: bytes that arrived are progress *even if* the read
/// afterwards failed, because the next request resumes from the real on-disk
/// length. Everything else that delivered nothing is an error.
fn classify_response(
    bytes_in_this_request: u64,
    stream_interrupted: bool,
    total_bytes: Option<u64>,
    current_downloaded: u64,
) -> StreamStep {
    if bytes_in_this_request > 0 {
        return StreamStep::Progress;
    }
    if stream_interrupted {
        return StreamStep::Interrupted;
    }
    match total_bytes {
        Some(advertised) if advertised > current_downloaded => StreamStep::Short { advertised },
        // A zero advertised length, an advertised length already satisfied, or
        // no advertised length at all: in every one of those, "bytes still
        // outstanding" cannot be shown and yet nothing arrived.
        _ => StreamStep::NoProgress,
    }
}

/// The cover-art sidecar path for a committed audio file.
///
/// One path, one owner: the sidecar name is derived from the output name, which
/// is what the reservation makes exclusive, so a job that owns `<name>.mp4`
/// owns `<name>.jpg` by the same token.
fn sidecar_path(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("jpg")
}

/// One job's exclusive claim on an output name.
///
/// The claim **is** the staging file. Nothing else is created, nothing else is
/// registered, and there is no release to forget: the marker disappears when
/// the staging file is renamed to its final name at commit time, or when a
/// failure path removes it — which is exactly the window the claim is meant to
/// cover.
struct OutputReservation {
    /// Where the verified bytes will live.
    output_path: PathBuf,
    /// The claim itself. Present from reservation until the rename or cleanup.
    staging_path: PathBuf,
    /// The name the *public* copy should carry.
    ///
    /// Always the clean `<title>.<ext>`, even when the internal path needed a
    /// dedup suffix. The suffix exists only to keep two internal files apart;
    /// the owner looking at `Download/Auralis/` has not downloaded anything
    /// twice, so showing them a UUID is a lie about their own library.
    public_name: String,
}

/// Filenames to try, in order, for one download's output.
///
/// The first is the clean `<title>.<ext>`; the rest carry a short job-id suffix.
/// The suffix is what the fallback *looks* like — exclusivity comes from
/// [`reserve_output`] — but it is derived from the job id, so two concurrent
/// jobs of the same title never even propose the same fallback name.
fn candidate_file_names(stem: &str, ext: &str, id: Uuid) -> Vec<String> {
    let short: String = id.to_string().chars().take(8).collect();
    (0..RESERVE_ATTEMPTS)
        .map(|attempt| match attempt {
            0 => format!("{stem}.{ext}"),
            1 => format!("{stem}_{short}.{ext}"),
            n => format!("{stem}_{short}_{n}.{ext}"),
        })
        .collect()
}

/// Take an exclusive claim on an output name for `id`.
///
/// # Why an exclusive create and not a `stat`
///
/// The name used to be chosen with `if path.exists() { add a uuid }`, and there
/// was an `await` between that test and the first create. Two jobs with the
/// same title both observed "free" and were handed the *same* `output_path`;
/// one of them then deleted the other's finished file on the way out, and two
/// that both succeeded overwrote each other. `create_new` is the only
/// filesystem operation that answers "is this name free?" and *takes* it in a
/// single step, so the answer and the claim cannot be separated by a
/// scheduling point.
///
/// The claim is deliberately a staging file and not a zero-byte file at the
/// destination. Occupying the destination would put an empty
/// `Song.mp4` in the library folder for the length of the download, and — worse
/// — `rename` fails on Windows when the target exists, so every Windows
/// download would be pushed onto the non-atomic copy fallback.
///
/// A destination that already exists is *rejected*, never claimed: a job must
/// not adopt or overwrite a file from an earlier run.
async fn reserve_output(
    output_dir: &Path,
    staging_dir: &Path,
    stem: &str,
    ext: &str,
    id: Uuid,
) -> Result<OutputReservation, DownloaderError> {
    for file_name in candidate_file_names(stem, ext, id) {
        let output_path = output_dir.join(&file_name);
        if output_path.exists() {
            debug!(
                download_id = %id,
                candidate = %file_name,
                "Output name already used by an earlier download"
            );
            continue;
        }
        let staging_path = staging_dir.join(format!("{file_name}.{STAGING_EXT}"));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
            .await
        {
            Ok(claim) => {
                // Closed immediately: the file's *existence* is the claim, and
                // an open handle would only stop the commit from renaming over
                // it on Windows.
                drop(claim);
                // A cover-art sidecar whose audio is gone is a leftover from an
                // earlier run of the same title. The claim is exclusive, so
                // nothing else can be using it, and inheriting it would attach
                // the wrong artwork to this download — or, when this download
                // has no thumbnail of its own, leave the stale one sitting next
                // to the new file for good.
                let stale_cover = sidecar_path(&output_path);
                if let Err(e) = tokio::fs::remove_file(&stale_cover).await {
                    debug!(
                        path = %stale_cover.display(),
                        error = %e,
                        "No stale cover-art sidecar to clear"
                    );
                }
                return Ok(OutputReservation {
                    output_path,
                    staging_path,
                    public_name: format!("{stem}.{ext}"),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                debug!(
                    download_id = %id,
                    candidate = %file_name,
                    "Output name already claimed by a live download"
                );
                continue;
            }
            Err(e) => {
                return Err(DownloaderError::IoError(e));
            }
        }
    }
    // Built here rather than hoisted above the loop: hoisting it and moving it
    // into the successful arm would be a move out of a loop body, which the
    // borrow checker rejects outright.
    Err(DownloaderError::InvalidState(format!(
        "could not claim an unused filename for {stem}.{ext} in {}",
        output_dir.display()
    )))
}

/// Delete the bytes this job owns — and only those.
///
/// The output path is removed only while the claim is still held, i.e. only
/// while the staging file this job created is still on disk. Because the claim
/// is what makes the name exclusive, a file sitting at the output path at that
/// moment can only be this job's own partial work. Once the claim is gone the
/// file is *committed*, and removing it would delete a finished track.
///
/// The claim is read before the staging file is touched, because removing the
/// staging file is what releases it.
async fn discard_owned_paths(staging: &Path, output: &Path) {
    let holds_claim = tokio::fs::metadata(staging)
        .await
        .map(|meta| meta.is_file())
        .unwrap_or(false);
    cleanup_staging_file(staging).await;
    if !holds_claim {
        debug!(
            output = %output.display(),
            "Keeping the output file — this job's claim is already released, so it is committed"
        );
        return;
    }
    if let Err(e) = tokio::fs::remove_file(output).await {
        debug!(path = %output.display(), error = %e, "No partial output file to remove");
    }
    let cover = sidecar_path(output);
    if let Err(e) = tokio::fs::remove_file(&cover).await {
        debug!(path = %cover.display(), error = %e, "No cover-art sidecar to remove");
    }
}

/// Empty the staging directory at construction.
///
/// A staging file *is* a claim (see [`reserve_output`]), so a crash part way
/// through a download would otherwise hold that name until the user manually
/// cleared the directory. The same directory holds the transient public-name
/// link, which a crash can strand too. Both are per-process scratch, and both
/// are gone by the time anything can be mid-download.
///
/// Wholesale, and only here: the `Downloader` is constructed once per process,
/// before any job exists, so nothing reachable from this function can be live.
/// A job killed mid-run — never swept, only released early — leaks its claim
/// until the next start, which is the safe direction to be wrong in.
fn sweep_stale_staging(staging_dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(staging_dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            debug!(path = %staging_dir.display(), error = %e, "Could not clear the staging directory");
        }
        return;
    }
    if let Err(e) = std::fs::create_dir_all(staging_dir) {
        debug!(path = %staging_dir.display(), error = %e, "Could not recreate the staging directory");
    }
}

/// Present `internal` under `public_name` without copying a single byte of it.
///
/// `publish_to_downloads` builds the MediaStore display name from the file name
/// of the path it is handed, so the only lever this side owns is the path. A
/// hard link says "this file, under that name" with no resolution involved
/// anywhere and no second copy of the bytes; the caller removes the link as
/// soon as the copy exists.
///
/// The link goes in `link_dir` (the staging directory), **not** beside the
/// finished file. Beside it is exactly where the clean name is *already taken* —
/// that is the only reason the internal name needed a suffix in the first place —
/// so a link placed there would fail with `EEXIST` in precisely the case it
/// exists for. The staging directory is this job's own scratch space, and
/// nothing scans it.
///
/// Returns `None` when the internal name already is the public name (nothing to
/// do) and when no link could be made. The second case is a lost nicety, not a
/// failure: the caller publishes the internal path and the public copy keeps the
/// suffix, which is what it did before.
///
/// Two live jobs of the same title both want the same link name. Only one gets
/// it; the other publishes from its internal path. That is a race on a
/// presentation detail with no correctness consequence — no shared bytes, no
/// deletion, no lost file — so it is left unarbitrated rather than paid for with
/// a second round of locking.
///
/// The link is briefly visible to the library scanner, which is why the caller's
/// post-commit flag keeps `download:completed` (and the scan it triggers) behind
/// it.
#[cfg(any(target_os = "android", test))]
fn link_under_public_name(internal: &Path, link_dir: &Path, public_name: &str) -> Option<PathBuf> {
    let existing = internal
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    if existing == public_name {
        return None;
    }
    let link = link_dir.join(public_name);
    match std::fs::hard_link(internal, &link) {
        Ok(()) => Some(link),
        Err(e) => {
            // Kept short on purpose: rustfmt cannot break a string literal, and a
            // `warn!` line it cannot fit is known to drop the statements around
            // it out of the `cargo fmt --check` gate.
            warn!(
                internal = %internal.display(),
                public_name = %public_name,
                error = %e,
                "No link under the clean name — the public copy will keep the suffix"
            );
            None
        }
    }
}

/// A user request to interrupt a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    Pause,
    Cancel,
}

impl Interrupt {
    /// Past tense, for error messages: "cannot pause download in …".
    fn verb(self) -> &'static str {
        match self {
            Interrupt::Pause => "pause",
            Interrupt::Cancel => "cancel",
        }
    }
}

/// What a downloader may do about an interrupt request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestOutcome {
    /// Carry the request out.
    Apply,
    /// The job is already in the requested state: nothing to do, and that is
    /// success rather than an error.
    AlreadySettled,
    /// Refuse. The job is past the point where interrupting it is safe.
    Refuse,
}

/// The interrupt state table, as data.
///
/// | state       | pause           | cancel            |
/// |-------------|-----------------|-------------------|
/// | Queued      | apply           | apply             |
/// | Downloading | apply           | apply             |
/// | Paused      | already settled | apply             |
/// | Committing  | refuse          | refuse            |
/// | Completed   | refuse          | refuse            |
/// | Failed      | refuse          | refuse            |
/// | Cancelled   | refuse          | already settled   |
///
/// `Committing` is the row DL-02 added. It spans the rename, and after the
/// rename there is no staging file left to truncate and no partial file worth
/// deleting: a cancel in that window used to remove a finished, playable track,
/// and a pause reported `Paused` over a job with no resume source. The refusal
/// is a *deletion* guard; keeping the request from aborting the task mid-rename
/// is [`Downloader::commit_gate`]'s job, and neither layer is redundant.
fn classify_interrupt(request: Interrupt, status: DownloadStatus) -> RequestOutcome {
    match status {
        DownloadStatus::Queued | DownloadStatus::Downloading => RequestOutcome::Apply,
        DownloadStatus::Paused => match request {
            Interrupt::Pause => RequestOutcome::AlreadySettled,
            Interrupt::Cancel => RequestOutcome::Apply,
        },
        // One state per arm rather than one `Committing | Completed | Failed`
        // arm. Grouping them forces rustfmt to wrap the body in a block, and how
        // a block-bodied or-pattern arm is wrapped is not something to leave to
        // two rustfmt versions that have already been caught disagreeing
        // elsewhere in this project. Nothing here needs the grouping.
        DownloadStatus::Committing => RequestOutcome::Refuse,
        DownloadStatus::Completed => RequestOutcome::Refuse,
        DownloadStatus::Failed => RequestOutcome::Refuse,
        DownloadStatus::Cancelled => match request {
            Interrupt::Pause => RequestOutcome::Refuse,
            Interrupt::Cancel => RequestOutcome::AlreadySettled,
        },
    }
}

/// Why a refused interrupt was refused.
///
/// These are `const`s rather than inline literals because they are long enough
/// that an unbreakable line is a real possibility, and an unbreakable line is
/// the one thing that is known to put code outside the `cargo fmt --check` gate.
/// Both name the *consequence* rather than only the state, because "cannot
/// cancel" does not tell the caller whether their file is safe.
const REASON_INTERRUPT_COMMITTING: &str = "the download already passed every check and is \
                                        being written to its final location — the file is \
                                        complete, and interrupting now would either delete it \
                                        or leave nothing to resume from";
const REASON_INTERRUPT_TERMINAL: &str = "the download has already finished";

/// The error a refused interrupt reports.
fn interrupt_error(request: Interrupt, status: DownloadStatus) -> DownloaderError {
    let reason = match status {
        DownloadStatus::Committing => REASON_INTERRUPT_COMMITTING,
        _ => REASON_INTERRUPT_TERMINAL,
    };
    DownloaderError::InvalidState(format!(
        "cannot {} download in {status} state: {reason}",
        request.verb()
    ))
}

impl Downloader {
    /// Create a new downloader that writes files into `output_dir`.
    pub fn new(output_dir: PathBuf) -> Self {
        // Ensure staging directory exists
        let tmp_dir = output_dir.join(".tmp");
        let _ = std::fs::create_dir_all(&tmp_dir);
        // Only correct here, at construction: no job can exist yet, so every
        // claim found in there belongs to a previous process.
        sweep_stale_staging(&tmp_dir);

        Self {
            output_dir,
            active_downloads: Arc::new(RwLock::new(HashMap::new())),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            commit_gate: Arc::new(Mutex::new(())),
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

        let tmp_dir = self.output_dir.join(".tmp");
        // Propagated rather than ignored: without the directory there is no
        // staging file, and without one of those there is no way to claim a
        // name — so the job could only continue by picking one on trust.
        tokio::fs::create_dir_all(&tmp_dir)
            .await
            .map_err(DownloaderError::IoError)?;

        let claimed = reserve_output(
            &self.output_dir,
            &tmp_dir,
            &sanitize_filename(&req.title),
            &ext,
            id,
        )
        .await?;
        let OutputReservation {
            output_path,
            staging_path,
            public_name,
        } = claimed;

        let mut progress =
            DownloadProgress::with_id(id, req.stream_url.clone(), req.title.clone(), req.format);
        progress.platform = req.platform.clone();
        progress.total_bytes = req.total_bytes;
        progress.expected_duration_secs = req.expected_duration_secs;
        progress.status = DownloadStatus::Downloading;
        progress.output_path = Some(output_path.to_string_lossy().to_string());
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
                    title: req.title.clone(),
                    artist: req.artist,
                    album: req.album,
                    output_path,
                    staging_path,
                    public_name,
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
        let commit_gate = self.commit_gate.clone();

        let handle = tokio::spawn(async move {
            let job = {
                let jobs = jobs.read().await;
                match jobs.get(&id) {
                    Some(j) => j.clone(),
                    None => return,
                }
            };

            if let Err(e) =
                Self::run_stream(id, &job, start_byte, active.clone(), commit_gate).await
            {
                let url_host = job.stream_url.split('/').nth(2).unwrap_or("unknown");
                error!(download_id = %id, url_host = %url_host, url = %job.stream_url, start_byte = start_byte, error = %e, "Download failed — cleaning staging file and marking failed");
                warn!(download_id = %id, error = %e, "DIAGNOSTIC download_failed id={} host={} url={} error={}", id, url_host, job.stream_url, e);

                // Only this job's own bytes, and only while it still holds the
                // claim — after the rename the file is committed and must
                // survive its own job's failure.
                discard_owned_paths(&job.staging_path, &job.output_path).await;

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
        commit_gate: Arc<Mutex<()>>,
    ) -> Result<(), DownloaderError> {
        let host = job.stream_url.split('/').nth(2).unwrap_or("unknown");
        let url_snip = if job.stream_url.chars().count() > 160 {
            format!("{}…", job.stream_url.chars().take(160).collect::<String>())
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

            // An advertised length of zero describes no object, so no byte
            // count can ever satisfy it: the completion check below requires
            // `total > 0`, and "bytes still outstanding" requires
            // `total > current_downloaded`, which is false for zero. A
            // zero-byte body is not an error either, so the loop asked again and
            // again with nothing changing. Reject it here, before any request,
            // so the job ends with a reason instead of hanging. `total_bytes`
            // can reach zero from the resolver, from `clen` in the URL, or from
            // a `Content-Range`/`Content-Length` in a response, so the check
            // lives where all three are already in scope.
            if total_bytes == Some(0) {
                return Err(DownloaderError::DownloadFailed(format!(
                    "Server advertised a zero-length object [{host} url={url_snip}] — there is \
                     nothing to download, so the transfer was stopped \
                     (end_reason={end_reason}, chunk_iteration={chunk_iteration})"
                )));
            }

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
                    // No host named here on purpose. A previous version hardcoded
                    // `rr1---sn-gwpa-cived`, and the device report of 2026-09-29 showed
                    // why that is actively harmful: the refusing host was
                    // `rr1---sn-gwpa-cive7`, printed a few characters earlier in the same
                    // message. Two different hosts in one error sends whoever reads it
                    // after an edge that never refused us. The real host is already in
                    // the message prefix.
                    403 => " — 403 Forbidden: googlevideo rejected UA/Referer/Origin/PO-token or the URL expired",
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

            // Every response that delivered nothing has to consume retry
            // budget. Before `classify_response` the two zero-byte cases that
            // could not prove bytes were outstanding fell through to the
            // completion check, matched nothing there, and asked again — the
            // server being silent was read as "the object ended" when it
            // proved nothing of the kind.
            match classify_response(
                bytes_in_this_request,
                stream_interrupted,
                total_bytes,
                current_downloaded,
            ) {
                StreamStep::Progress => consecutive_errors = 0, // Successful progress made!
                StreamStep::Interrupted => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        return Err(DownloaderError::HttpError(format!(
                            "Stream interrupted and max consecutive errors ({MAX_CONSECUTIVE_ERRORS}) reached [{host}] ({current_downloaded}/{:?} bytes)",
                            total_bytes
                        )));
                    }
                    continue;
                }
                StreamStep::Short { advertised } => {
                    // The server closed the connection cleanly but still owes us
                    // bytes. That is NOT the end of the stream — resume via Range
                    // until the retry budget is spent, then fail honestly.
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        return Err(DownloaderError::DownloadFailed(format!(
                            "Incomplete download: server ended the stream at {current_downloaded} of {advertised} bytes after {MAX_CONSECUTIVE_ERRORS} resume attempts (end_reason={end_reason}, host={host}) — file not saved"
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
                StreamStep::NoProgress => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        return Err(DownloaderError::DownloadFailed(format!(
                            "No progress: the server ended the response without sending any bytes \
                             ({current_downloaded}/{:?} bytes) after {MAX_CONSECUTIVE_ERRORS} \
                             attempts (end_reason={end_reason}, host={host}) — file not saved",
                            total_bytes
                        )));
                    }
                    warn!(
                        download_id = %id,
                        downloaded = current_downloaded,
                        total = ?total_bytes,
                        consecutive_errors = consecutive_errors,
                        "Response delivered no bytes — treating it as a failed attempt and retrying"
                    );
                    continue;
                }
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
                // total_bytes is unknown, and this response delivered bytes.
                // A zero-byte response never reaches here: `classify_response`
                // returns `NoProgress` for it, which consumes retry budget
                // rather than treating silence as the end of the object.
                info!(
                    download_id = %id,
                    downloaded = current_downloaded,
                    "Chunk completed with unknown total_bytes; probing next range for remaining bytes"
                );
                continue;
            }
        }

        // ---- Completeness gate -------------------------------------------------
        // A decodable container header is NOT proof that the audio bytes
        // arrived: YouTube's m4a carries the full duration in its front `moov`
        // atom, so a half-downloaded file still "validates" and used to be
        // renamed + reported as completed (and played as silence afterwards).
        // Decide completeness from the byte count, never from metadata alone.
        // This MUST stay the true on-disk length, never `downloaded_bytes`.
        //
        // `range_topup::top_up` appends with `append(true)`, which writes at the
        // real end-of-file regardless of the `start` it is given, and it grants
        // a `200` response the "no range needed" exception when `start == 0`.
        // Both are only sound while `start` is the file's real length: a `200`
        // from byte 0 would otherwise be appended after existing bytes and
        // reproduce exactly the corruption the top-up validation exists to
        // prevent. `have` further down likewise advances by the clamped `added`,
        // not by the progress counter. Do not "simplify" this to the counter.
        //
        // A stat failure is a hard error rather than `unwrap_or(0)`: reporting
        // a non-empty file as empty would both re-admit that whole-object
        // append and skip the completeness checks below.
        let staged_bytes = match tokio::fs::metadata(&job.staging_path).await {
            Ok(m) => m.len(),
            Err(e) => {
                return Err(DownloaderError::IoError(std::io::Error::new(
                    e.kind(),
                    format!(
                        "{}: cannot stat the staged file to decide whether it is complete: {e}",
                        job.staging_path.display()
                    ),
                )));
            }
        };
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

        // Decoded-length gate. `lofty` reads the container header, which for a
        // YouTube MP4 advertises the FULL track length from the front `moov`
        // atom even when the media data stops early — so the check above passes
        // a file that only holds the first minute. The decoder's own duration
        // counts the samples that are really present (the same number the
        // player shows), and is the only trustworthy completeness signal.
        //
        // All three inspections happen in one `spawn_blocking` hop: reading the
        // container (up to 192 MB) and walking the decoded stream twice are
        // seconds of a Tokio worker each, and a download is not the only task on
        // that runtime. The decoder's verdict still decides *whether* to run
        // them; `acceptance` decides what they mean.
        //
        // The three arguments are bound to short names first. A `gather_forensics(…)`
        // call spelled out in full sits right on the `fn_call_width` boundary, and
        // rustfmt 1.63 and current stable disagree about how to break a method
        // chain that lands there — which is a `cargo fmt --check` failure that
        // only CI can see. Short names keep the call well inside the width.
        let staging = &job.staging_path;
        let ext = &job.ext;
        let dur = expected_duration_secs;
        let initial = gather_forensics(staging, ext, dur).await;
        if let Some(truncation) = initial.decoded.as_ref().err().cloned() {
            // A decoder's opinion of a file's length is not evidence. Real
            // device data (v2.6.44, track BElct8HWkp8) showed the opposite: a
            // 21.4 MB muxed file that holds 596 kbps x 287 s of media was
            // rejected because rodio reported 99 s for it. So ask the container
            // and the decoded content instead, and only treat the download as
            // short when they agree.
            let expected_secs = f64::from(expected_duration_secs.unwrap_or(0));
            warn!(
                download_id = %id,
                container = %initial.container.summary(),
                content = %initial.content.summary(),
                expected_duration = ?expected_duration_secs,
                "Decoder reported a short file; container and content were inspected"
            );

            // `latest` is what the failure below describes, so it is refreshed
            // after every append: the file changed, and facts about the bytes it
            // used to hold are not facts about the file. Reusing the report
            // gathered here would judge the pre-append state forever, which is
            // how the recovery path used to accept (or refuse) a file it had
            // never looked at again.
            let mut latest = initial;

            if acceptance(&latest, expected_secs) == Acceptance::Accept {
                // The decoder under-reported: keep the file. Its own duration is
                // the truth here, and the library scanner will store that.
                warn!(
                    download_id = %id,
                    container = %latest.container.summary(),
                    content = %latest.content.summary(),
                    "Container and decoded audio both cover the full track - keeping the file despite the decoder's short verdict"
                );
            } else {
                use super::range_topup::{url_param_str, MAX_TOPUP_ROUNDS, TOPUP_CHUNK_BYTES};
                let mut topup_log: Vec<String> = Vec::new();
                let mut last_topup_error: Option<String> = None;
                let mut have = staged_bytes;
                // A plain flag rather than `Option<ForensicReport>`: `latest` is
                // already the accepted report when this is set, and keeping it
                // owned means the failure below can describe the same facts
                // without a second copy.
                let mut recovered = false;
                for round in 1..=MAX_TOPUP_ROUNDS {
                    match super::range_topup::top_up(
                        &client,
                        job.headers.as_ref(),
                        &request_url,
                        &job.staging_path,
                        have,
                        TOPUP_CHUNK_BYTES,
                    )
                    .await
                    {
                        Ok(added) => {
                            have += added;
                            topup_log
                                .push(format!("round {round}: +{added} bytes (file now {have})"));
                            // The bytes are on disk, so every fact gathered
                            // before this append now describes a file that no
                            // longer exists. Re-derive all three, off the
                            // runtime thread, and judge the new state — the
                            // append is the only thing that can make a windowed
                            // object whole, so a pre-append verdict here would
                            // refuse a file the top-up just finished.
                            let report = gather_forensics(staging, ext, dur).await;
                            let verdict = acceptance(&report, expected_secs);
                            latest = report;
                            if let Acceptance::Reject(why) = verdict {
                                topup_log.push(format!("round {round}: still not whole: {why}"));
                                continue;
                            }
                            topup_log.push(format!(
                                "round {round}: the appended file is whole ({})",
                                latest.content.summary()
                            ));
                            recovered = true;
                            break;
                        }
                        Err(why) => {
                            last_topup_error = Some(why);
                            break;
                        }
                    }
                }
                if recovered {
                    warn!(
                        download_id = %id,
                        bytes = have,
                        measured = ?latest.content.measured_secs(),
                        container = %latest.container.summary(),
                        content = %latest.content.summary(),
                        host = %host,
                        "Range top-up completed the file; the appended bytes were re-verified against the container and the decoded audio"
                    );
                } else {
                    let have_now = tokio::fs::metadata(&job.staging_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(have);
                    error!(
                        download_id = %id,
                        error = %truncation,
                        downloaded = have_now,
                        total = ?total_bytes,
                        expected_duration = ?expected_duration_secs,
                        end_reason = %end_reason,
                        host = %host,
                        container = %latest.container.summary(),
                        content = %latest.content.summary(),
                        "Refusing to save a short download"
                    );
                    let itag =
                        url_param_str(&request_url, "itag").unwrap_or_else(|| "?".to_string());
                    let clen = extract_url_param_u64(&request_url, "clen").unwrap_or(0);
                    let topup_summary = if let Some(why) = last_topup_error {
                        why
                    } else if topup_log.is_empty() {
                        "no range top-up was attempted".to_string()
                    } else {
                        topup_log.join(" | ")
                    };
                    // Two very different problems produce the same decoder verdict,
                    // and the difference decides whether a retry can ever work:
                    //  * the container itself only describes a short track -> the
                    //    server deliberately sent a window and calls it the whole
                    //    object, so a different client is the only way out;
                    //  * the container describes the full track but bytes are
                    //    missing -> a genuinely interrupted transfer, which the
                    //    range top-up above just failed to complete.
                    //
                    // Read off `latest`, i.e. the state after the last append —
                    // describing the pre-append file here would report a byte
                    // count the user cannot go and look at. The threshold is the
                    // same `MIN_COVERAGE_RATIO` the gate above used, so the
                    // wording cannot drift away from the decision it explains.
                    let container = &latest.container;
                    let explanation = if container
                        .table_secs
                        .is_some_and(|t| t < expected_secs * MIN_COVERAGE_RATIO)
                    {
                        "The container itself only describes a short track, so the server sent a windowed object and reports it as complete (this is the SABR behaviour, not a broken transfer)"
                    } else if matches!(&container.verdict, Some(Verdict::Truncated { .. })) {
                        "The container describes the full track but the file is missing bytes the sample table references, so the transfer was interrupted"
                    } else {
                        "The container could not be parsed, so the decoder's verdict could not be checked"
                    };
                    let facts_summary = container.summary();
                    let content_summary = latest.content.summary();
                    cleanup_staging_file(&job.staging_path).await;
                    return Err(DownloaderError::DownloadFailed(format!(
                        "{truncation} [{explanation}. received {have_now} bytes of {clen} \
                         advertised (itag={itag}, host={host}, end_reason={end_reason}); \
                         {facts_summary}; {content_summary}; \
                         bytes after the window: {topup_summary}]"
                    )));
                }
            }
        }

        // -------------------------------------------------------------------
        // The commit boundary (DL-02)
        // -------------------------------------------------------------------
        //
        // Everything above this line can be thrown away. Everything below it
        // cannot, and the difference is three ordered statements inside one
        // hold of `commit_gate`:
        //
        //   1. `Committing` — the job is no longer interruptible
        //   2. the rename    — the bytes become the file the user will keep
        //   3. `Completed`   — the terminal transition, with the internal path
        //
        // 3 immediately after 2, and not at the end of the function, because
        // what follows is seconds of work (a cover-art fetch with a 20 s
        // timeout, an MP4 tag rewrite, a MediaStore insert) during which the
        // job used to sit in `Downloading` with its staging file already gone:
        // a cancel deleted a finished track, and a pause reported `Paused` over
        // a job with nothing left to resume from.
        //
        // Holding the gate across all three is what stops an interrupt from
        // landing *between* them. A request that acquires the gate is either
        // strictly before this hold or strictly after it, so the state it reads
        // is one of those two and the decision it makes is never about a
        // half-committed job. Everything after the gate is released is
        // best-effort work on a file that already exists and plays.
        let _commit = commit_gate.lock().await;

        {
            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.begin_commit();
            }
        }

        info!(
            download_id = %id,
            staging = %job.staging_path.display(),
            destination = %job.output_path.display(),
            "Verified staging file — committing to final destination"
        );

        if let Err(e) = tokio::fs::rename(&job.staging_path, &job.output_path).await {
            warn!(
                download_id = %id,
                error = %e,
                "tokio::fs::rename failed, falling back to copy + remove"
            );
            // Only reachable for a destination this job exclusively owns (see
            // `reserve_output`), so the copy overwrites nothing that matters.
            tokio::fs::copy(&job.staging_path, &job.output_path)
                .await
                .map_err(DownloaderError::IoError)?;
            let _ = tokio::fs::remove_file(&job.staging_path).await;
        }

        // Durable. Terminal before anything optional runs, so that a job can
        // never be reported as anything but completed once the file exists.
        {
            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                state.complete(job.output_path.to_string_lossy().to_string());
                state.note_post_commit();
            }
        }

        drop(_commit);

        // Save thumbnail sidecar if requested
        if let Some(thumb) = &job.thumbnail {
            Self::save_thumbnail(&client, thumb, &job.output_path).await;
        }

        // Write title/artist/album into the finished file so the library
        // scanner reads real metadata instead of the sanitized filename /
        // "Unknown Artist". Sibling of `save_thumbnail`: both attach metadata
        // to the same committed file, both are non-fatal.
        //
        // `spawn_blocking` because lofty's MP4 write rebuilds the metadata
        // atoms and rewrites the file, which is blocking I/O on a Tokio worker
        // (same reason as `validate_audio_file_async`). A tagging failure —
        // including a join failure — is only logged; the download stays
        // Completed and the file stays exactly as downloaded.
        let tag_path = job.output_path.clone();
        let tag_title = job.title.clone();
        let tag_artist = job.artist.clone();
        let tag_album = job.album.clone();
        let tagged = tokio::task::spawn_blocking(move || {
            super::tags::write_tags(
                &tag_path,
                &tag_title,
                tag_artist.as_deref(),
                tag_album.as_deref(),
            )
        })
        .await;
        match tagged {
            Ok(Ok(())) => info!(
                download_id = %id,
                path = %job.output_path.display(),
                "Wrote title/artist/album tags into downloaded file"
            ),
            Ok(Err(tag_err)) => warn!(
                download_id = %id,
                path = %job.output_path.display(),
                error = %tag_err,
                "Could not write tags into downloaded file (non-fatal — file kept untagged)"
            ),
            Err(join_err) => warn!(
                download_id = %id,
                path = %job.output_path.display(),
                error = %join_err,
                "Tagging task failed (non-fatal — file kept untagged)"
            ),
        }

        // Where the user can actually find the file. On Android this is the
        // public `Download/Auralis` copy when publication succeeded, and the
        // app-private path when it did not — the two are not interchangeable, and
        // the difference is what `publish_error` is for.
        let completion_path = Self::publish_public_copy(id, job, &active).await;

        {
            let mut guard = active.write().await;
            if let Some(state) = guard.get_mut(&id) {
                // The durable state was set immediately after the rename; this
                // only says the record is now the whole story, which is the
                // emitter's cue to send `download:completed` with the public
                // path and the publish reason on it.
                state.finish_post_commit();
            }
        }

        info!(
            download_id = %id,
            path = %completion_path,
            "Download complete and verified"
        );

        Ok(())
    }

    /// Fetch a thumbnail/cover URL and save it as a `<audio>.jpg` sidecar so the
    /// library scanner can associate it with the downloaded track. Non-fatal.
    async fn save_thumbnail(client: &reqwest::Client, url: &str, audio_path: &Path) {
        // `create` (truncate) rather than an exclusive create: a resumed job
        // legitimately re-fetches its cover art, and the name is already this
        // job's alone, so there is nothing here to protect against.
        let cover_path = sidecar_path(audio_path);
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

    /// Publish the finished file where the user can find it, and report the path
    /// they will find it at.
    ///
    /// Split out of `run_stream` because almost all of it is Android-only, and
    /// an `#[cfg]` block inline in the middle of the function left the
    /// surrounding code reading as though the completion path were mutated on
    /// every target — which is why it needed a
    /// `#[cfg_attr(…, allow(unused_mut))]` to compile at all. Two definitions
    /// rather than one function with a `cfg`'d block at each end of its body,
    /// because a tail expression that is only present on some targets is a
    /// shape rustc has to reason about, and this does not need that risk.
    ///
    /// Runs after the commit and outside `commit_gate`, because it can take
    /// seconds and an interrupt must not have to queue behind it. A job
    /// interrupted here is `Completed` with, at worst, a `publish_error` — the
    /// file exists and plays, and that is the better of the two outcomes.
    #[cfg(target_os = "android")]
    async fn publish_public_copy(
        id: Uuid,
        job: &DownloadJob,
        active: &RwLock<HashMap<Uuid, DownloadProgress>>,
    ) -> String {
        let should_publish = std::panic::catch_unwind(|| {
            crate::domain::models::Settings::load()
                .map(|s| s.downloads.use_system_downloads)
                .unwrap_or(true)
        })
        .unwrap_or(true);
        info!(
            download_id = %id,
            should_publish = should_publish,
            src = %job.output_path.display(),
            "MediaStore publish check (use_system_downloads, default true)"
        );
        if !should_publish {
            info!(download_id = %id, "Skipping MediaStore publish per use_system_downloads=false");
            return job.output_path.to_string_lossy().to_string();
        }

        // The public copy is named from the path handed to
        // `publish_to_downloads`, so hand it one that *reads* as the clean title
        // even when the internal file needed a dedup suffix. A hard link says
        // that without a second copy of the bytes; `None` means either the names
        // already agree or the link could not be made, and both fall back to
        // publishing the internal path.
        let link_dir = job.staging_path.parent().map(Path::to_path_buf);
        let linked = link_dir
            .as_deref()
            .and_then(|dir| link_under_public_name(&job.output_path, dir, &job.public_name));
        let publish_src = linked.clone().unwrap_or_else(|| job.output_path.clone());
        let result = publish_to_downloads(&publish_src);
        // The link is a presentation of the same bytes, not a second file the
        // library should see: it goes away with the call that needed it.
        if let Some(link) = linked {
            if let Err(e) = tokio::fs::remove_file(&link).await {
                debug!(path = %link.display(), error = %e, "Could not remove the public-name link");
            }
        }

        // Carried, not logged-and-dropped. The `Err` distinguishes a context
        // that could not be acquired from a ContentResolver that would not
        // return a collection, an insert that produced no row id, and a copy
        // that threw — all of which previously collapsed into one
        // indistinguishable "keeping internal path". On a release build this
        // string is the only surviving evidence, and the user cannot act on a
        // file they cannot find without it.
        match result {
            Ok(pub_path) => {
                let mut guard = active.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    // Keep internal path for library scan dedup, but surface public
                    // path so `download:completed` shows the Files-visible location.
                    state.output_path = Some(pub_path.clone());
                }
                // Log before the move: `info!` borrows `pub_path`, so this has
                // to precede the assignment that consumes it.
                info!(
                    download_id = %id,
                    public = %pub_path,
                    internal = %job.output_path.display(),
                    "Published download to Download/Auralis"
                );
                pub_path
            }
            Err(reason) => {
                let mut guard = active.write().await;
                if let Some(state) = guard.get_mut(&id) {
                    state.note_publish_error(reason.clone());
                }
                warn!(
                    download_id = %id,
                    src = %job.output_path.display(),
                    reason = %reason,
                    "MediaStore publish failed — file is app-private only"
                );
                job.output_path.to_string_lossy().to_string()
            }
        }
    }

    /// Off Android there is no public copy to make, so the internal path is the
    /// answer. See the Android definition for what this is for.
    #[cfg(not(target_os = "android"))]
    async fn publish_public_copy(
        id: Uuid,
        job: &DownloadJob,
        active: &RwLock<HashMap<Uuid, DownloadProgress>>,
    ) -> String {
        let _ = (id, active);
        job.output_path.to_string_lossy().to_string()
    }

    /// The current status of `id`.
    async fn read_status(&self, id: Uuid) -> Result<DownloadStatus, DownloaderError> {
        let downloads = self.active_downloads.read().await;
        let state = downloads
            .get(&id)
            .ok_or(DownloaderError::DownloadNotFound(id))?;
        Ok(state.status)
    }

    /// Pause an in-progress download by aborting its task and truncating the
    /// staging file to the last fully-written byte.
    pub async fn pause(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Pausing download");

        // The gate is taken *before* the status is read, and the status is what
        // decides whether the task gets aborted at all. Both this and `cancel`
        // used to abort first and read the state afterwards, so a request that
        // arrived during a commit killed the task mid-rename and only then
        // discovered it had no business doing so. Inside the gate a job is
        // either before the commit or after it, so the state read here is not
        // about to change under us.
        let _gate = self.commit_gate.lock().await;
        let status = self.read_status(id).await?;
        match classify_interrupt(Interrupt::Pause, status) {
            RequestOutcome::Apply => {}
            RequestOutcome::AlreadySettled => return Ok(()),
            RequestOutcome::Refuse => return Err(interrupt_error(Interrupt::Pause, status)),
        }

        // Await the aborted task before touching the staging file. Without this,
        // pause/cancel can race the writer and truncate bytes it is still using.
        // Take the handle out under the lock, then await it with the lock
        // released. `if let Some(h) = self.tasks.write().await.remove(&id)`
        // would keep the write guard alive until the end of the whole `if let`
        // (temporaries in the scrutinee live for the entire expression), so the
        // `await` below would run while holding it. The task being aborted may
        // itself want `tasks.write()` to deregister on completion, and then both
        // sides wait forever.
        let handle = self.tasks.write().await.remove(&id);
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }

        let staging_path = {
            let jobs = self.jobs.read().await;
            jobs.get(&id)
                .map(|job| job.staging_path.clone())
                .ok_or_else(|| {
                    DownloaderError::InvalidState(format!("download {id} has no job record"))
                })?
        };
        let downloaded = self
            .active_downloads
            .read()
            .await
            .get(&id)
            .map(|state| state.downloaded_bytes)
            .unwrap_or(0);

        if downloaded > 0 {
            if let Ok(file) = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&staging_path)
                .await
            {
                let _ = file.set_len(downloaded).await;
            }
        }

        let mut downloads = self.active_downloads.write().await;
        if let Some(state) = downloads.get_mut(&id) {
            state.status = DownloadStatus::Paused;
            state.updated_at = Utc::now();
        }

        Ok(())
    }

    /// Resume a paused download from the last committed byte via HTTP Range.
    pub async fn resume(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Resuming download");

        if !self.jobs.read().await.contains_key(&id) {
            return Err(DownloaderError::InvalidState(format!(
                "download {id} has no job record"
            )));
        }

        let start = {
            let mut downloads = self.active_downloads.write().await;
            let state = downloads
                .get_mut(&id)
                .ok_or(DownloaderError::DownloadNotFound(id))?;
            if state.status != DownloadStatus::Paused {
                return Err(DownloaderError::InvalidState(format!(
                    "cannot resume download in {} state",
                    state.status
                )));
            }
            let start = state.downloaded_bytes;
            state.status = DownloadStatus::Downloading;
            state.updated_at = Utc::now();
            start
        };

        self.spawn_stream(id, start).await;
        Ok(())
    }

    /// Cancel a download, killing its task and removing any staging and partial files.
    pub async fn cancel(&self, id: Uuid) -> Result<(), DownloaderError> {
        info!(download_id = %id, "Cancelling download");

        // Gate first, status second, abort third — the same reason as `pause`.
        // This used to abort the task first and only then read the status to
        // discover the job was already `Completed`, which is how a cancel
        // arriving during the commit could kill a job whose file was seconds
        // from being finished.
        let _gate = self.commit_gate.lock().await;
        let status = self.read_status(id).await?;
        match classify_interrupt(Interrupt::Cancel, status) {
            RequestOutcome::Apply => {}
            RequestOutcome::AlreadySettled => return Ok(()),
            RequestOutcome::Refuse => return Err(interrupt_error(Interrupt::Cancel, status)),
        }

        // Take the handle out under the lock, then await it with the lock
        // released. `if let Some(h) = self.tasks.write().await.remove(&id)`
        // would keep the write guard alive until the end of the whole `if let`
        // (temporaries in the scrutinee live for the entire expression), so the
        // `await` below would run while holding it. The task being aborted may
        // itself want `tasks.write()` to deregister on completion, and then both
        // sides wait forever.
        let handle = self.tasks.write().await.remove(&id);
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }

        let paths = {
            let jobs = self.jobs.read().await;
            jobs.get(&id)
                .map(|job| (job.staging_path.clone(), job.output_path.clone()))
        };

        if let Some((staging, output)) = paths {
            // The staging file is this job's claim on the output name, so the
            // output may only be deleted while the claim is still held — which,
            // at a state where cancelling is allowed, it is.
            discard_owned_paths(&staging, &output).await;
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
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // ---------------------------------------------------------------------
    // parse_content_range_total
    // ---------------------------------------------------------------------

    #[test]
    fn test_parse_content_range_total_valid() {
        assert_eq!(parse_content_range_total("bytes 0-99/1234"), Some(1234));
        assert_eq!(parse_content_range_total("bytes 100-199/5000"), Some(5000));
        assert_eq!(parse_content_range_total("bytes 0-0/1"), Some(1));
        assert_eq!(
            parse_content_range_total("bytes 500-600/18446744073709551615"),
            Some(18446744073709551615)
        );
    }

    #[test]
    fn test_parse_content_range_total_unsatisfied() {
        // Unsatisfied range: bytes */total
        assert_eq!(parse_content_range_total("bytes */1234"), Some(1234));
        assert_eq!(parse_content_range_total("bytes */0"), Some(0));
    }

    #[test]
    fn test_parse_content_range_total_invalid() {
        // Star total
        assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
        assert_eq!(parse_content_range_total("bytes */*"), None);

        // Empty string
        assert_eq!(parse_content_range_total(""), None);

        // No slash
        assert_eq!(parse_content_range_total("bytes 0-99"), None);

        // Non-numeric total
        assert_eq!(parse_content_range_total("bytes 0-99/abc"), None);

        // Garbage
        assert_eq!(parse_content_range_total("garbage"), None);
        assert_eq!(parse_content_range_total("/"), None);
        assert_eq!(parse_content_range_total("bytes /"), None);
    }

    // ---------------------------------------------------------------------
    // Fixtures for the completeness cross-check
    // ---------------------------------------------------------------------

    /// A temp file that removes itself, so a failing assertion cannot leave a
    /// stray directory behind for the next run to trip over.
    struct TempFile {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempFile {
        fn new(tag: &str, bytes: &[u8]) -> TempFile {
            let dir = std::env::temp_dir().join(format!(
                "auralis_dl_{tag}_{}_{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            let path = dir.join("probe.m4a");
            std::fs::write(&path, bytes).expect("write probe file");
            TempFile { dir, path }
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A `ContentFacts` shaped the way `forensics::inspect_content` builds it:
    /// the decoder's claim, plus the sample stream that was really iterated.
    ///
    /// Deliberately built with the same arithmetic as production, including
    /// deriving `audible_secs` from the audible sample count, so a test cannot
    /// assert a relationship the real code would not hold.
    fn content_facts(sample_rate: u32, total_samples: u64, audible_samples: u64) -> ContentFacts {
        ContentFacts {
            decoded_secs: Some(total_samples / u64::from(sample_rate.max(1))),
            audible_secs: Some(audible_samples as f64 / f64::from(sample_rate.max(1))),
            sample_rate,
            total_samples,
            audible_samples,
        }
    }

    /// An `MP4` whose single audio track declares a 4-minute sample table, and
    /// `media_bytes` of the media data that table points at.
    ///
    /// This is the only shape that can drive `Verdict::Complete` /
    /// `Verdict::Truncated` from a test: `forensics::inspect_container` answers
    /// `None` for anything it cannot walk as MP4, and the cross-check routes
    /// that case to the decoder instead — which would leave the container half
    /// of the gate untested.
    fn synthetic_mp4(media_bytes: usize) -> Vec<u8> {
        fn box_of(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = Vec::with_capacity(payload.len() + 8);
            out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            out
        }

        const CHUNK_OFFSET: u32 = 1000;
        const SAMPLE_BYTES: u32 = 4096;
        const SAMPLES: u32 = 2;
        // 4 minutes of audio at a 48 kHz timescale, i.e. a table that only a
        // complete file can satisfy.
        const TABLE_UNITS: u32 = 48_000 * 240;

        // stts: one run of `SAMPLES` samples of `TABLE_UNITS / SAMPLES` each.
        let mut stts = vec![0u8, 0, 0, 0];
        stts.extend_from_slice(&1u32.to_be_bytes());
        stts.extend_from_slice(&SAMPLES.to_be_bytes());
        stts.extend_from_slice(&(TABLE_UNITS / SAMPLES).to_be_bytes());

        // stsc: from chunk 1, `SAMPLES` samples per chunk.
        let mut stsc = vec![0u8, 0, 0, 0];
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&SAMPLES.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());

        // stsz: a uniform sample size, so no per-sample table is needed.
        let mut stsz = vec![0u8, 0, 0, 0];
        stsz.extend_from_slice(&SAMPLE_BYTES.to_be_bytes());
        stsz.extend_from_slice(&SAMPLES.to_be_bytes());

        // stco: one chunk, at CHUNK_OFFSET.
        let mut stco = vec![0u8, 0, 0, 0];
        stco.extend_from_slice(&1u32.to_be_bytes());
        stco.extend_from_slice(&CHUNK_OFFSET.to_be_bytes());

        let stbl = box_of(
            b"stbl",
            &[
                box_of(b"stts", &stts),
                box_of(b"stsc", &stsc),
                box_of(b"stsz", &stsz),
                box_of(b"stco", &stco),
            ]
            .concat(),
        );

        // mdhd: timescale 48000, duration TABLE_UNITS.
        let mut mdhd = vec![0u8, 0, 0, 0];
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&48_000u32.to_be_bytes());
        mdhd.extend_from_slice(&TABLE_UNITS.to_be_bytes());
        mdhd.extend_from_slice(&0u16.to_be_bytes());

        let minf = box_of(b"minf", &stbl);
        let mdia = box_of(b"mdia", &[box_of(b"mdhd", &mdhd), minf].concat());

        // hdlr: handler_type "soun" marks this as the audio track.
        let mut hdlr = vec![0u8, 0, 0, 0];
        hdlr.extend_from_slice(&0u32.to_be_bytes());
        hdlr.extend_from_slice(b"soun");

        let trak = box_of(b"trak", &[box_of(b"hdlr", &hdlr), mdia].concat());
        let moov = box_of(b"moov", &trak);

        let mut file = moov;
        while file.len() < CHUNK_OFFSET as usize {
            file.push(0);
        }
        file.resize(CHUNK_OFFSET as usize + media_bytes, 0u8);
        file
    }

    /// A report describing a container that is complete and whose table claims
    /// `table_secs`, plus the given decoded content.
    fn complete_container_report(table_secs: f64, content: ContentFacts) -> ForensicReport {
        ForensicReport {
            container: ContainerFacts {
                container: "mp4-stbl",
                audio_track_found: true,
                table_secs: Some(table_secs),
                declared_secs: Some(table_secs),
                verdict: Some(Verdict::Complete),
                ..Default::default()
            },
            content,
            decoded: Ok(None),
        }
    }

    // ---------------------------------------------------------------------
    // NEW-04 — the gate must compare length before audible position
    // ---------------------------------------------------------------------

    #[test]
    fn a_complete_track_that_ends_in_silence_is_accepted() {
        // The defect NEW-04 fixes. A 240 s track whose last stretch is silence:
        // every advertised byte arrived, the sample table covers the track, and
        // the decoded stream is the full 240 s. The old rule read
        // `near_expected(content.audible_secs)` — *audible audio for 90 % of the
        // track* — so anything past a 24 s outro was "unproven", cost four
        // pointless range-top-up rounds, and was ultimately refused.
        //
        // The fixtures below all sit well inside what the old rule rejected, so
        // each one is a case the old gate demonstrably refused and the new one
        // demonstrably keeps.
        for audible in [144.0_f64, 132.0] {
            let report = complete_container_report(
                240.0,
                content_facts(48_000, 240 * 48_000, (audible * 48_000.0) as u64),
            );
            assert!(
                audible < 240.0 * 0.9,
                "fixture with {audible}s of audible audio must be one the old 90% \
                 audible rule refused, or it proves nothing"
            );
            assert_eq!(
                acceptance(&report, 240.0),
                Acceptance::Accept,
                "a legitimate silent tail is not a truncation: {}",
                report.content.summary()
            );
        }

        // The exact old threshold, pinned because it is the boundary the old
        // rule was written around and the new one no longer consults.
        let boundary =
            complete_container_report(240.0, content_facts(48_000, 240 * 48_000, 216 * 48_000));
        assert_eq!(acceptance(&boundary, 240.0), Acceptance::Accept);
    }

    #[test]
    fn a_complete_container_overrules_a_short_decode() {
        // Deliberate inversion of the pre-§4.7.13 belief this test used to
        // pin (it asserted `Reject(REASON_SHORT_AUDIO)` on this exact shape).
        // The 2026-09-27 device download, `yF9nmg_jHNs`: all advertised bytes
        // received, container `table=216.3s`, `decoded=75s measured=54.4s`.
        // Measured on the dev box 2026-09-29, the transfer was always whole —
        // the decoder walked 25.2 % of the samples and stopped. A short
        // decode is non-evidence, so the container's proof stands.
        let report =
            complete_container_report(216.3, content_facts(48_000, 54 * 48_000, 54 * 48_000));
        assert_eq!(
            acceptance(&report, 216.0),
            Acceptance::Accept,
            "a complete container overrules a short decode: {}",
            report.content.summary()
        );
    }

    #[test]
    fn a_windowed_object_with_complete_bytes_is_still_rejected() {
        // The shape only the audible signal can catch, and the one that would
        // have been lost if acceptance had simply moved to `measured_secs`:
        // every advertised byte arrived, the sample table describes the whole
        // track, the decoded stream is the *right length* — because what came
        // after the cutoff was silence.
        //
        // 99 s of 287 s is the measured v2.6.44 device ratio, so this is not a
        // shape that was invented to make the test pass.
        let audible_secs = 99.0_f64;
        let report = complete_container_report(
            287.0,
            content_facts(44_100, 287 * 44_100, (audible_secs * 44_100.0) as u64),
        );
        let verdict = acceptance(&report, 287.0);
        assert_eq!(
            verdict,
            Acceptance::Reject(REASON_SILENT_TAIL),
            "a windowed object must keep being refused even when every byte arrived: {}",
            report.content.summary()
        );
    }

    #[test]
    fn a_short_decode_is_non_evidence_when_the_container_is_complete() {
        // The two signals must not be collapsed into one. A short decode is
        // non-evidence once the container has proven the file whole, so the
        // audible veto only fires when the decode walked the full track and
        // found early silence.
        let expected = 240.0;

        // Short decoded length, and audible is *fine* (the whole thing is
        // loud). Accepted — the container proved the file; the decoder
        // stopping early disproves nothing.
        let short_but_loud =
            complete_container_report(240.0, content_facts(48_000, 60 * 48_000, 60 * 48_000));
        assert_eq!(
            acceptance(&short_but_loud, expected),
            Acceptance::Accept,
            "a loud but short decode is non-evidence: {}",
            short_but_loud.content.summary()
        );

        // Short decoded length, and audible ends where the decode ended.
        // Accepted — audible == end-of-decode adds no information beyond
        // "the decoder stopped"; it is not observed trailing silence.
        let short_and_quiet =
            complete_container_report(240.0, content_facts(48_000, 60 * 48_000, 48 * 48_000));
        assert_eq!(
            acceptance(&short_and_quiet, expected),
            Acceptance::Accept,
            "audible-at-decode-end is not trailing silence: {}",
            short_and_quiet.content.summary()
        );

        // Long enough, and the audible tail is 10 % of the track: accepted, so
        // the veto does not fire on a fade-out.
        let long_fading =
            complete_container_report(240.0, content_facts(48_000, 240 * 48_000, 216 * 48_000));
        assert_eq!(acceptance(&long_fading, expected), Acceptance::Accept);

        // Long enough, and audio stops at 20 % of the track: vetoed.
        let long_but_windowed =
            complete_container_report(240.0, content_facts(48_000, 240 * 48_000, 48 * 48_000));
        assert!(
            matches!(
                acceptance(&long_but_windowed, expected),
                Acceptance::Reject(_)
            ),
            "a 20 % audible tail is not a fade-out: {}",
            long_but_windowed.content.summary()
        );
    }

    #[test]
    fn a_short_container_table_is_refused_before_anything_else() {
        // The server sent a window and called it the whole object: the sample
        // table itself describes a short track. Nothing about the decoded audio
        // can make that acceptable.
        let report =
            complete_container_report(75.0, content_facts(48_000, 75 * 48_000, 75 * 48_000));
        let verdict = acceptance(&report, 216.0);
        assert_eq!(verdict, Acceptance::Reject(REASON_SHORT_TABLE),);
    }

    #[test]
    fn missing_bytes_the_sample_table_references_are_refused() {
        // The one signal that is exact and needs no decoding.
        let report = ForensicReport {
            container: ContainerFacts {
                container: "mp4-stbl",
                audio_track_found: true,
                table_secs: Some(240.0),
                missing_bytes: Some(4096),
                verdict: Some(Verdict::Truncated {
                    missing_bytes: 4096,
                }),
                ..Default::default()
            },
            content: content_facts(48_000, 240 * 48_000, 240 * 48_000),
            decoded: Ok(None),
        };
        assert_eq!(
            acceptance(&report, 240.0),
            Acceptance::Reject(REASON_MISSING_BYTES),
        );
    }

    #[test]
    fn an_unparseable_container_falls_back_to_the_decoder_verbatim() {
        // `forensics::inspect_container` returns no verdict for a WebM, an
        // oversized file, or any layout it cannot walk, and documents that as
        // "keep your previous behaviour". The previous behaviour is the
        // decoder's verdict — so both directions are pinned, because silently
        // turning the fallback into a blanket accept (or a blanket reject)
        // would change what an Opus download is allowed to do.
        let undecodable = ForensicReport {
            container: ContainerFacts {
                container: "unknown",
                ..Default::default()
            },
            content: ContentFacts::default(),
            decoded: Ok(None),
        };
        assert_eq!(
            acceptance(&undecodable, 240.0),
            Acceptance::Accept,
            "no structural evidence and a decoder that is happy: previous behaviour kept"
        );

        let decoder_agrees_short = ForensicReport {
            decoded: Err("Truncated download: only 75s of 240s".to_string()),
            ..undecodable
        };
        assert_eq!(
            acceptance(&decoder_agrees_short, 240.0),
            Acceptance::Reject(REASON_UNPARSABLE),
        );
    }

    #[test]
    fn an_unmeasured_decoded_length_does_not_veto_a_complete_container() {
        // `ContentFacts::measured_secs()` is `None` exactly when no decoder could
        // be built. `forensics` is explicit that `None` "never means empty" and
        // that a caller must fall back to its other evidence — which is the
        // container's byte accounting, and that is exact.
        let report = complete_container_report(240.0, ContentFacts::default());
        assert_eq!(
            report.content.measured_secs(),
            None,
            "the fixture really is unmeasured"
        );
        assert_eq!(
            acceptance(&report, 240.0),
            Acceptance::Accept,
            "nothing was measured, so the length signal abstains: {}",
            report.content.summary()
        );

        // A *present but short* measurement is the same non-evidence: the
        // container proved the file, the decoder stopped early.
        let measured_short =
            complete_container_report(240.0, content_facts(48_000, 12_000, 12_000));
        assert_eq!(
            acceptance(&measured_short, 240.0),
            Acceptance::Accept,
            "a short decode does not veto a complete container: {}",
            measured_short.content.summary()
        );
    }

    // ---------------------------------------------------------------------
    // NEW-02 — the facts must describe the file as it is *now*
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn a_verdict_must_be_re_derived_after_an_append() {
        // NEW-02 in its exact shape: the container is consulted once, `top_up`
        // appends bytes, and the pre-append facts then judge the post-append
        // file. Here the only difference between the two states is the tail the
        // sample table still points at, and it is the difference between
        // refusing and saving.
        let short = synthetic_mp4(4096);
        let probe = TempFile::new("recompute", &short);
        let before = gather_forensics(&probe.path, "m4a", None).await;
        assert!(
            matches!(before.container.verdict, Some(Verdict::Truncated { .. })),
            "the fixture must start short: {}",
            before.container.summary()
        );
        assert!(
            matches!(acceptance(&before, 0.0), Acceptance::Reject(_)),
            "a file missing bytes its sample table references is not whole"
        );

        // The top-up: append exactly the bytes the sample table still points at.
        // The count is read out of the report rather than hard-coded, so this
        // does not depend on how `forensics` computes the media-data end — only
        // on the fact that it reports one, and reports the same one twice.
        let missing = before
            .container
            .audio_data_end
            .expect("a truncated sample table must say where its media data ends")
            .saturating_sub(before.container.size_bytes) as usize;
        assert!(missing > 0, "the fixture must have bytes outstanding");

        // A read-resize-write rather than `append` + `write_all`: the only thing
        // that matters here is the resulting file length, and this cannot leave a
        // handle open across the second gather.
        let mut with_tail = std::fs::read(&probe.path).expect("read the short file");
        with_tail.resize(with_tail.len() + missing, 0u8);
        std::fs::write(&probe.path, &with_tail).expect("append the missing media bytes");
        assert_eq!(
            with_tail.len() as u64,
            before.container.audio_data_end.expect("audio_data_end"),
            "the file must now reach the end the sample table points at"
        );

        let after = gather_forensics(&probe.path, "m4a", None).await;
        assert_eq!(
            after.container.verdict,
            Some(Verdict::Complete),
            "the appended file must read as complete: {}",
            after.container.summary()
        );
        assert_eq!(
            acceptance(&after, 0.0),
            Acceptance::Accept,
            "the same decision, applied to the re-derived facts, must accept: {}",
            after.container.summary()
        );

        // And the point of the whole exercise: the *stale* report still says
        // reject. Any implementation that hoisted the facts out of the top-up
        // loop would reach that answer and never recover, even after four rounds
        // that completed the file.
        assert!(
            matches!(acceptance(&before, 0.0), Acceptance::Reject(_)),
            "reusing the pre-append facts would refuse a file the append finished"
        );
        assert_ne!(
            before.container.size_bytes, after.container.size_bytes,
            "a re-derived report must be about the new file, not the old one"
        );
    }

    #[tokio::test]
    async fn gather_forensics_reads_the_file_rather_than_caching_it() {
        // The mechanism NEW-02 leans on, tested on its own: two gathers of the
        // same path must differ once the file changes. A gatherer that memoised,
        // or that snapshotted the size once, would pass the cross-check test
        // above by accident.
        let probe = TempFile::new("fresh", b"PROBE-BODY");
        let first = gather_forensics(&probe.path, "m4a", None).await;
        assert_eq!(first.container.size_bytes, 10);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&probe.path)
            .expect("open for append");
        file.write_all(b"-AND-MORE").expect("append");
        drop(file);

        let second = gather_forensics(&probe.path, "m4a", None).await;
        assert_eq!(
            second.container.size_bytes, 19,
            "the second gather must see the appended bytes"
        );
    }

    // ---------------------------------------------------------------------
    // NEW-03 — the inspections must not run on the runtime thread
    // ---------------------------------------------------------------------

    #[test]
    fn the_three_inspections_exist_in_exactly_one_place_and_it_is_blocking() {
        // "Off the Tokio runtime" is a property of *where the call is written*,
        // and there is no runtime signal a unit test can assert on directly: a
        // blocking call on a worker thread still returns exactly the right
        // answer. So this pins the shape the property depends on.
        //
        // It exists because the fix is one deleted `spawn_blocking` away from
        // regressing, and because `inspect_container` reads up to 192 MB into
        // memory — a regression here is a multi-second stall of every other task
        // on the runtime, with no error and no failing test anywhere else.
        // `include_str!`, not `concat!`. `concat!` produced a *path*, and every
        // search below ran against those 52 characters: `test_module` was always
        // `None`, so the slice was the path itself and each `matches(...).count()`
        // was 0. The pre-existing NEW-03 test had this shape and could not pass
        // — the marker assertion below is what it tripped over, which is the one
        // place the mistake was visible.
        const SOURCE: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/infrastructure/media/downloader.rs"
        ));

        // The test module holds the very strings it searches for, so counting
        // them in the whole file would count this test. Only the code above the
        // test module is production.
        let test_module = SOURCE.find("\n#[cfg(test)]\nmod tests {");
        let production = match test_module {
            Some(at) => &SOURCE[..at],
            None => SOURCE,
        };
        assert!(
            test_module.is_some(),
            "the test module marker must be findable, or this test is scanning itself"
        );

        for (name, needle) in [
            ("inspect_container", "inspect_container("),
            ("inspect_content", "inspect_content("),
            ("verify_decoded_duration", "verify_decoded_duration("),
        ] {
            let occurrences = production.matches(needle).count();
            assert_eq!(
                occurrences, 1,
                "{name} must be called from exactly one place, the blocking hop, \
                 but production code calls it {occurrences} times — an inline \
                 call has crept back in"
            );
        }

        // The one place must be a `spawn_blocking` closure, and it must be
        // `gather_forensics`, which is what `run_stream` awaits.
        let hop_start = production
            .find("async fn gather_forensics(")
            .expect("gather_forensics must exist");
        let hop_end = hop_start
            + production[hop_start..]
                .find("\n/// What one completed HTTP response")
                .expect("gather_forensics must be followed by the next item");
        let hop = &production[hop_start..hop_end];
        assert!(
            hop.contains("spawn_blocking"),
            "the single call site must be inside a spawn_blocking hop"
        );
        // `run_stream` must reach the forensics through the hop, and it must do
        // so at the *post-append* site too — that second call is NEW-02, and
        // counting them keeps the two fixes from being unlinked later.
        //
        // The slice is `run_stream`'s own body, from its signature to the next
        // method's doc comment. It cannot stop at the hop: `gather_forensics` is
        // defined far *above* `run_stream`, and the two call sites sit deep
        // inside a function that is itself several hundred lines long.
        let stream_start = production
            .find("    async fn run_stream(")
            .expect("run_stream must exist");
        let stream_end = stream_start
            + production[stream_start..]
                .find("\n    /// Fetch a thumbnail/cover URL")
                .expect("run_stream must be followed by save_thumbnail");
        let stream = &production[stream_start..stream_end];
        assert_eq!(
            stream
                .matches("gather_forensics(staging, ext, dur).await")
                .count(),
            2,
            "run_stream must call the blocking hop once for the initial facts and \
             once more after each append"
        );
    }

    // ---------------------------------------------------------------------
    // DL-05 — a zero advertised length must terminate the job
    // ---------------------------------------------------------------------

    #[test]
    fn a_zero_advertised_length_is_never_a_completed_response() {
        // The exact state that used to loop forever: with `Some(0)`, the
        // "all bytes received" test needs `total > 0` and the "bytes still
        // outstanding" test needs `total > current`, so a zero-byte clean EOF
        // satisfied neither and the loop asked again with nothing changed.
        assert_eq!(
            classify_response(0, false, Some(0), 0),
            StreamStep::NoProgress,
            "a zero-length object owes nothing and delivered nothing: that is not progress"
        );
        assert_eq!(
            classify_response(0, false, Some(0), 12_345),
            StreamStep::NoProgress,
            "the same holds once bytes are held, because 0 > 12345 is false"
        );
    }

    #[test]
    fn every_response_that_delivered_nothing_consumes_retry_budget() {
        assert_eq!(
            classify_response(4096, false, Some(10_000), 4096),
            StreamStep::Progress,
            "bytes arrived: the budget resets"
        );
        assert_eq!(
            classify_response(4096, true, Some(10_000), 4096),
            StreamStep::Progress,
            "bytes that did arrive are progress even if the read then failed, \
             because the next request resumes from the real on-disk length"
        );
        assert_eq!(
            classify_response(0, true, None, 4096),
            StreamStep::Interrupted,
            "a stalled read is an error"
        );
        assert_eq!(
            classify_response(0, false, Some(10_000), 4096),
            StreamStep::Short { advertised: 10_000 },
            "a clean early end with bytes provably outstanding"
        );
        // The two cases that used to fall through every branch and be treated
        // as a finished stream.
        assert_eq!(
            classify_response(0, false, None, 4096),
            StreamStep::NoProgress,
            "an unknown length plus silence is not the end of the object"
        );
        assert_eq!(
            classify_response(0, false, Some(4096), 4096),
            StreamStep::NoProgress,
            "a length already satisfied plus silence is not the end of the object"
        );
    }

    // ---------------------------------------------------------------------
    // DL-01 — one job, one output name
    // ---------------------------------------------------------------------

    /// An output dir plus its staging dir, cleaned up with the test.
    struct ReservationDir {
        dir: PathBuf,
        output: PathBuf,
        staging: PathBuf,
    }

    impl ReservationDir {
        fn new(tag: &str) -> ReservationDir {
            let dir = std::env::temp_dir().join(format!("auralis_own_{tag}_{}", Uuid::new_v4()));
            let output = dir.join("downloads");
            let staging = output.join(".tmp");
            std::fs::create_dir_all(&staging).expect("create staging dir");
            ReservationDir {
                dir,
                output,
                staging,
            }
        }
    }

    impl Drop for ReservationDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn two_downloads_of_one_title_are_never_given_the_same_output() {
        // The defect: the name used to be chosen with `if path.exists()`, and
        // two jobs with the same title both saw "free" and were handed the same
        // path — so one of them deleted the other's finished file, and two that
        // both succeeded overwrote each other.
        let area = ReservationDir::new("same_title");
        let first = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("first download must be able to claim the clean name");
        let second = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("second download must fall through to a dedup name");

        assert_ne!(
            first.output_path, second.output_path,
            "two live jobs must not share an output path"
        );
        assert_ne!(
            first.staging_path, second.staging_path,
            "and therefore not share the claim that makes it exclusive"
        );
        assert!(
            first.staging_path.exists() && second.staging_path.exists(),
            "both claims are held until their own job releases them"
        );
    }

    #[tokio::test]
    async fn a_claim_is_exclusive_even_for_the_same_job_id() {
        // Belt and braces against the list, not the filesystem: the fallback
        // names are derived from the job id, so one job asking twice proposes
        // the same candidate twice and the exclusive create has to be what stops
        // it. A "fix" that only deduplicated the candidate list would pass the
        // test above and fail this one.
        let area = ReservationDir::new("same_id");
        let id = Uuid::new_v4();
        let first = reserve_output(&area.output, &area.staging, "Song", "m4a", id)
            .await
            .expect("first claim");
        let second = reserve_output(&area.output, &area.staging, "Song", "m4a", id)
            .await
            .expect("second claim must not reuse the name the first holds");
        assert_ne!(first.output_path, second.output_path);
    }

    #[tokio::test]
    async fn an_existing_file_is_never_claimed_or_overwritten() {
        // A file from an earlier run must be neither adopted nor clobbered: the
        // old code only ever *renamed over* it when the dedup name was also
        // taken, which is the other half of the same data loss.
        let area = ReservationDir::new("existing");
        let existing = area.output.join("Song.m4a");
        std::fs::write(&existing, b"not ours").expect("seed existing file");

        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("a dedup name is available");
        assert_ne!(
            claimed.output_path, existing,
            "a name already holding a file cannot be claimed"
        );
        assert_eq!(
            std::fs::read(&existing).expect("existing file still readable"),
            b"not ours",
            "and the file an earlier download wrote is untouched"
        );
    }

    #[tokio::test]
    async fn the_public_name_stays_clean_however_the_internal_one_did_not() {
        // Defect 3: the dedup suffix exists so two *internal* files can coexist.
        // Showing it to the owner in `Download/Auralis/` tells them they
        // downloaded the same track twice when they did not.
        let area = ReservationDir::new("public_name");
        std::fs::write(area.output.join("Song.m4a"), b"earlier").expect("seed earlier download");

        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("claim a dedup name");
        let internal = claimed
            .output_path
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .to_string();
        assert_ne!(
            internal, "Song.m4a",
            "precondition: the internal name is suffixed"
        );
        assert_eq!(
            claimed.public_name, "Song.m4a",
            "the public name is the clean title, not the internal one"
        );
    }

    #[tokio::test]
    async fn a_stale_cover_sidecar_is_cleared_rather_than_inherited() {
        // The sidecar is a separate file derived from the same name, so it
        // carries its own hazard: an earlier download's artwork can outlive its
        // audio, and then a download with no thumbnail of its own leaves the old
        // cover art sitting next to the new file, attached to the wrong track.
        let area = ReservationDir::new("stale_cover");
        let cover = area.output.join("Song.jpg");
        std::fs::write(&cover, b"stale artwork").expect("seed stale sidecar");

        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("claim the clean name");
        let name = claimed
            .output_path
            .file_name()
            .expect("a file name")
            .to_string_lossy();
        assert_eq!(name, "Song.m4a");
        assert!(
            !cover.exists(),
            "a sidecar whose audio is gone must not survive into the new download"
        );
    }

    #[tokio::test]
    async fn a_committed_file_outlives_its_own_jobs_failure() {
        // `discard_owned_paths` is what a failing job and a cancelling one both
        // call, and it is the only thing allowed to delete an output path. The
        // rename consumes the claim, so after a commit it must decline to touch
        // the file: that is the completed download, and the "just clean up the
        // output path" instinct is what deleted it.
        let area = ReservationDir::new("committed");
        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("claim");
        std::fs::write(&claimed.staging_path, b"verified bytes").expect("write staging bytes");
        // The commit.
        std::fs::rename(&claimed.staging_path, &claimed.output_path).expect("commit");

        discard_owned_paths(&claimed.staging_path, &claimed.output_path).await;

        assert!(
            claimed.output_path.exists(),
            "a job that has already committed must not delete its own finished file"
        );
        assert!(
            !claimed.staging_path.exists(),
            "the claim is spent either way"
        );
    }

    #[tokio::test]
    async fn a_partial_file_is_removed_by_the_job_that_owns_it() {
        // The other half, so the guard above cannot be satisfied by simply never
        // deleting anything: while the claim is held the output is this job's
        // own partial work, and a failure must clear it.
        let area = ReservationDir::new("partial");
        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("claim");
        std::fs::write(&claimed.staging_path, b"half a track").expect("write staging bytes");
        std::fs::write(&claimed.output_path, b"leftover").expect("write stray output");
        std::fs::write(sidecar_path(&claimed.output_path), b"art").expect("write stray cover");

        discard_owned_paths(&claimed.staging_path, &claimed.output_path).await;

        assert!(!claimed.output_path.exists(), "partial output removed");
        assert!(
            !sidecar_path(&claimed.output_path).exists(),
            "and its cover sidecar with it"
        );
        assert!(!claimed.staging_path.exists(), "the claim is released");
    }

    #[test]
    fn a_claim_left_by_a_dead_process_is_swept_at_startup() {
        // A staging file *is* a claim, so a crash part way through a download
        // would hold that name until the user cleared the directory by hand — and
        // every later download of that title would be pushed onto a dedup
        // suffix for no reason.
        let area = ReservationDir::new("sweep");
        std::fs::write(area.staging.join("Song.m4a.part"), b"interrupted").expect("seed claim");
        std::fs::write(area.staging.join("Song.m4a"), b"stranded link").expect("seed link");
        std::fs::write(area.output.join("Song.m4a"), b"committed").expect("seed a real file");

        sweep_stale_staging(&area.staging);

        assert!(
            area.staging
                .read_dir()
                .expect("read staging")
                .next()
                .is_none(),
            "the staging directory is scratch and must be empty at startup"
        );
        assert!(
            area.output.join("Song.m4a").exists(),
            "and a committed file is not scratch: the sweep must not reach it"
        );
    }

    #[test]
    fn the_dedup_fallback_names_are_distinct_from_each_other() {
        // The reservation is what makes the choice safe, but a list that offered
        // the same name twice would make every collision look like a bug in the
        // log — and the third attempt onwards is not a shape any test of
        // behaviour would reach.
        let id = Uuid::new_v4();
        let names = candidate_file_names("Song", "m4a", id);
        assert_eq!(names.len(), RESERVE_ATTEMPTS);
        assert_eq!(names[0], "Song.m4a", "the clean name is always tried first");
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "every fallback must be distinct");
    }

    // ---------------------------------------------------------------------
    // DL-02 — the commit boundary
    // ---------------------------------------------------------------------

    #[test]
    fn the_interrupt_table_refuses_everything_after_verification() {
        // The whole table, because the interesting cells are the ones a
        // hand-written test would leave out. `Committing` is the new row: a
        // cancel there used to delete a finished file, and a pause there used to
        // report `Paused` over a job whose staging file had just been renamed
        // away.
        let all = [
            DownloadStatus::Queued,
            DownloadStatus::Downloading,
            DownloadStatus::Paused,
            DownloadStatus::Committing,
            DownloadStatus::Completed,
            DownloadStatus::Failed,
            DownloadStatus::Cancelled,
        ];
        for status in all {
            let pause = classify_interrupt(Interrupt::Pause, status);
            let cancel = classify_interrupt(Interrupt::Cancel, status);
            // One arm per state rather than grouped or-patterns: a grouped arm
            // whose body needs a block is a formatting decision the two rustfmt
            // versions in this project disagree about, and the table is the
            // thing under test.
            let expected = match status {
                DownloadStatus::Queued => (RequestOutcome::Apply, RequestOutcome::Apply),
                DownloadStatus::Downloading => (RequestOutcome::Apply, RequestOutcome::Apply),
                DownloadStatus::Paused => (RequestOutcome::AlreadySettled, RequestOutcome::Apply),
                DownloadStatus::Committing => (RequestOutcome::Refuse, RequestOutcome::Refuse),
                DownloadStatus::Completed => (RequestOutcome::Refuse, RequestOutcome::Refuse),
                DownloadStatus::Failed => (RequestOutcome::Refuse, RequestOutcome::Refuse),
                DownloadStatus::Cancelled => {
                    (RequestOutcome::Refuse, RequestOutcome::AlreadySettled)
                }
            };
            assert_eq!(
                (pause, cancel),
                expected,
                "pause/cancel of a {status} download"
            );
        }
    }

    #[test]
    fn a_refusal_names_the_state_and_the_consequence() {
        let error = interrupt_error(Interrupt::Cancel, DownloadStatus::Committing);
        let message = error.to_string();
        assert!(message.contains("cancel"), "says which request: {message}");
        assert!(
            message.contains("committing"),
            "says which state: {message}"
        );
        assert!(
            message.contains("complete") && message.contains("resume"),
            "and says what the user stands to lose, which is the part that was \
             missing: {message}"
        );
    }

    /// A downloader with one job already registered in `status`, holding a claim
    /// on an output name, and no live task.
    async fn downloader_with_job_in(
        dir: &Path,
        status: DownloadStatus,
    ) -> (Downloader, Uuid, OutputReservation) {
        let staging = dir.join(".tmp");
        std::fs::create_dir_all(&staging).expect("create staging dir");
        let downloader = Downloader::new(dir.to_path_buf());
        let id = Uuid::new_v4();
        let claimed = reserve_output(dir, &staging, "Song", "m4a", id)
            .await
            .expect("claim the output name");
        std::fs::write(&claimed.output_path, b"a finished track").expect("write the output");
        let job = DownloadJob {
            stream_url: "https://example.com/song.m4a".to_string(),
            title: "Song".to_string(),
            artist: None,
            album: None,
            output_path: claimed.output_path.clone(),
            staging_path: claimed.staging_path.clone(),
            public_name: claimed.public_name.clone(),
            thumbnail: None,
            headers: None,
            expected_duration_secs: None,
            total_bytes: None,
            format: AudioFormat::M4a,
            ext: "m4a".to_string(),
        };
        let mut progress =
            DownloadProgress::with_id(id, job.stream_url.clone(), job.title.clone(), job.format);
        progress.status = status;
        downloader.jobs.write().await.insert(id, job);
        downloader
            .active_downloads
            .write()
            .await
            .insert(id, progress);
        (downloader, id, claimed)
    }

    #[tokio::test]
    async fn a_committing_job_refuses_to_pause_or_cancel_and_keeps_its_file() {
        // The behavioural half of DL-02, on the real `pause`/`cancel` rather than
        // on the table they consult: a refusal that still deleted the file would
        // satisfy a table-only test.
        let dir = std::env::temp_dir().join(format!("auralis_commit_{}", Uuid::new_v4()));
        let (downloader, id, claimed) =
            downloader_with_job_in(&dir, DownloadStatus::Committing).await;

        let pause = downloader.pause(id).await;
        let cancel = downloader.cancel(id).await;

        assert!(
            matches!(pause, Err(DownloaderError::InvalidState(_))),
            "pause during the commit must be refused, got {pause:?}"
        );
        assert!(
            matches!(cancel, Err(DownloaderError::InvalidState(_))),
            "cancel during the commit must be refused, got {cancel:?}"
        );
        assert!(
            claimed.output_path.exists(),
            "and a refused request must not have deleted the file"
        );
        assert_eq!(
            downloader.read_status(id).await.expect("still tracked"),
            DownloadStatus::Committing,
            "a refused request must not have moved the job either"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_committed_job_is_still_untouchable_and_reports_its_state() {
        // The guard DL-02 was built on, pinned so a later edit cannot quietly
        // narrow it to `Committing` only.
        let dir = std::env::temp_dir().join(format!("auralis_done_{}", Uuid::new_v4()));
        let (downloader, id, claimed) =
            downloader_with_job_in(&dir, DownloadStatus::Completed).await;

        assert!(matches!(
            downloader.cancel(id).await,
            Err(DownloaderError::InvalidState(_))
        ));
        assert!(matches!(
            downloader.pause(id).await,
            Err(DownloaderError::InvalidState(_))
        ));
        assert!(claimed.output_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cancelled_job_still_deletes_its_own_partial_bytes() {
        // The other direction: `Committing` must not turn cancel into a no-op.
        let dir = std::env::temp_dir().join(format!("auralis_cancel_{}", Uuid::new_v4()));
        let (downloader, id, claimed) = downloader_with_job_in(&dir, DownloadStatus::Paused).await;

        downloader
            .cancel(id)
            .await
            .expect("a paused job may be cancelled");

        assert!(
            !claimed.output_path.exists(),
            "an uncommitted job's own bytes are removed on cancel"
        );
        assert_eq!(
            downloader.read_status(id).await.expect("still tracked"),
            DownloadStatus::Cancelled
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_commit_boundary_is_ordered_and_held() {
        // The ordering DL-02 is about cannot be observed from outside without a
        // real 4-minute download and a CD-R, and a test that asserted a
        // reimplementation of it would pass no matter what the shipped code did —
        // which is how `pot-for-TV` shipped a live 403. So this reads the real
        // source and checks the order the file is in.
        // `include_str!`, not `concat!`. `concat!` produced a *path*, and every
        // search below ran against those 52 characters: `test_module` was always
        // `None`, so the slice was the path itself and each `matches(...).count()`
        // was 0. The pre-existing NEW-03 test had this shape and could not pass
        // — the marker assertion below is what it tripped over, which is the one
        // place the mistake was visible.
        const SOURCE: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/infrastructure/media/downloader.rs"
        ));
        let test_module = SOURCE.find("\n#[cfg(test)]\nmod tests {");
        let production = match test_module {
            Some(at) => &SOURCE[..at],
            None => SOURCE,
        };
        let start = production
            .find("    async fn run_stream(")
            .expect("run_stream must exist");
        let body = &production[start..];

        let at = |needle: &str| {
            body.find(needle)
                .unwrap_or_else(|| panic!("{needle} must appear in run_stream"))
        };
        let gate = at("let _commit = commit_gate.lock().await;");
        let committing = at("state.begin_commit();");
        let rename = at("tokio::fs::rename(&job.staging_path, &job.output_path)");
        let completed = at("state.complete(job.output_path.to_string_lossy().to_string());");
        let released = at("drop(_commit);");
        let cover = at("Self::save_thumbnail(&client, thumb, &job.output_path).await;");
        let publish = at("Self::publish_public_copy(id, job, &active).await;");
        let settled = at("state.finish_post_commit();");

        assert!(
            gate < committing && committing < rename && rename < completed,
            "the gate is taken, then Committing, then the rename, then Completed"
        );
        assert!(
            completed < released && released < cover && cover < publish && publish < settled,
            "Completed is terminal immediately after the rename, and everything \
             that can improve the file happens after it"
        );
    }

    #[test]
    fn an_interrupt_is_decided_before_the_task_is_aborted() {
        // The table above decides whether a request may be *obeyed*; it cannot
        // decide whether the task is killed first. That ordering is the other
        // half of DL-02 and it is not observable from the state table, because
        // a request that reads `Downloading` and *then* aborts has already done
        // the damage by the time the table gets a say — which is exactly what
        // `cancel` used to do, killing the task mid-rename and only then
        // discovering the job was `Completed` and declining to delete anything.
        //
        // So the order is pinned in the source. A test that reimplemented the
        // order would pass whatever the shipped code did, which is how a
        // regression here once shipped as a live defect.
        // `include_str!`, not `concat!`. `concat!` produced a *path*, and every
        // search below ran against those 52 characters: `test_module` was always
        // `None`, so the slice was the path itself and each `matches(...).count()`
        // was 0. The pre-existing NEW-03 test had this shape and could not pass
        // — the marker assertion below is what it tripped over, which is the one
        // place the mistake was visible.
        const SOURCE: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/infrastructure/media/downloader.rs"
        ));
        let test_module = SOURCE.find("\n#[cfg(test)]\nmod tests {");
        let production = match test_module {
            Some(at) => &SOURCE[..at],
            None => SOURCE,
        };

        let pause = span_of(
            production,
            "    pub async fn pause(",
            "    /// Resume a paused download",
            "pause",
        );
        let cancel = span_of(
            production,
            "    pub async fn cancel(",
            "    /// Prune finished, failed, and cancelled download records",
            "cancel",
        );

        for (name, body) in [("pause", pause), ("cancel", cancel)] {
            let at = |needle: &str| {
                body.find(needle)
                    .unwrap_or_else(|| panic!("{name} must contain {needle:?}"))
            };
            let gate = at("self.commit_gate.lock().await");
            let status = at("self.read_status(id).await?");
            let verdict = at("classify_interrupt");
            let abort = at("handle.abort()");
            assert!(
                gate < status && status < verdict && verdict < abort,
                "{name}: the gate is taken, the state read, the verdict taken, \
                 and only then is the task aborted"
            );
        }

        // The rest of the ordering. `cancel` deletes, so the deletion has to come
        // after the abort has been awaited, and `pause` must never delete at all
        // — a pause that removed bytes would turn "stop here" into "lose this".
        // And neither may touch the staging file before the aborted task has
        // actually stopped, which is the race the code has always guarded.
        let cancel_delete = cancel
            .find("discard_owned_paths")
            .expect("cancel must clean up through discard_owned_paths");
        assert!(
            cancel.find("handle.abort()").expect("cancel aborts") < cancel_delete,
            "cancel: the files are touched only after the task is stopped"
        );
        assert!(
            !pause.contains("discard_owned_paths") && !pause.contains("remove_file"),
            "pause: stopping a download must never delete anything"
        );
        let awaited = pause
            .find("let _ = handle.await;")
            .expect("pause must await the aborted task");
        assert!(
            awaited < pause.find("set_len").expect("pause truncates what it owns"),
            "pause: the staging file is truncated only once the writer has stopped"
        );
    }

    /// The text of one `fn`/`async fn` in the production slice, by its signature
    /// and whatever doc comment follows it.
    fn span_of<'a>(production: &'a str, start: &str, end: &str, what: &str) -> &'a str {
        let from = production
            .find(start)
            .unwrap_or_else(|| panic!("{what} must exist in the production source"));
        let to = production
            .find(end)
            .unwrap_or_else(|| panic!("{what} must be followed by {end:?}"));
        assert!(to > from, "{what} must precede {end:?}");
        &production[from..to]
    }

    // ---------------------------------------------------------------------
    // Defect 3 — the public name
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn a_deduped_file_can_be_published_under_its_clean_name() {
        // `publish_to_downloads` derives the MediaStore display name from the
        // path it is given, so the only lever this side owns is the path. A hard
        // link is what carries the clean name without a second copy of the
        // bytes, and without anything having to resolve at read time.
        //
        // The link goes in the staging directory precisely because the clean
        // name is *taken* next to the file: that is the only reason the internal
        // name was deduped. Placing it beside the file fails with EEXIST in the
        // one case it exists for — which is what the first version of this test
        // did, and the failure it produced is the reason the link is here.
        let area = ReservationDir::new("public_link");
        std::fs::write(area.output.join("Song.m4a"), b"earlier").expect("seed earlier download");
        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("claim a dedup name");
        std::fs::write(&claimed.output_path, b"the real bytes").expect("write the finished file");

        let link =
            link_under_public_name(&claimed.output_path, &area.staging, &claimed.public_name)
                .expect("a link must be available for a deduped name");

        assert_eq!(
            link.file_name().expect("a file name").to_string_lossy(),
            "Song.m4a",
            "the path handed to the publisher is named after the clean title"
        );
        assert_eq!(
            std::fs::read(&link).expect("link readable"),
            b"the real bytes",
            "and it carries the finished file's bytes, not a copy of its own"
        );
        assert!(
            claimed.output_path.exists(),
            "and the internal file is untouched by the presentation"
        );
    }

    #[tokio::test]
    async fn no_link_is_made_when_the_names_already_agree() {
        // The overwhelmingly common case. Returning a link here would put a
        // second copy of every ordinary download in the library folder for the
        // duration of the publish.
        //
        // The finished file has to exist before this is meaningful: a link
        // cannot be made from a path that is not there, so a test that asserted
        // `None` on a merely-*reserved* name would pass with the guard deleted —
        // which is exactly what the first version of it did, and mutation
        // testing found it.
        let area = ReservationDir::new("public_link_same");
        let claimed = reserve_output(&area.output, &area.staging, "Song", "m4a", Uuid::new_v4())
            .await
            .expect("claim the clean name");
        std::fs::write(&claimed.output_path, b"the real bytes").expect("write the finished file");
        let name = claimed
            .output_path
            .file_name()
            .expect("a file name")
            .to_string_lossy();
        assert_eq!(name, "Song.m4a");
        assert!(
            link_under_public_name(&claimed.output_path, &area.staging, &claimed.public_name)
                .is_none(),
            "nothing to present when the internal name is already the public one"
        );
    }

    /// A loopback origin that answers every request with one fixed response, and
    /// counts how many requests it served.
    ///
    /// The count is what makes the zero-length test honest: it distinguishes
    /// "the downloader stopped" from "the server gave up", so a downloader that
    /// asked ten thousand times and then happened to fail would still fail.
    struct CannedServer {
        url: String,
        served: Arc<AtomicUsize>,
        running: Arc<AtomicBool>,
        accept: Option<std::thread::JoinHandle<()>>,
    }

    impl CannedServer {
        fn start(response: &'static str) -> CannedServer {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
            let port = listener.local_addr().expect("local_addr").port();
            listener.set_nonblocking(true).expect("non-blocking");
            let served = Arc::new(AtomicUsize::new(0));
            let running = Arc::new(AtomicBool::new(true));
            let accept = {
                let served = Arc::clone(&served);
                let running = Arc::clone(&running);
                std::thread::spawn(move || {
                    while running.load(Ordering::Relaxed) {
                        match listener.accept() {
                            // `serve_one` takes the stream by value, so the
                            // binding is never mutated. CI's clippy
                            // (`-D warnings`) rejects the `mut`.
                            Ok((stream, _)) => {
                                let served = Arc::clone(&served);
                                let running = Arc::clone(&running);
                                std::thread::spawn(move || {
                                    serve_one(stream, response, &served, &running);
                                });
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            Err(_) => break,
                        }
                    }
                })
            };
            CannedServer {
                url: format!("http://127.0.0.1:{port}/videoplayback?itag=140&clen=0"),
                served,
                running,
                accept: Some(accept),
            }
        }
    }

    impl Drop for CannedServer {
        fn drop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(handle) = self.accept.take() {
                let _ = handle.join();
            }
        }
    }

    /// Answer requests on one connection until the peer goes away. Read byte by
    /// byte and reply per request head, because a request with a body would
    /// otherwise be answered twice.
    fn serve_one(
        mut stream: std::net::TcpStream,
        response: &'static str,
        served: &AtomicUsize,
        running: &AtomicBool,
    ) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while running.load(Ordering::Relaxed) {
            match stream.read(&mut byte) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            head.push(byte[0]);
            if head.len() >= 4 && head[head.len() - 4..] == *b"\r\n\r\n" {
                served.fetch_add(1, Ordering::Relaxed);
                head.clear();
                if stream.write_all(response.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
            if head.len() > 16 * 1024 {
                return;
            }
        }
    }

    #[tokio::test]
    async fn a_zero_advertised_length_fails_instead_of_retrying_forever() {
        // The 2026-09-27 shape: the edge reports a `Content-Length: 0` object and
        // an empty body, and the resolver's `total_bytes` is `Some(0)`. Every
        // branch of the read loop's state machine said "keep going", so the job
        // spun until the process was killed. It must end, with a reason.
        const EMPTY: &str =
            "HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: 0\r\n\r\n";
        let server = CannedServer::start(EMPTY);

        let dir = std::env::temp_dir().join(format!("auralis_dl_zero_{}", Uuid::new_v4()));
        let downloader = Downloader::new(dir.clone());
        let id = downloader
            .download(StreamDownload {
                stream_url: server.url.clone(),
                title: "Zero Length".to_string(),
                artist: None,
                album: None,
                platform: "direct".to_string(),
                format: AudioFormat::M4a,
                ext: "m4a".to_string(),
                total_bytes: Some(0),
                thumbnail: None,
                headers: None,
                expected_duration_secs: None,
            })
            .await
            .expect("a download request should be accepted");

        let progress = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(state) = downloader.get_progress(id).await {
                    if matches!(
                        state.status,
                        DownloadStatus::Completed
                            | DownloadStatus::Failed
                            | DownloadStatus::Cancelled
                    ) {
                        return state;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("a zero-length object must not leave the download spinning");

        assert_eq!(
            progress.status,
            DownloadStatus::Failed,
            "expected a failure, got {status:?}: {error:?}",
            status = progress.status,
            error = progress.error
        );
        let error = progress.error.unwrap_or_default();
        assert!(
            error.contains("zero-length object"),
            "the failure must name the cause: {error}"
        );
        // `DownloadProgress::output_path` is a `String`, not a `PathBuf`, so the
        // on-disk check needs an explicit conversion. Calling `.exists()` on it
        // straight is a compile error, not a lint — this is the E0599 that kept
        // v2.6.59 and v2.6.60 red.
        let saved_to = downloader
            .get_progress(id)
            .await
            .expect("progress record")
            .output_path
            .expect("an output path");
        assert!(
            !std::path::Path::new(&saved_to).exists(),
            "nothing may be saved for an empty object, but {saved_to} exists"
        );

        let served = server.served.load(Ordering::Relaxed);
        assert!(
            served <= 1,
            "the job must be refused before or on the first request, \
             but the origin served {served}"
        );

        if let Some(handle) = downloader.tasks.write().await.remove(&id) {
            handle.abort();
            let _ = handle.await;
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_download_progress_id_matches_job_id() {
        let dir = std::env::temp_dir().join(format!("auralis_dl_id_test_{}", Uuid::new_v4()));
        let downloader = Downloader::new(dir.clone());
        let request = StreamDownload {
            stream_url: "http://127.0.0.1:1/test.wav".to_string(),
            title: "Identity Test".to_string(),
            artist: None,
            album: None,
            platform: "direct".to_string(),
            format: AudioFormat::Wav,
            ext: "wav".to_string(),
            total_bytes: None,
            thumbnail: None,
            headers: None,
            expected_duration_secs: None,
        };

        let job_id = downloader
            .download(request)
            .await
            .expect("test download should be accepted");
        let progress = downloader
            .get_progress(job_id)
            .await
            .expect("started download should have progress");

        assert_eq!(
            progress.id, job_id,
            "serialized progress ID must match the downloader job ID"
        );

        if let Some(handle) = downloader.tasks.write().await.remove(&job_id) {
            handle.abort();
            let _ = handle.await;
        }
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn test_resume_rejects_non_paused_download() {
        let dir = std::env::temp_dir().join(format!("auralis_dl_state_test_{}", Uuid::new_v4()));
        let downloader = Downloader::new(dir.clone());
        let id = Uuid::new_v4();
        let job = DownloadJob {
            stream_url: "https://example.com/audio.mp3".to_string(),
            title: "Audio".to_string(),
            artist: None,
            album: None,
            output_path: dir.join("audio.mp3"),
            staging_path: dir.join(".tmp").join("audio.mp3.part"),
            public_name: "audio.mp3".to_string(),
            thumbnail: None,
            headers: None,
            expected_duration_secs: None,
            total_bytes: None,
            format: AudioFormat::Mp3,
            ext: "mp3".to_string(),
        };
        let mut progress = DownloadProgress::with_id(
            id,
            "https://example.com/audio.mp3".to_string(),
            "Audio".to_string(),
            AudioFormat::Mp3,
        );
        progress.status = DownloadStatus::Downloading;
        downloader.jobs.write().await.insert(id, job);
        downloader
            .active_downloads
            .write()
            .await
            .insert(id, progress);

        let result = downloader.resume(id).await;

        assert!(matches!(result, Err(DownloaderError::InvalidState(_))));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

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
    fn test_sanitize_filename_unicode_boundary() {
        // The cap is 246 BYTES, not characters: 255 NAME_MAX − 8 for the longest
        // extension we allow − 1 for the dot. A char-based limit let a 200-char
        // CJK title reach ~600 bytes and fail the OS with ENAMETOOLONG, so
        // `sanitize_filename` now truncates on a char boundary at a byte budget.
        //
        // This test asserted 200 *characters* and had been failing since that
        // change landed — which is one of the reasons build-linux stayed red.
        // 246 / 2 bytes per 'é' is exactly 123 characters, so assert the byte
        // budget and the boundary, and let the character count follow from it
        // rather than being a second number that can drift.
        let name = "é".repeat(201);
        let sanitized = sanitize_filename(&name);

        assert!(
            sanitized.len() <= 246,
            "must fit the byte budget, got {} bytes",
            sanitized.len()
        );
        assert_eq!(
            sanitized.chars().count(),
            123,
            "246 bytes of 2-byte 'é' is 123 characters"
        );
        assert!(!sanitized.is_empty());

        // The point of walking back to a boundary: the result must still be valid
        // UTF-8 and must not end in a replacement or partial scalar. This is the
        // case the old `String::truncate(200)` PANICKED on.
        assert!(
            std::str::from_utf8(sanitized.as_bytes()).is_ok(),
            "a truncated multibyte name must stay valid UTF-8"
        );

        // A 3-byte scalar must land on a boundary too, not merely not panic.
        let cjk = "字".repeat(200);
        let cut = sanitize_filename(&cjk);
        assert!(
            cut.len() <= 246,
            "3-byte scalars must also respect the budget"
        );
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
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
