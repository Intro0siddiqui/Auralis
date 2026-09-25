//! Completing a *windowed* media response with explicit range requests.
//!
//! Why this exists
//! ---------------
//! In 2026 YouTube serves some InnerTube clients through SABR, and the
//! googlevideo edge then answers an ordinary (un-ranged) `GET` with a
//! **windowed** resource: the transfer completes, every advertised byte is
//! delivered, the container header still advertises the full track length — and
//! yet the file only decodes to part of the audio.
//!
//! Measured on a real device (v2.6.43, residential Jio, track `BElct8HWkp8`):
//! two different clients (`IOS` itag 140, `ANDROID_VR` itag 140) both delivered
//! exactly 99 s of a 287 s track, each at 100 % of its own `clen`. The cutoff is
//! a property of the *response*, not of the client, which is why rotating
//! clients cannot fix it and why resuming cannot either: the server believes it
//! already sent everything.
//!
//! The only remaining lever is to ask explicitly for the bytes *after* what we
//! hold. That either works — and the download completes — or it does not, and
//! the status codes it produced (`416` = the object really does end here, i.e. a
//! hard wall; `403` = gated) are the proof that no client-side resume can help.
//! Either way the caller learns something it can show the user.
//!
//! Scope: this module only appends bytes to a file that is already on disk. It
//! deliberately knows nothing about downloads, progress reporting or retries.

use reqwest::header::RANGE;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::warn;

/// Bytes requested per top-up call.
///
/// Kept modest on purpose: a SABR window hands over roughly a minute of media
/// per response, so a 2 MiB slice fits comfortably inside one window at typical
/// audio bitrates while still finishing a track in a handful of requests.
pub const TOPUP_CHUNK_BYTES: u64 = 2 * 1024 * 1024;

/// How many top-up rounds to try before declaring the stream unrecoverable.
pub const MAX_TOPUP_ROUNDS: usize = 4;

/// Seconds allowed for the response *headers* of a top-up request.
const HEADER_TIMEOUT_SECS: u64 = 20;

/// Seconds allowed while reading the body of a top-up request.
const BODY_STALL_TIMEOUT_SECS: u64 = 30;

/// Extract a raw (non-numeric) query parameter, e.g. `itag=140`.
pub fn url_param_str(url: &str, param: &str) -> Option<String> {
    let key = format!("{param}=");
    let start = url.find(&key)? + key.len();
    let val_str = &url[start..];
    let end = val_str.find('&').unwrap_or(val_str.len());
    Some(val_str[..end].to_string())
}

/// Add an explicit `range=start-end` window to a googlevideo URL.
///
/// Any `range` / `range2` the URL already carried is dropped first: those cap the
/// response window, and appending a second one would be ambiguous.
pub fn with_range_param(url: &str, start: u64, end: u64) -> String {
    let filtered = url
        .split('&')
        .filter(|part| {
            let key = part.split('=').next().unwrap_or("");
            !key.eq_ignore_ascii_case("range") && !key.eq_ignore_ascii_case("range2")
        })
        .collect::<Vec<_>>()
        .join("&");
    if filtered.contains('?') {
        format!("{filtered}&range={start}-{end}")
    } else {
        format!("{filtered}?range={start}-{end}")
    }
}

/// Append the next slice of the object to an already partially written file.
///
/// Returns the number of bytes appended, or a diagnostic string naming every
/// request shape that was tried. Three shapes are attempted because different
/// edges honour different mechanisms and a given URL gives no hint which one it
/// expects: the `range` query parameter plus the HTTP `Range` header, the header
/// alone, then the parameter alone.
pub async fn top_up(
    client: &reqwest::Client,
    job_headers: Option<&HashMap<String, String>>,
    url: &str,
    dest: &Path,
    start: u64,
    max_bytes: u64,
) -> Result<u64, String> {
    let end = start.saturating_add(max_bytes).saturating_sub(1);
    let ranged = with_range_param(url, start, end);
    // (label, request target, also send the Range header)
    let variants: [(&'static str, &str, bool); 3] = [
        ("url+header", ranged.as_str(), true),
        ("header", url, true),
        ("url", ranged.as_str(), false),
    ];

    let mut failures: Vec<String> = Vec::new();
    for (via, target, send_header) in variants {
        let mut req = super::downloader::inject_stream_headers(client.get(target), job_headers);
        if send_header {
            req = req.header(RANGE, format!("bytes={start}-{end}"));
        }
        let mut res = match tokio::time::timeout(
            Duration::from_secs(HEADER_TIMEOUT_SECS),
            req.send(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                failures.push(format!("{via}: transport error: {e}"));
                continue;
            }
            Err(_) => {
                failures.push(format!("{via}: timed out after {HEADER_TIMEOUT_SECS}s"));
                continue;
            }
        };
        let status = res.status();
        if !status.is_success() {
            failures.push(format!(
                "{via}: HTTP {} ({} bytes advertised)",
                status.as_u16(),
                res.content_length().unwrap_or(0)
            ));
            continue;
        }

        let mut file = match tokio::fs::OpenOptions::new().append(true).open(dest).await {
            Ok(f) => f,
            Err(e) => return Err(format!("{via}: cannot open file for append: {e}")),
        };
        let mut appended: u64 = 0;
        let mut read_problem: Option<String> = None;
        while appended < max_bytes {
            match tokio::time::timeout(Duration::from_secs(BODY_STALL_TIMEOUT_SECS), res.chunk())
                .await
            {
                Ok(Ok(Some(chunk))) => {
                    if let Err(e) = file.write_all(&chunk).await {
                        read_problem = Some(format!("write failed: {e}"));
                        break;
                    }
                    appended += chunk.len() as u64;
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => {
                    read_problem = Some(format!("read error: {e}"));
                    break;
                }
                Err(_) => {
                    read_problem = Some(format!("stalled for {BODY_STALL_TIMEOUT_SECS}s"));
                    break;
                }
            }
        }
        drop(file);

        if appended > 0 {
            if let Some(problem) = read_problem {
                // Partial data is still progress — keep the bytes, note the reason.
                warn!(bytes = appended, problem = %problem, via = via, "Range top-up ended early but delivered bytes");
            }
            return Ok(appended);
        }
        let suffix = read_problem.map(|p| format!(" ({p})")).unwrap_or_default();
        failures.push(format!(
            "{via}: HTTP {} but the body was empty{suffix}",
            status.as_u16()
        ));
    }

    Err(failures.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_range_param_replaces_an_existing_window() {
        // A resolved googlevideo URL may already carry a capping `range=`; the
        // top-up must replace it rather than add a second one.
        let url = "https://rr3---sn-gwpa-civee.googlevideo.com/videoplayback?expire=1&itag=140&range=0-1048575&clen=4194304";
        let ranged = with_range_param(url, 1_048_576, 3_145_727);
        assert!(ranged.contains("range=1048576-3145727"), "got {ranged}");
        assert_eq!(ranged.matches("range=").count(), 1, "got {ranged}");
        assert!(
            ranged.contains("itag=140"),
            "other params must survive: {ranged}"
        );
        assert!(ranged.contains("clen=4194304"), "got {ranged}");
    }

    #[test]
    fn with_range_param_drops_range2_too() {
        let url = "https://x.googlevideo.com/videoplayback?id=1&range2=0-99";
        let ranged = with_range_param(url, 100, 199);
        assert!(!ranged.contains("range2"), "got {ranged}");
        assert!(ranged.contains("range=100-199"), "got {ranged}");
    }

    #[test]
    fn with_range_param_adds_a_query_when_there_is_none() {
        let ranged = with_range_param("https://example.test/audio.m4a", 0, 1023);
        assert!(ranged.contains("?range=0-1023"), "got {ranged}");
    }

    #[test]
    fn url_param_str_reads_itag() {
        let url = "https://x.googlevideo.com/videoplayback?expire=9&itag=140&clen=1234";
        assert_eq!(url_param_str(url, "itag").as_deref(), Some("140"));
        assert_eq!(url_param_str(url, "mime"), None);
        assert_eq!(
            url_param_str("https://x.test/a?itag=18", "itag").as_deref(),
            Some("18")
        );
    }

    #[test]
    fn top_up_against_a_dead_port_reports_every_variant() {
        // Port 1 on loopback refuses immediately: all three request shapes must
        // be reported so the failure text can explain what the server said.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let dir = std::env::temp_dir().join(format!("auralis_topup_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let dest = dir.join("partial.m4a");
        std::fs::write(&dest, b"partial").unwrap();
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("client");
        let url = "http://127.0.0.1:1/videoplayback?itag=140&clen=99";
        let result = runtime.block_on(top_up(&client, None, url, &dest, 7, TOPUP_CHUNK_BYTES));
        let err = result.expect_err("a dead port cannot deliver bytes");
        assert!(err.contains("url+header"), "got {err}");
        assert!(err.contains("header"), "got {err}");
        assert!(err.contains("url:"), "got {err}");
        // The partial file must be left exactly as it was.
        assert_eq!(std::fs::read(&dest).unwrap(), b"partial".to_vec());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
