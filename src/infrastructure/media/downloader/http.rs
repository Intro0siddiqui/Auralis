use std::collections::HashMap;
use std::time::Duration;
use tracing::info;

use crate::domain::models::DownloadStatus;

use super::DownloaderError;

/// Slack allowed when comparing received bytes against the advertised object
/// size. A few tens of KiB absorbs container/manifest rounding differences
/// between what the resolver reports and what the edge serves.
pub(crate) const COMPLETE_TOLERANCE_BYTES: u64 = 64 * 1024;

/// Minimum plausible audio bitrate (≈64 kbps) used to sanity-check an
/// advertised object size against the known track duration. A 4-minute track
/// that "totals" 1 MB is a truncated response window, not a 1-minute song.
pub(crate) const MIN_PLAUSIBLE_BYTES_PER_SEC: u64 = 8_000;
#[allow(dead_code)]
pub(crate) const MIN_PLAUSIBLE_BITRATE_BPS: u64 = MIN_PLAUSIBLE_BYTES_PER_SEC;

/// Tracks shorter than this are exempt from the bitrate plausibility check
/// (very short clips legitimately have tiny byte counts and rounding noise).
pub(crate) const MIN_DURATION_FOR_BITRATE_CHECK: u32 = 20;

/// Bytes/second floor used **only** when neither the resolver nor the server
/// gave a size: ≈32 kbps. Used to catch "the stream just stopped" truncations.
pub(crate) const MIN_BYTES_PER_SEC_FALLBACK: u64 = 4_000;

/// Why a refused interrupt was refused.
///
/// These are `const`s rather than inline literals because they are long enough
/// that an unbreakable line is a real possibility, and an unbreakable line is
/// the one thing that is known to put code outside the `cargo fmt --check` gate.
/// Both name the *consequence* rather than only the state, because "cannot
/// cancel" does not tell the caller whether their file is safe.
pub(crate) const REASON_INTERRUPT_COMMITTING: &str =
    "the download already passed every check and is \
                                        being written to its final location — the file is \
                                        complete, and interrupting now would either delete it \
                                        or leave nothing to resume from";
pub(crate) const REASON_INTERRUPT_TERMINAL: &str = "the download has already finished";

/// A user request to interrupt a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interrupt {
    Pause,
    Cancel,
}

impl Interrupt {
    /// Past tense, for error messages: "cannot pause download in …".
    pub(crate) fn verb(self) -> &'static str {
        match self {
            Interrupt::Pause => "pause",
            Interrupt::Cancel => "cancel",
        }
    }
}

/// What a downloader may do about an interrupt request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestOutcome {
    /// Carry the request out.
    Apply,
    /// The job is already in the requested state: nothing to do, and that is
    /// success rather than an error.
    AlreadySettled,
    /// Refuse. The job is past the point where interrupting it is safe.
    Refuse,
}

/// Helper to extract an integer query parameter from a URL (e.g. `clen=15234567`).
pub(crate) fn extract_url_param_u64(url: &str, param: &str) -> Option<u64> {
    let key = format!("{param}=");
    let start = url.find(&key)? + key.len();
    let val_str = &url[start..];
    let end = val_str.find('&').unwrap_or(val_str.len());
    val_str[..end].parse::<u64>().ok()
}

/// Helper to extract a float query parameter from a URL (e.g. `dur=245.123`).
pub(crate) fn extract_url_param_f64(url: &str, param: &str) -> Option<f64> {
    let key = format!("{param}=");
    let start = url.find(&key)? + key.len();
    let val_str = &url[start..];
    let end = val_str.find('&').unwrap_or(val_str.len());
    val_str[..end].parse::<f64>().ok()
}

/// Build the HTTP client used to stream one job.
///
/// **There is deliberately no address pinning here**, and there was for two releases.
///
/// A googlevideo URL carries `ip=`, and §4.7.16 of `AGENTS.md` concluded the CDN checks
/// that the request egresses from it. It does not. Measured 2026-10-02 with
/// `examples/fetch_probe.rs`, on the device itself, against a freshly resolved URL:
///
/// ```text
/// bind (OS-chosen)                                -> HTTP 206
/// bind <the ip= address>                          -> HTTP 206
/// bind <a DIFFERENT local address on the device>  -> HTTP 206   <- the load-bearing row
/// bind <an address that has since rotated away>   -> CONNECT FAILED
/// ```
///
/// Row three settles it: same URL, egressing from a different IPv6 address than `ip=`,
/// still served. There is no source-address binding to satisfy.
///
/// The 403 that §4.7.16 attributed to binding was **signature validation**. `ip` is
/// listed in `sparams`, so it is covered by `sig`/`lsig`, and editing the parameter
/// invalidates the signature. @audit's research said exactly this and the pin was built
/// over it anyway.
///
/// v2.6.68 used `resolve_to_addrs(host, ip)`, which asks for the opposite thing: dial that
/// address to reach the CDN. Since `ip=` is the *client's* address, that asks the phone to
/// be the CDN. It could never connect, and it converted a diagnosable 403-at-byte-0 into
/// an opaque `error sending request` at byte 0.
///
/// Correcting it to `local_address` did not help either — rows one and two above are
/// identical — and it made things worse in row four. `ip=` names the RFC 4941 privacy
/// address, which rotates; this session watched it rotate between two resolves. Pinning
/// to a rotated-away address breaks a download that would otherwise have succeeded.
///
/// So the mechanism is gone rather than corrected. Keeping a corrected version of a
/// mechanism that turns out not to exist only leaves a failure mode behind.
pub(crate) fn build_stream_client(ua: &str) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .user_agent(ua.to_string())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
}

/// Remove query parameters that cap the response window (`range`, `range2`).
///
/// Throttled googlevideo URLs can carry `&range=0-1048575`. The edge then serves
/// only that window and reports it as the whole object, so a partial download
/// is indistinguishable from a complete one. Dropping the parameter makes the
/// server return the full resource.
pub(crate) fn strip_response_range_params(url: &str) -> String {
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
pub(crate) fn parse_content_range_total(value: &str) -> Option<u64> {
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
pub(crate) async fn probe_total_bytes(
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
pub(crate) fn classify_interrupt(request: Interrupt, status: DownloadStatus) -> RequestOutcome {
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

/// The error a refused interrupt reports.
pub(crate) fn interrupt_error(request: Interrupt, status: DownloadStatus) -> DownloaderError {
    let reason = match status {
        DownloadStatus::Committing => REASON_INTERRUPT_COMMITTING,
        _ => REASON_INTERRUPT_TERMINAL,
    };
    DownloaderError::InvalidState(format!(
        "cannot {} download in {status} state: {reason}",
        request.verb()
    ))
}
