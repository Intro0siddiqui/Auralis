//! Local HTTP Streaming Proxy
//!
//! Enables direct streaming of YouTube audio into HTML5 `<audio>` elements
//! without pre-downloading the entire audio file to disk.
//!
//! Because browser `<audio>` elements cannot attach custom HTTP headers
//! (`User-Agent`, `Referer`, `Origin`, InnerTube client context), direct
//! requests to `googlevideo.com` trigger HTTP 403 Forbidden. This proxy runs
//! on a local loopback port, receives requests from `<audio src="...">`,
//! forwards them upstream with all required headers and range requests, and
//! pipes the chunks back to the client.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Maximum number of active stream sessions cached in memory.
const MAX_SESSIONS: usize = 100;

/// Default Android YouTube client User-Agent used as fallback.
const DEFAULT_USER_AGENT: &str =
    "com.google.android.youtube/20.10.38 (Linux; U; Android 14; en_US; Pixel 8 Build/UD1A.230803.041)";
const DEFAULT_REFERER: &str = "https://www.youtube.com/";
const DEFAULT_ORIGIN: &str = "https://www.youtube.com";

/// A registered media stream session containing the upstream URL and headers.
#[derive(Clone, Debug)]
pub struct StreamSession {
    pub url: String,
    pub headers: HashMap<String, String>,
}

/// Local HTTP streaming proxy server.
#[derive(Clone)]
pub struct StreamProxy {
    port: u16,
    sessions: Arc<RwLock<HashMap<String, StreamSession>>>,
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl StreamProxy {
    /// Start the proxy server listening on an ephemeral port on `127.0.0.1`.
    pub async fn start() -> Result<Self, std::io::Error> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let sessions = Arc::new(RwLock::new(HashMap::new()));

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        let proxy = Self {
            port,
            sessions: sessions.clone(),
            client: client.clone(),
        };

        let sessions_loop = sessions.clone();
        let client_loop = client.clone();

        tokio::spawn(async move {
            info!(port, "StreamProxy listening on 127.0.0.1:{port}");
            loop {
                match listener.accept().await {
                    Ok((socket, _peer)) => {
                        let sessions = sessions_loop.clone();
                        let client = client_loop.clone();
                        tokio::spawn(async move {
                            if let Err(e) = Self::handle_connection(socket, sessions, client).await
                            {
                                debug!("StreamProxy connection handler closed: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        warn!("StreamProxy accept error: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        });

        Ok(proxy)
    }

    /// Return the bound loopback TCP port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Access the underlying reqwest client.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Register an upstream URL and required headers, returning the local proxy URL.
    pub async fn register(&self, url: String, headers: HashMap<String, String>) -> String {
        let id = Uuid::new_v4().to_string();
        let mut guard = self.sessions.write().await;
        if guard.len() >= MAX_SESSIONS {
            if let Some(oldest_key) = guard.keys().next().cloned() {
                guard.remove(&oldest_key);
            }
        }
        guard.insert(id.clone(), StreamSession { url, headers });
        format!("http://127.0.0.1:{}/stream/{}", self.port, id)
    }

    /// Handle an individual incoming HTTP client request.
    async fn handle_connection(
        mut socket: tokio::net::TcpStream,
        sessions: Arc<RwLock<HashMap<String, StreamSession>>>,
        client: reqwest::Client,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 1024];
        let mut header_end = None;

        // Read until the end of HTTP request headers (\r\n\r\n or \n\n)
        while buf.len() < 32768 {
            let n = socket.read(&mut chunk).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = Some(pos + 4);
                break;
            }
            if let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
                header_end = Some(pos + 2);
                break;
            }
        }

        let Some(header_end) = header_end else {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
            return Ok(());
        };

        let header_str = match std::str::from_utf8(&buf[..header_end]) {
            Ok(s) => s,
            Err(_) => {
                let resp =
                    "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(resp.as_bytes()).await;
                return Ok(());
            }
        };

        let mut lines = header_str.lines();
        let request_line = match lines.next() {
            Some(l) => l,
            None => return Ok(()),
        };

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path_and_query = parts.next().unwrap_or("");

        // Handle CORS preflight
        if method.eq_ignore_ascii_case("OPTIONS") {
            let cors_resp = "HTTP/1.1 204 No Content\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
Access-Control-Allow-Headers: *\r\n\
Content-Length: 0\r\n\
Connection: close\r\n\r\n";
            let _ = socket.write_all(cors_resp.as_bytes()).await;
            let _ = socket.flush().await;
            return Ok(());
        }

        let is_head = method.eq_ignore_ascii_case("HEAD");
        let is_get = method.eq_ignore_ascii_case("GET");
        if !is_get && !is_head {
            let resp =
                "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.flush().await;
            return Ok(());
        }

        // Validate stream route
        let path = path_and_query.split('?').next().unwrap_or("");
        if !path.starts_with("/stream/") {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.flush().await;
            return Ok(());
        }

        let id = path["/stream/".len()..].trim_matches('/');
        let session = {
            let guard = sessions.read().await;
            guard.get(id).cloned()
        };

        let Some(session) = session else {
            debug!(id, "StreamProxy: session not found");
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.flush().await;
            return Ok(());
        };

        // Extract client Range header
        let mut range_header = None;
        for line in lines {
            if let Some((name, val)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("range") {
                    range_header = Some(val.trim().to_string());
                }
            }
        }

        // Parse client Range request:
        // Client can send "bytes=START-" or "bytes=START-END".
        let (client_start, client_end) = match range_header.as_deref() {
            Some(r) => {
                if let Some(spec) = r.strip_prefix("bytes=") {
                    let mut parts = spec.split('-');
                    let s_str = parts.next().unwrap_or("").trim();
                    let e_str = parts.next().unwrap_or("").trim();
                    let s = s_str.parse::<u64>().unwrap_or(0);
                    let e = e_str.parse::<u64>().ok();
                    (s, e)
                } else {
                    (0, None)
                }
            }
            None => (0, None),
        };

        // YouTube CDN (googlevideo) strictly returns HTTP 403 Forbidden for
        // open-ended ranges (`bytes=0-`) or requests without a Range header.
        // Every upstream request MUST specify a bounded range window (e.g. 512 KiB).
        const STREAM_CHUNK_SIZE: u64 = 512 * 1024; // 512 KiB chunk window

        let initial_chunk_end = match client_end {
            Some(e) => e.min(client_start + STREAM_CHUNK_SIZE - 1),
            None => client_start + STREAM_CHUNK_SIZE - 1,
        };

        let initial_range = format!("bytes={client_start}-{initial_chunk_end}");
        let initial_req =
            Self::build_upstream_request(&client, &session, &session.url, &initial_range, is_head);

        let mut initial_res = match initial_req.send().await {
            Ok(r) => r,
            Err(e) => {
                error!(url = %session.url, error = %e, "StreamProxy upstream initial request failed");
                let body = format!("Bad Gateway: {e}");
                let resp = format!(
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.flush().await;
                return Ok(());
            }
        };

        let status = initial_res.status();
        let status_code = status.as_u16();

        if !status.is_success() && status_code != 206 {
            warn!(url = %session.url, status = status_code, "StreamProxy upstream returned error");
            let resp = format!(
                "HTTP/1.1 {status_code} Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n"
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.flush().await;
            return Ok(());
        }

        let effective_url = initial_res.url().to_string();

        // Extract total size from upstream Content-Range
        let total_size = initial_res
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cr| cr.rsplit('/').next())
            .and_then(|t| t.trim().parse::<u64>().ok());

        let target_end = match client_end {
            Some(e) => e,
            None => total_size
                .map(|t| t.saturating_sub(1))
                .unwrap_or(initial_chunk_end),
        };

        let content_length = target_end.saturating_sub(client_start) + 1;
        let content_type = initial_res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("audio/mp4");

        // Write HTTP 206 response headers to client
        let mut resp_header_bytes = Vec::with_capacity(512);
        resp_header_bytes.extend_from_slice(b"HTTP/1.1 206 Partial Content\r\n");
        resp_header_bytes.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        resp_header_bytes
            .extend_from_slice(format!("Content-Length: {content_length}\r\n").as_bytes());

        if let Some(total) = total_size {
            resp_header_bytes.extend_from_slice(
                format!("Content-Range: bytes {client_start}-{target_end}/{total}\r\n").as_bytes(),
            );
        } else {
            resp_header_bytes.extend_from_slice(
                format!("Content-Range: bytes {client_start}-{target_end}/*\r\n").as_bytes(),
            );
        }

        resp_header_bytes.extend_from_slice(b"Accept-Ranges: bytes\r\n");
        resp_header_bytes.extend_from_slice(b"Access-Control-Allow-Origin: *\r\n");
        resp_header_bytes.extend_from_slice(
            b"Access-Control-Expose-Headers: Content-Range, Content-Length, Accept-Ranges\r\n",
        );
        resp_header_bytes.extend_from_slice(b"Connection: keep-alive\r\n\r\n");

        socket.write_all(&resp_header_bytes).await?;

        if is_head {
            let _ = socket.flush().await;
            return Ok(());
        }

        // Stream initial chunk to client
        let mut client_disconnected = false;
        while let Ok(Some(chunk)) = initial_res.chunk().await {
            if let Err(e) = socket.write_all(&chunk).await {
                debug!("StreamProxy client disconnected during initial chunk: {e}");
                client_disconnected = true;
                break;
            }
        }

        // Continuously stream remaining chunks over the socket until target_end
        if !client_disconnected && initial_chunk_end < target_end {
            let mut current_offset = initial_chunk_end + 1;
            while current_offset <= target_end {
                let chunk_end = (current_offset + STREAM_CHUNK_SIZE - 1).min(target_end);
                let range_str = format!("bytes={current_offset}-{chunk_end}");
                let next_req = Self::build_upstream_request(
                    &client,
                    &session,
                    &effective_url,
                    &range_str,
                    false,
                );

                let mut next_res = match next_req.send().await {
                    Ok(r) if r.status().is_success() => r,
                    Ok(r) => {
                        debug!(offset = current_offset, status = %r.status(), "StreamProxy chunk upstream non-success");
                        break;
                    }
                    Err(e) => {
                        debug!(offset = current_offset, error = %e, "StreamProxy chunk upstream send failed");
                        break;
                    }
                };

                let mut chunk_err = false;
                while let Ok(Some(chunk)) = next_res.chunk().await {
                    if let Err(e) = socket.write_all(&chunk).await {
                        debug!("StreamProxy client disconnected during continuous streaming: {e}");
                        chunk_err = true;
                        break;
                    }
                }

                if chunk_err {
                    break;
                }
                current_offset = chunk_end + 1;
            }
        }

        let _ = socket.flush().await;
        Ok(())
    }

    /// Helper to build an upstream reqwest Request with required headers.
    fn build_upstream_request(
        client: &reqwest::Client,
        session: &StreamSession,
        url: &str,
        range: &str,
        is_head: bool,
    ) -> reqwest::RequestBuilder {
        let mut req = if is_head {
            client.head(url)
        } else {
            client.get(url)
        };

        for (k, v) in &session.headers {
            if k.eq_ignore_ascii_case("range") {
                continue;
            }
            req = req.header(k, v);
        }

        let has_ua = session
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("user-agent"));
        if !has_ua {
            req = req.header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT);
        }

        let has_referer = session
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("referer"));
        if !has_referer {
            req = req.header(reqwest::header::REFERER, DEFAULT_REFERER);
        }

        let has_origin = session
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("origin"));
        if !has_origin {
            req = req.header(reqwest::header::ORIGIN, DEFAULT_ORIGIN);
        }

        req.header(reqwest::header::RANGE, range)
    }
}
