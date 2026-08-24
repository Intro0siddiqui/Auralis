//! Download Commands
//!
//! Tauri command handlers for media downloads. The frontend resolves a
//! user-facing URL (e.g. YouTube) into a direct audio stream URL via
//! `youtube.js`; these commands stream that URL to disk through the
//! [`Downloader`].

use crate::domain::models::{AudioFormat, DownloadProgress, DownloadStatus};
use crate::infrastructure::media::downloader::{Downloader, StreamDownload};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, State};
use tracing::{error, info, warn};
use uuid::Uuid;

/// A single resolved download request.
///
/// The `url` is a *direct* audio stream URL (already resolved by the frontend
/// `youtube.js` resolver), not a raw user-facing link.
#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// Resolved direct audio stream URL (https).
    pub url: String,
    /// Display title.
    pub title: String,
    /// Source platform label (e.g. `youtube`, `direct`).
    pub platform: Option<String>,
    /// Container/format metadata (display only).
    pub format: Option<AudioFormat>,
    /// File extension of the saved bytes (e.g. `webm`, `m4a`).
    pub ext: Option<String>,
    /// Known total size in bytes, if known.
    pub total_bytes: Option<u64>,
    /// Optional thumbnail URL for display.
    pub thumbnail: Option<String>,
    /// Optional headers (UA/Referer/Origin) matched to the InnerTube client.
    pub headers: Option<HashMap<String, String>>,
}

/// Playlist download request — a pre-resolved list of items.
#[derive(Debug, Serialize, Deserialize)]
pub struct PlaylistDownloadRequest {
    pub items: Vec<DownloadRequest>,
    pub format: Option<AudioFormat>,
    pub max_items: Option<u32>,
}

/// Start downloading a single resolved audio track.
#[tauri::command]
pub async fn download_audio(
    request: DownloadRequest,
    app: AppHandle,
    downloader: State<'_, Downloader>,
) -> Result<DownloadProgress, String> {
    let host = request.url.split('/').nth(2).unwrap_or("unknown");
    let hdr_keys = request
        .headers
        .as_ref()
        .map(|h| h.keys().cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| "-".to_string());
    info!(url = %request.url, host = %host, title = %request.title, platform = ?request.platform, ext = ?request.ext, total_bytes = ?request.total_bytes, headers = %hdr_keys, "Download audio requested");
    if request.url.len() > 200 {
        tracing::debug!(url = %request.url, headers = ?request.headers, "Full download request");
    }

    if !request.url.starts_with("https://") {
        error!(url = %request.url, "Rejected non-HTTPS download request");
        return Err("Only secure HTTPS URLs are supported".to_string());
    }

    let format = request.format.unwrap_or(AudioFormat::Mp3);
    let stream = StreamDownload {
        stream_url: request.url.clone(),
        title: request.title.clone(),
        platform: request
            .platform
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        format,
        ext: request
            .ext
            .clone()
            .unwrap_or_else(|| format.extension().to_string()),
        total_bytes: request.total_bytes,
        thumbnail: request.thumbnail.clone(),
        headers: request.headers.clone(),
    };

    let id = downloader.download(stream).await.map_err(|e| {
        error!(url = %request.url, host = %host, title = %request.title, error = %e, "Failed to start download — check logs for DIAGNOSTIC");
        format!("Failed to start download [{host}]: {e}")
    })?;

    let state = downloader
        .get_progress(id)
        .await
        .ok_or("Download not found after starting")?;

    // Stream progress + completion events to the frontend while the
    // download task runs. On failure the `error` field already contains
    // a verbose diagnostic (HTTP status + body snippet + host) built in
    // run_stream — it is emitted verbatim so the JS toast/logcat can show it.
    let app_handle = app.clone();
    let dl = (*downloader).clone();
    let emit_id = id;
    tauri::async_runtime::spawn(async move {
        loop {
            match dl.get_progress(emit_id).await {
                Some(progress) => {
                    let _ = app_handle.emit("download:progress", &progress);
                    if matches!(
                        progress.status,
                        DownloadStatus::Completed
                            | DownloadStatus::Failed
                            | DownloadStatus::Cancelled
                    ) {
                        if progress.status == DownloadStatus::Failed {
                            let host = progress.url.split('/').nth(2).unwrap_or("unknown");
                            error!(download_id = %emit_id, host = %host, title = %progress.title, error = ?progress.error, url = %progress.url, downloaded = progress.downloaded_bytes, total = ?progress.total_bytes, "Emitting download:completed (failed) — DIAGNOSTIC visible to frontend/logcat");
                            warn!(download_id = %emit_id, "DIAGNOSTIC download_completed_failed id={} host={} title={} error={:?} downloaded={}/{:?} url={}", emit_id, host, progress.title, progress.error, progress.downloaded_bytes, progress.total_bytes, progress.url);
                        } else {
                            info!(download_id = %emit_id, status = %progress.status, "Emitting download:completed");
                        }
                        let _ = app_handle.emit("download:completed", &progress);
                        // Also emit a dedicated diagnostic event so even if the
                        // frontend misses `completed`, the JS console (mirrored to
                        // logcat via webview-log-js-console-messages) still shows it.
                        if progress.status == DownloadStatus::Failed {
                            let diag = format!(
                                "DIAGNOSTIC download_failed id={} host={} title={} status={} error={:?} downloaded={}/{:?} url={}",
                                emit_id,
                                progress.url.split('/').nth(2).unwrap_or("unknown"),
                                progress.title,
                                progress.status,
                                progress.error,
                                progress.downloaded_bytes,
                                progress.total_bytes,
                                progress.url
                            );
                            let _ = app_handle.emit("download:diagnostic", diag);
                        }
                        break;
                    }
                }
                None => break,
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    Ok(state)
}

/// Start downloading a playlist (one download per pre-resolved item).
#[tauri::command]
pub async fn download_playlist(
    request: PlaylistDownloadRequest,
    downloader: State<'_, Downloader>,
) -> Result<Vec<DownloadProgress>, String> {
    info!(count = request.items.len(), "Playlist download requested");

    let max_items = request.max_items.unwrap_or(25) as usize;
    let mut results: Vec<DownloadProgress> = Vec::new();

    for item in request.items.into_iter().take(max_items) {
        if !item.url.starts_with("https://") {
            error!(url = %item.url, "Skipping non-https playlist item");
            continue;
        }

        let format = item.format.or(request.format).unwrap_or(AudioFormat::Mp3);
        let stream = StreamDownload {
            stream_url: item.url.clone(),
            title: item.title.clone(),
            platform: item
                .platform
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            format,
            ext: item
                .ext
                .clone()
                .unwrap_or_else(|| format.extension().to_string()),
            total_bytes: item.total_bytes,
            thumbnail: item.thumbnail,
            headers: item.headers.clone(),
        };

        match downloader.download(stream).await {
            Ok(id) => {
                if let Some(progress) = downloader.get_progress(id).await {
                    results.push(progress);
                }
            }
            Err(e) => {
                error!(url = %item.url, error = %e, "Failed to queue playlist track");
            }
        }
    }

    Ok(results)
}

/// Pause an in-progress download.
#[tauri::command]
pub async fn pause_download(id: Uuid, downloader: State<'_, Downloader>) -> Result<(), String> {
    info!(download_id = %id, "Pause download requested");

    downloader.pause(id).await.map_err(|e| {
        error!(error = %e, "Failed to pause download");
        e.to_string()
    })
}

/// Resume a paused download.
#[tauri::command]
pub async fn resume_download(id: Uuid, downloader: State<'_, Downloader>) -> Result<(), String> {
    info!(download_id = %id, "Resume download requested");

    downloader.resume(id).await.map_err(|e| {
        error!(error = %e, "Failed to resume download");
        e.to_string()
    })
}

/// Cancel a download (queued or in-progress).
#[tauri::command]
pub async fn cancel_download(id: Uuid, downloader: State<'_, Downloader>) -> Result<(), String> {
    info!(download_id = %id, "Cancel download requested");

    downloader.cancel(id).await.map_err(|e| {
        error!(error = %e, "Failed to cancel download");
        e.to_string()
    })
}

/// Get current progress for a specific download.
#[tauri::command]
pub async fn get_download_progress(
    id: Uuid,
    downloader: State<'_, Downloader>,
) -> Result<Option<DownloadProgress>, String> {
    info!(download_id = %id, "Get download progress requested");

    Ok(downloader.get_progress(id).await)
}

/// List all active downloads.
#[tauri::command]
pub async fn list_downloads(
    downloader: State<'_, Downloader>,
) -> Result<Vec<DownloadProgress>, String> {
    info!("List downloads requested");

    Ok(downloader.list_active().await)
}

/// Request payload for native HTTP fetch (bypasses browser CORS).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpFetchRequest {
    pub url: String,
    pub method: Option<String>,
    pub headers: Option<HashMap<String, Option<String>>>,
    pub body: Option<String>,
}

/// Response payload from native HTTP fetch.
#[derive(Debug, Serialize, Deserialize)]
pub struct HttpFetchResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Hosts the frontend is allowed to reach through native HTTP fetch.
/// Exact-match hosts.
const HTTP_FETCH_ALLOWED_HOSTS: &[&str] = &[
    "www.youtube.com",
    "music.youtube.com",
    "youtubei.googleapis.com",
    "i.ytimg.com",
    "jnn-pa.googleapis.com",
    "www.google.com",
];
/// Subdomain-suffix hosts (`<anything>.suffix`).
const HTTP_FETCH_ALLOWED_SUFFIXES: &[&str] = &[
    "googlevideo.com",
    "ytimg.com",
    "youtube.com",
    "google.com",
    "googleapis.com",
];

/// Check whether `host` is on the native-fetch egress allowlist
/// (dot-boundary suffix match; case-insensitive; trailing dot tolerated).
fn host_is_allowed_egress(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if HTTP_FETCH_ALLOWED_HOSTS.contains(&host.as_str()) {
        return true;
    }
    HTTP_FETCH_ALLOWED_SUFFIXES.iter().any(|suffix| {
        host.len() > suffix.len()
            && host.ends_with(suffix)
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
    })
}

/// Reject hosts that are literal private/loopback/link-local IP addresses
/// (cloud metadata endpoints, internal services, localhost).
fn ensure_not_private_ip_host(host: &str) -> Result<(), String> {
    use std::net::IpAddr;

    let is_private_v4 = |ip: std::net::Ipv4Addr| {
        ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || ip.is_unspecified()
            || ip.is_broadcast()
    };

    let stripped = host.trim().trim_start_matches('[').trim_end_matches(']');
    let Ok(ip) = stripped.parse::<IpAddr>() else {
        return Ok(()); // regular hostname — allowlist above governs it
    };

    let blocked = match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                is_private_v4(v4)
            } else {
                let seg = v6.segments();
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                    || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link local
            }
        }
    };

    if blocked {
        Err(format!(
            "Blocked request to private/reserved address '{host}'"
        ))
    } else {
        Ok(())
    }
}

/// Native HTTP fetch command executed via reqwest without browser CORS restrictions.
///
/// Egress is restricted to the YouTube/InnerTube infrastructure the frontend
/// resolver legitimately needs (see `ui/js/youtube.js`) to prevent the command
/// from being abused as an SSRF primitive against internal/metadata endpoints.
#[tauri::command]
pub async fn http_fetch(request: HttpFetchRequest) -> Result<HttpFetchResponse, String> {
    let url = reqwest::Url::parse(&request.url)
        .map_err(|e| format!("Invalid URL '{}': {e}", request.url))?;
    if url.scheme() != "https" {
        warn!(url = %request.url, "Rejected non-HTTPS native fetch");
        return Err("Only HTTPS URLs are permitted for native fetch".to_string());
    }
    let host = url.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err("URL has no host".to_string());
    }
    if !host_is_allowed_egress(host) {
        warn!(host = %host, "Blocked native fetch to non-allowlisted host");
        return Err(format!(
            "Host '{host}' is not on the native fetch allowlist"
        ));
    }
    ensure_not_private_ip_host(host)?;

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .user_agent("Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UD1A.230803.041) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let method = match request
        .method
        .as_deref()
        .unwrap_or("GET")
        .to_uppercase()
        .as_str()
    {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => reqwest::Method::GET,
    };

    let mut req_builder = client.request(method, &request.url);

    if let Some(headers) = request.headers {
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("accept-encoding") || k.eq_ignore_ascii_case("host") {
                continue;
            }
            if let Some(v_str) = v {
                if let (Ok(name), Ok(val)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(&v_str),
                ) {
                    req_builder = req_builder.header(name, val);
                }
            }
        }
    }

    if let Some(body) = request.body {
        req_builder = req_builder.body(body);
    }

    let response = req_builder
        .send()
        .await
        .map_err(|e| format!("HTTP request to {} failed: {e}", request.url))?;

    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();

    let mut resp_headers = HashMap::new();
    for (k, v) in response.headers() {
        if k.as_str().eq_ignore_ascii_case("content-encoding")
            || k.as_str().eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        if let Ok(v_str) = v.to_str() {
            resp_headers.insert(k.as_str().to_string(), v_str.to_string());
        }
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    Ok(HttpFetchResponse {
        status,
        status_text,
        headers: resp_headers,
        body,
    })
}
