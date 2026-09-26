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
//! exactly 99 s of a 287 s track, each at 100 % of its own `clen`. The cutoff
//! is a property of the *response*, not of the client, which is why rotating
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
//!
//! Why the response is validated
//! -----------------------------
//! Appending whatever a `2xx` happens to carry is unsafe, because the edge is
//! free to ignore the range it was given. A `200` answers with the *whole*
//! object from byte 0, and appending it to a file that already holds N bytes
//! produces `[prefix][start-of-object]`: a file that never existed, which still
//! decodes to *something*, and therefore slips past the caller's only post-check
//! (a decoded-duration test) as a "recovered" download. A `206` for the wrong
//! window is the same hazard with a friendlier status code. So a response is
//! appended only when it is provably the slice that was asked for, and every
//! rejection is worded so it can be told apart from a `416` — the one status
//! that means "this object is finished" rather than "this edge did not oblige".

use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
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

/// The span a `206` claims to be delivering, plus the object length it
/// advertises. `total` is `None` for the `*` form, which RFC 9110 allows when
/// the server does not know the object length.
struct ContentRange {
    start: u64,
    end: u64,
    total: Option<u64>,
}

/// Why a response must not be appended to the staging file.
enum Reject {
    /// This request shape answered with something unusable. Another shape may
    /// still deliver the window, so the loop continues.
    Shape(String),
    /// The object provably ends at or before `start`, so no request shape can
    /// ever produce more bytes. Retrying is pointless and the caller should
    /// stop.
    ObjectEnds(String),
}

/// Split a `Content-Range` value into its unit and remainder.
///
/// The separator is matched as any of space, tab or `=`: RFC 9110 spells the
/// header `bytes {range}/{complete-length}` while some origins emit
/// `bytes={range}/{complete-length}`, and the two must not be confused with each
/// other for the sake of a defensive parse.
fn split_content_range(value: &str) -> Option<(&str, &str)> {
    let (unit, rest) = value
        .trim()
        .split_once(|c: char| c == ' ' || c == '\t' || c == '=')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    Some((unit, rest.trim()))
}

/// Parse `Content-Range: bytes {start}-{end}/{total}`.
///
/// Anything that does not fit that grammar — including a reversed span — yields
/// `None`, so the caller refuses the response rather than guessing where its
/// body begins.
fn parse_content_range(value: &str) -> Option<ContentRange> {
    let (_unit, rest) = split_content_range(value)?;
    let (span, total) = rest.split_once('/')?;
    let (start, end) = span.trim().split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    if end < start {
        return None;
    }
    let total: Option<u64> = match total.trim() {
        "*" => None,
        digits => Some(digits.parse::<u64>().ok()?),
    };
    Some(ContentRange { start, end, total })
}

/// Extract the object length from the `Content-Range: bytes */{length}` that a
/// `416` carries, so the hard wall can be described in bytes rather than only
/// in status codes.
fn parse_unsatisfied_length(value: &str) -> Option<u64> {
    let (_unit, rest) = split_content_range(value)?;
    rest.strip_prefix("*/")?.trim().parse().ok()
}

/// Render an advertised total for a diagnostic (`*` when unknown).
fn describe_total(total: Option<u64>) -> String {
    match total {
        Some(bytes) => bytes.to_string(),
        None => "*".to_string(),
    }
}

/// Decide how many bytes one top-up response may append, or why it must not be
/// appended at all.
///
/// `start` is how many bytes the staging file already holds, and therefore the
/// offset the response body has to begin at for the append to mean anything.
/// `max_bytes` is the per-call allowance, which is *not* the number of bytes
/// still missing: the caller re-sends the whole chunk on every round.
fn plan_append(
    status: StatusCode,
    content_range: Option<&str>,
    content_length: Option<u64>,
    start: u64,
    max_bytes: u64,
) -> Result<u64, Reject> {
    let code = status.as_u16();
    if max_bytes == 0 {
        return Err(Reject::Shape(format!(
            "a top-up at byte {start} was requested with a zero-byte allowance"
        )));
    }

    let (span, total) = if code == 206 {
        let Some(raw) = content_range else {
            // A 206 without a Content-Range says nothing about where its body
            // starts, and nothing downstream can recover that information.
            return Err(Reject::Shape(format!(
                "HTTP 206 with no Content-Range header, so the body cannot be confirmed to \
                 start at byte {start}; nothing was appended"
            )));
        };
        let Some(parsed) = parse_content_range(raw) else {
            return Err(Reject::Shape(format!(
                "HTTP 206 with an unusable Content-Range header {raw:?}, so the body cannot be \
                 confirmed to start at byte {start}; nothing was appended"
            )));
        };
        let total = parsed.total;
        (Some(parsed), total)
    } else if code == 200 && start == 0 {
        // The one status that is safe without a Content-Range: with an empty
        // staging file, "the whole object from byte 0" and "the range starting
        // at 0" are literally the same bytes, so there is nothing to misplace.
        // The total is still the object's full length, which keeps the clamp
        // below honest about where the object ends.
        (None, content_length)
    } else if code == 200 {
        // The edge ignored the range and answered with the whole object from
        // byte 0. Appending it would splice a second copy of the object's
        // opening onto its tail — corruption that still decodes, so it would
        // sail through the caller's decoded-duration check.
        return Err(Reject::Shape(format!(
            "HTTP 200 but the range was ignored - the edge answered with the whole object from \
             byte 0, and appending it would duplicate the object's start (the file already holds \
             {start} bytes)"
        )));
    } else {
        return Err(Reject::Shape(format!(
            "HTTP {code} is not a range response (206 is required), so nothing was appended"
        )));
    };

    if let Some(span) = &span {
        if span.start != start {
            return Err(Reject::Shape(format!(
                "HTTP 206 covers bytes {}-{} of {} but byte {start} was requested: the body does \
                 not start where the file ends, so appending it would corrupt the file",
                span.start,
                span.end,
                describe_total(span.total),
            )));
        }
    }

    match total {
        // Never write past the advertised end of the object. `max_bytes` is only
        // an allowance, so without this clamp a late round could append bytes
        // that the object does not contain at all.
        Some(total) if total <= start => Err(Reject::ObjectEnds(format!(
            "Content-Range advertises a {total}-byte object and the file already holds {start}: \
             there are no bytes left to request, so the window cannot be topped up"
        ))),
        Some(total) => Ok(max_bytes.min(total - start)),
        None => Ok(max_bytes),
    }
}

/// Append the next slice of the object to an already partially written file.
///
/// Returns the number of bytes appended, or a diagnostic string naming every
/// request shape that was tried. Three shapes are attempted because different
/// edges honour different mechanisms and a given URL gives no hint which one it
/// expects: the `range` query parameter plus the HTTP `Range` header, the header
/// alone, then the parameter alone.
///
/// A response is only written when its headers prove it is the requested slice:
/// see [`plan_append`]. Every header-level check runs before the staging file is
/// opened, so a rejected response cannot leave a single byte behind.
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
        let content_range = res
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if !status.is_success() {
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                // 416 is the one non-success status that is an *answer* rather
                // than a refusal: the object is finished. It is worded apart
                // from the range-validation failures below on purpose, because
                // "the server will not give us more bytes" and "the server
                // ignored our Range" call for opposite remedies, and the caller
                // shows this text to the user.
                let reason = match content_range.as_deref().and_then(parse_unsatisfied_length) {
                    Some(len) => format!("range not satisfiable: the object ends at {len} bytes"),
                    None => "range not satisfiable: the edge did not state the object length"
                        .to_string(),
                };
                warn!(
                    via = via,
                    offset = start,
                    "Range top-up hit a hard 416 wall"
                );
                failures.push(format!(
                    "{via}: HTTP {} ({reason}; no client-side resume can extend it)",
                    status.as_u16()
                ));
            } else {
                failures.push(format!(
                    "{via}: HTTP {} ({} bytes advertised)",
                    status.as_u16(),
                    res.content_length().unwrap_or(0)
                ));
            }
            continue;
        }

        // Still nothing written: the staging file is not even opened until the
        // headers have been cleared.
        let limit = match plan_append(
            status,
            content_range.as_deref(),
            res.content_length(),
            start,
            max_bytes,
        ) {
            Ok(limit) => limit,
            Err(Reject::Shape(why)) => {
                warn!(
                    via = via,
                    offset = start,
                    status = status.as_u16(),
                    "Rejected a range top-up response that was not the requested slice"
                );
                failures.push(format!("{via}: {why}"));
                continue;
            }
            Err(Reject::ObjectEnds(why)) => {
                warn!(
                    via = via,
                    offset = start,
                    status = status.as_u16(),
                    "Range top-up found the object already complete at the requested offset"
                );
                failures.push(format!("{via}: {why}"));
                // No shape can produce bytes the object does not contain.
                break;
            }
        };
        if limit < max_bytes {
            warn!(
                via = via,
                offset = start,
                allowance = max_bytes,
                limit = limit,
                "Clamping a range top-up to the advertised end of the object"
            );
        }

        let mut file = match tokio::fs::OpenOptions::new().append(true).open(dest).await {
            Ok(f) => f,
            Err(e) => return Err(format!("{via}: cannot open file for append: {e}")),
        };
        let mut appended: u64 = 0;
        let mut read_problem: Option<String> = None;
        while appended < limit {
            match tokio::time::timeout(Duration::from_secs(BODY_STALL_TIMEOUT_SECS), res.chunk())
                .await
            {
                Ok(Ok(Some(chunk))) => {
                    // A single chunk can overshoot the clamp, so trim it: bytes
                    // past the advertised end of the object must never reach the
                    // file, however eager the edge is.
                    let room = usize::try_from(limit - appended).unwrap_or(usize::MAX);
                    let take = chunk.len().min(room);
                    if take == 0 {
                        break;
                    }
                    if let Err(e) = file.write_all(&chunk[..take]).await {
                        read_problem = Some(format!("write failed: {e}"));
                        break;
                    }
                    appended += take as u64;
                    if take < chunk.len() {
                        // The clamp, not the server, ended this response. That is
                        // the whole point of the append, not a fault to report.
                        break;
                    }
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
    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// Bytes already in the staging file for the scripted-response tests: the
    /// file is *not* empty, so every response is validated against offset 10.
    const HELD: &[u8] = b"HELD-BYTES";

    /// A staging file pre-seeded with known bytes, in its own temp directory.
    struct Staging {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Staging {
        fn new(tag: &str, seed: &[u8]) -> Staging {
            let dir =
                std::env::temp_dir().join(format!("auralis_topup_{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            let path = dir.join("partial.m4a");
            std::fs::write(&path, seed).expect("seed staging file");
            Staging { dir, path }
        }

        fn bytes(&self) -> Vec<u8> {
            std::fs::read(&self.path).expect("read staging file")
        }
    }

    impl Drop for Staging {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A scripted HTTP/1.1 origin.
    ///
    /// It answers every request head with the matching scripted response and
    /// falls back to the last one, which is what the three request shapes need:
    /// all three must see the same misbehaviour for the loop to exhaust. Every
    /// head it saw is recorded so a test can assert what the client actually
    /// asked for — a scripted response on its own proves nothing about the
    /// request that provoked it.
    struct ScriptedServer {
        port: u16,
        seen: Arc<Mutex<Vec<String>>>,
        running: Arc<AtomicBool>,
        accept: Option<std::thread::JoinHandle<()>>,
    }

    impl ScriptedServer {
        fn start(responses: Vec<String>) -> ScriptedServer {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
            let port = listener.local_addr().expect("local_addr").port();
            listener
                .set_nonblocking(true)
                .expect("non-blocking listener");
            let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let running = Arc::new(AtomicBool::new(true));
            let responses = Arc::new(responses);
            let accept = {
                let seen = Arc::clone(&seen);
                let running = Arc::clone(&running);
                let responses = Arc::clone(&responses);
                std::thread::spawn(move || {
                    while running.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let conn = Connection {
                                    responses: Arc::clone(&responses),
                                    seen: Arc::clone(&seen),
                                    running: Arc::clone(&running),
                                };
                                std::thread::spawn(move || conn.serve(stream));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            Err(_) => break,
                        }
                    }
                })
            };
            ScriptedServer {
                port,
                seen,
                running,
                accept: Some(accept),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{}", self.port, path)
        }

        fn requests(&self) -> Vec<String> {
            self.seen.lock().expect("request log lock").clone()
        }
    }

    impl Drop for ScriptedServer {
        fn drop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(handle) = self.accept.take() {
                let _ = handle.join();
            }
        }
    }

    /// One accepted connection, which may carry all three request shapes.
    ///
    /// The script is reference-counted because each connection is handed to its
    /// own thread, and a thread has to be `'static`.
    struct Connection {
        responses: Arc<Vec<String>>,
        seen: Arc<Mutex<Vec<String>>>,
        running: Arc<AtomicBool>,
    }

    impl Connection {
        fn serve(self, mut stream: TcpStream) {
            // A read timeout keeps a connection thread from outliving the test
            // by much once the client has gone away.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut served = 0usize;
            while self.running.load(Ordering::Relaxed) {
                let head = match read_request_head(&mut stream) {
                    Ok(Some(head)) => head,
                    Ok(None) | Err(_) => return,
                };
                if let Ok(mut log) = self.seen.lock() {
                    log.push(head);
                }
                let Some(response) = self
                    .responses
                    .get(served)
                    .or_else(|| self.responses.last())
                else {
                    return;
                };
                served += 1;
                if stream.write_all(response.as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
            }
        }
    }

    /// Read a request head up to the blank line. `Ok(None)` means the peer closed
    /// the connection.
    fn read_request_head(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
        let mut buf: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if stream.read(&mut byte)? == 0 {
                return Ok(None);
            }
            buf.push(byte[0]);
            if buf.len() >= 4 && buf[buf.len() - 4..] == *b"\r\n\r\n" {
                return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
            }
            if buf.len() > 16 * 1024 {
                return Ok(None);
            }
        }
    }

    /// Build a raw response: `body` is appended verbatim after the blank line.
    ///
    /// Header values are taken as `String` so that every call site spells them
    /// out rather than borrowing a computed value whose lifetime would have to
    /// be reasoned about at each one.
    fn response(status_line: &str, headers: &[(&str, String)], body: &[u8]) -> String {
        let mut out = String::from(format!("{status_line}\r\n"));
        for (name, value) in headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str("\r\n");
        out.push_str(&String::from_utf8_lossy(body));
        out
    }

    /// True when a recorded request head carries an HTTP `Range` header.
    fn has_range_header(head: &str) -> bool {
        head.to_lowercase()
            .lines()
            .any(|line| line.trim_start().starts_with("range:"))
    }

    /// True when a recorded request target carries `range={start}-`.
    fn has_range_query(head: &str, start: u64) -> bool {
        head.lines()
            .next()
            .is_some_and(|request_line| request_line.contains(&format!("range={start}-")))
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("client")
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

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
    fn content_range_parses_a_known_total() {
        let parsed = parse_content_range("bytes 10-19/100").expect("a well-formed Content-Range");
        assert_eq!(parsed.start, 10, "start");
        assert_eq!(parsed.end, 19, "end");
        assert_eq!(parsed.total, Some(100), "total");
    }

    #[test]
    fn content_range_parses_the_unknown_total_form() {
        // `bytes 10-19/*` is legal: the length is not known, and the caller must
        // fall back to the allowance instead of inventing a total.
        let parsed = parse_content_range("bytes 10-19/*").expect("the asterisk total is legal");
        assert_eq!(parsed.total, None, "total");
        assert_eq!(
            parse_content_range("bytes=10-19/100").map(|c| c.total),
            Some(Some(100)),
            "the `bytes=` spelling some origins send must parse too"
        );
    }

    #[test]
    fn content_range_rejects_junk_and_reversed_spans() {
        for bad in [
            "items 10-19/100",
            "bytes 19-10/100",
            "bytes 10-19",
            "bytes ten-nineteen/100",
            "bytes 10-19/one-hundred",
        ] {
            assert!(
                parse_content_range(bad).is_none(),
                "must be rejected: {bad}"
            );
        }
    }

    #[test]
    fn unsatisfied_content_range_reports_the_object_length() {
        assert_eq!(parse_unsatisfied_length("bytes */4096"), Some(4096));
        assert_eq!(parse_unsatisfied_length("bytes */0"), Some(0));
        assert_eq!(parse_unsatisfied_length("bytes 10-19/4096"), None);
        assert_eq!(parse_unsatisfied_length("nonsense"), None);
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

    #[test]
    fn top_up_rejects_a_200_that_ignored_the_range_and_leaves_the_file_untouched() {
        // The corruption case. The edge answers a ranged request with `200` and
        // the whole object from byte 0. Appending it would put a second copy of
        // the object's opening on the end of the file — a file that never
        // existed, but that still decodes, so the caller's duration check would
        // accept it as a recovered download.
        let body = b"WHOLE-OBJECT-FROM-BYTE-ZERO";
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 200 OK",
            &[
                ("Content-Type", "audio/mp4".to_string()),
                ("Content-Length", body.len().to_string()),
            ],
            body,
        )]);
        let staging = Staging::new("ignored200", HELD);
        let client = test_client();
        let start = HELD.len() as u64;

        let err = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140&clen=999999"),
            &staging.path,
            start,
            TOPUP_CHUNK_BYTES,
        ))
        .expect_err("a 200 that ignored the range must never be appended");

        assert!(err.contains("200"), "the status must be reported: {err}");
        assert!(err.contains("the range was ignored"), "got {err}");
        assert!(err.contains("whole object from byte 0"), "got {err}");
        assert!(err.contains("duplicate the object's start"), "got {err}");
        assert_eq!(
            staging.bytes(),
            HELD,
            "the staging file must be byte-for-byte unchanged"
        );

        // The client did ask for a range, so this is the edge misbehaving.
        let seen = server.requests();
        assert_eq!(seen.len(), 3, "all three shapes must be tried: {seen:?}");
        assert!(has_range_query(&seen[0], start), "shape 1: {:?}", seen[0]);
        assert!(has_range_header(&seen[0]), "shape 1: {:?}", seen[0]);
        assert!(!has_range_query(&seen[1], start), "shape 2: {:?}", seen[1]);
        assert!(has_range_header(&seen[1]), "shape 2: {:?}", seen[1]);
        assert!(has_range_query(&seen[2], start), "shape 3: {:?}", seen[2]);
        assert!(!has_range_header(&seen[2]), "shape 3: {:?}", seen[2]);
    }

    #[test]
    fn top_up_rejects_a_206_whose_content_range_starts_elsewhere() {
        // A `206` for the wrong window is the same corruption hazard with a
        // friendlier status code, so it has to be refused as firmly.
        let body = b"BYTES-FROM-SOMEWHERE-ELSE-ENTIRELY";
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 206 Partial Content",
            &[
                ("Content-Type", "audio/mp4".to_string()),
                ("Content-Range", "bytes 4096-4127/8192".to_string()),
                ("Content-Length", body.len().to_string()),
            ],
            body,
        )]);
        let staging = Staging::new("wrong206", HELD);
        let client = test_client();
        let start = HELD.len() as u64;

        let err = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            start,
            TOPUP_CHUNK_BYTES,
        ))
        .expect_err("a 206 for another window must never be appended");

        assert!(err.contains("206"), "the status must be reported: {err}");
        assert!(
            err.contains("4096"),
            "the served offset must be reported: {err}"
        );
        assert!(
            err.contains(&format!("byte {start} was requested")),
            "the requested offset must be reported: {err}"
        );
        assert!(err.contains("corrupt"), "got {err}");
        assert_eq!(
            staging.bytes(),
            HELD,
            "the staging file must be byte-for-byte unchanged"
        );
    }

    #[test]
    fn top_up_rejects_a_206_without_a_usable_content_range() {
        // A `206` that will not say where its body starts is unusable, whether
        // the header is missing outright or simply unparsable.
        for (tag, headers) in [
            ("missing_cr", vec![("Content-Length", "4".to_string())]),
            (
                "junk_cr",
                vec![
                    ("Content-Range", "bytes nonsense/8192".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
            ),
        ] {
            let server = ScriptedServer::start(vec![response(
                "HTTP/1.1 206 Partial Content",
                &headers,
                b"NOPE",
            )]);
            let staging = Staging::new(tag, HELD);
            let client = test_client();
            let start = HELD.len() as u64;

            let err = block_on(top_up(
                &client,
                None,
                &server.url("/videoplayback?itag=140"),
                &staging.path,
                start,
                TOPUP_CHUNK_BYTES,
            ))
            .expect_err("a 206 without a usable Content-Range is not appendable");

            assert!(err.contains("Content-Range"), "{tag}: {err}");
            assert!(err.contains("nothing was appended"), "{tag}: {err}");
            assert_eq!(staging.bytes(), HELD, "{tag}: the file was modified");
        }
    }

    #[test]
    fn top_up_accepts_a_well_formed_206_for_the_requested_offset() {
        // The success path, pinned so the validation cannot drift into being
        // over-strict: a `206` that names exactly the offset the file ends at
        // must be appended in full.
        let body = b"NEXT-EIGHT";
        let start = HELD.len() as u64;
        let total = start + body.len() as u64;
        let content_range = format!("bytes {start}-{}/{}", total - 1, total);
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 206 Partial Content",
            &[
                ("Content-Type", "audio/mp4".to_string()),
                ("Content-Range", content_range),
                ("Content-Length", body.len().to_string()),
            ],
            body,
        )]);
        let staging = Staging::new("good206", HELD);
        let client = test_client();

        let added = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            start,
            TOPUP_CHUNK_BYTES,
        ))
        .expect("a well-formed 206 for the requested offset must be accepted");

        assert_eq!(added, body.len() as u64, "bytes appended");
        let mut expected = HELD.to_vec();
        expected.extend_from_slice(body);
        assert_eq!(
            staging.bytes(),
            expected,
            "the served bytes must be appended"
        );
    }

    #[test]
    fn top_up_clamps_an_append_to_the_advertised_object_end() {
        // The caller passes a 2 MiB *allowance*, not the number of bytes still
        // missing. This object ends 2 bytes from where the file stops, so only
        // those 2 bytes may be written even though the server keeps talking.
        let body = vec![b'X'; 64];
        let start = HELD.len() as u64;
        let total = start + 2;
        let content_range = format!("bytes {start}-{}/{}", total - 1, total);
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 206 Partial Content",
            &[
                ("Content-Range", content_range),
                ("Content-Length", body.len().to_string()),
            ],
            &body,
        )]);
        let staging = Staging::new("clamp_total", HELD);
        let client = test_client();

        let added = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            start,
            TOPUP_CHUNK_BYTES,
        ))
        .expect("a 206 that only over-delivers is still a 206");

        assert_eq!(added, 2, "the append must stop at the advertised end");
        let mut expected = HELD.to_vec();
        expected.extend_from_slice(&body[..2]);
        assert_eq!(
            staging.bytes(),
            expected,
            "nothing may be written past the advertised object end"
        );
    }

    #[test]
    fn top_up_clamps_an_append_to_the_requested_allowance() {
        // The mirror image: a generous advertised total, but the caller only
        // allows 8 more bytes, so 8 is the hard cap.
        let body = vec![b'Y'; 64];
        let start = HELD.len() as u64;
        let total = 1_000_000u64;
        let content_range = format!("bytes {start}-{}/{}", start + 63, total);
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 206 Partial Content",
            &[
                ("Content-Range", content_range),
                ("Content-Length", body.len().to_string()),
            ],
            &body,
        )]);
        let staging = Staging::new("clamp_allowance", HELD);
        let client = test_client();

        let added = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            start,
            8,
        ))
        .expect("a 206 for the requested offset is appendable");

        assert_eq!(added, 8, "the append must stop at the allowance");
        let mut expected = HELD.to_vec();
        expected.extend_from_slice(&body[..8]);
        assert_eq!(staging.bytes(), expected, "the file grew by the allowance");
    }

    #[test]
    fn top_up_stops_immediately_when_the_object_already_ends_at_the_offset() {
        // The object is exactly as long as the file already is. There is nothing
        // to ask for, so the remaining request shapes must not be spent: no
        // shape can produce bytes the object does not contain.
        let start = HELD.len() as u64;
        let content_range = format!("bytes {start}-{start}/{start}");
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 206 Partial Content",
            &[
                ("Content-Range", content_range),
                ("Content-Length", "1".to_string()),
            ],
            b"Z",
        )]);
        let staging = Staging::new("object_ends", HELD);
        let client = test_client();

        let err = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            start,
            TOPUP_CHUNK_BYTES,
        ))
        .expect_err("an object that ends here cannot be topped up");

        assert!(
            err.contains(&format!("a {start}-byte object")),
            "the advertised length must be reported: {err}"
        );
        assert!(err.contains("no bytes left to request"), "got {err}");
        assert_eq!(staging.bytes(), HELD, "the file was modified");
        let seen = server.requests();
        assert_eq!(
            seen.len(),
            1,
            "a hard stop must not spend the other shapes: {seen:?}"
        );
    }

    #[test]
    fn top_up_reports_a_416_as_a_hard_wall_and_leaves_the_file_untouched() {
        // 416 means the object is finished, which is the opposite diagnosis from
        // "the edge ignored our Range": the first cannot be recovered by
        // rotating clients, the second can. The two must stay distinguishable in
        // the text the caller shows the user.
        let start = HELD.len() as u64;
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 416 Range Not Satisfiable",
            &[
                ("Content-Range", format!("bytes */{start}")),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        )]);
        let staging = Staging::new("status416", HELD);
        let client = test_client();

        let err = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            start,
            TOPUP_CHUNK_BYTES,
        ))
        .expect_err("a 416 delivers no bytes");

        assert!(
            err.contains("416"),
            "the status code must be reported: {err}"
        );
        assert!(
            err.contains("range not satisfiable"),
            "the reason must be reported: {err}"
        );
        assert!(
            err.contains(&format!("the object ends at {start} bytes")),
            "the hard wall must be quantified: {err}"
        );
        // It must not read like a range-validation failure.
        assert!(
            !err.contains("the range was ignored"),
            "416 must not be worded as an ignored range: {err}"
        );
        assert!(
            !err.contains("206"),
            "416 must not be worded as a bad partial response: {err}"
        );
        assert!(
            err.contains("no client-side resume can extend it"),
            "the interpretation must survive: {err}"
        );
        assert_eq!(
            staging.bytes(),
            HELD,
            "the staging file must be byte-for-byte unchanged"
        );
        // All three shapes are still tried, so the joined text stays a complete
        // account of what was attempted.
        let seen = server.requests();
        assert_eq!(seen.len(), 3, "all three shapes must be tried: {seen:?}");
    }

    #[test]
    fn a_416_without_a_content_range_still_names_the_status() {
        // Not every edge states the object length; the diagnosis degrades to the
        // status code rather than disappearing.
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 416 Range Not Satisfiable",
            &[("Content-Length", "0".to_string())],
            b"",
        )]);
        let staging = Staging::new("status416_bare", HELD);
        let client = test_client();

        let err = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            HELD.len() as u64,
            TOPUP_CHUNK_BYTES,
        ))
        .expect_err("a 416 delivers no bytes");

        assert!(err.contains("416"), "got {err}");
        assert!(err.contains("range not satisfiable"), "got {err}");
        assert!(err.contains("did not state the object length"), "got {err}");
        assert_eq!(staging.bytes(), HELD, "the file was modified");
    }

    #[test]
    fn top_up_accepts_a_200_when_the_staging_file_is_still_empty() {
        // The deliberate exception: with nothing held, "the whole object from
        // byte 0" and "the range starting at 0" are the same bytes, so a 200 is
        // exactly what was asked for and must not be rejected.
        let body = b"START-OF-OBJECT";
        let server = ScriptedServer::start(vec![response(
            "HTTP/1.1 200 OK",
            &[
                ("Content-Type", "audio/mp4".to_string()),
                ("Content-Length", body.len().to_string()),
            ],
            body,
        )]);
        let staging = Staging::new("status200_empty", b"");
        let client = test_client();

        let added = block_on(top_up(
            &client,
            None,
            &server.url("/videoplayback?itag=140"),
            &staging.path,
            0,
            TOPUP_CHUNK_BYTES,
        ))
        .expect("a 200 at offset 0 is the requested range");

        assert_eq!(added, body.len() as u64, "bytes appended");
        assert_eq!(staging.bytes(), body.to_vec());
    }
}
