//! Fetches one googlevideo URL several ways and reports which ones are served.
//!
//! **Why this exists.** On 2026-10-02 the app 403'd on `Ral6kFSx7ZY` while
//! `curl` returned HTTP 206 for the same track, same class (muxed itag 18), same
//! phone, same residential line. Everything the two paths share has been ruled
//! out; what remains is everything they do NOT share:
//!
//!   - the HTTP client itself (`reqwest` vs `curl`), which means TLS and HTTP/2
//!     fingerprinting, and
//!   - the header set, since `inject_stream_headers` adds Referer, Origin,
//!     Accept, Accept-Language and Sec-Fetch-Mode that a bare curl does not send.
//!
//! Neither had ever been measured against the other. This does that, in one run,
//! on the same URL, in the same process, at the same time — which is the only
//! way the comparison means anything.
//!
//! Usage:
//!   cargo run --example fetch_probe -- <url> [video_id]
//!
//! The URL carries an `expire=` parameter and stops working after roughly six
//! hours, so it has to be freshly resolved. `/tmp/opencode/yts/prove.mjs` prints
//! a usable one.

use std::time::Duration;

/// What distinguishes each probe. Every one hits the same URL.
struct Probe {
    label: &'static str,
    /// The exact header set `inject_stream_headers` produces.
    app_headers: bool,
    /// Whether to send `Range: bytes=0-1023` (a 1 KiB slice, not the whole file).
    ranged: bool,
    /// Overrides the default UA when set.
    ua: Option<&'static str>,
}

const PROBES: &[Probe] = &[
    Probe {
        label: "A reqwest + app headers + Range   (what the app actually sends)",
        app_headers: true,
        ranged: true,
        ua: None,
    },
    Probe {
        label: "B reqwest + UA + Range           (no Referer/Origin/Sec-Fetch)",
        app_headers: false,
        ranged: true,
        ua: Some("curl/8.5.0"),
    },
    Probe {
        label: "C reqwest + bare, no UA, Range   (reqwest's own default)",
        app_headers: false,
        ranged: true,
        ua: None,
    },
    Probe {
        label: "D reqwest + app headers, NO Range (does Range change anything?)",
        app_headers: true,
        ranged: false,
        ua: None,
    },
    Probe {
        label: "E reqwest + HTTP/1.1 only + app headers (isolates the h2 fingerprint)",
        app_headers: true,
        ranged: true,
        ua: None,
    },
];

fn build_client(http1_only: bool) -> reqwest::Client {
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30));
    if http1_only {
        // reqwest's HTTP/2 ALPN negotiation is a fingerprint input, and curl on
        // this box negotiates HTTP/2 by default, so this probe is about removing
        // h2 specifically — the thing most likely to differ from curl in an
        // unexpected direction.
        b = b.http1_only();
    }
    b.build().expect("client builds")
}

/// Fetch the URL bound to a chosen local source address.
///
/// This is the experiment `ip=` predicts. The URL is signed for the address the
/// resolver egressed from; if the CDN checks that, then fetching the *same* URL
/// from a *different* local address must be refused. That is §4.7.16's tampering
/// result reproduced honestly — from the other direction, by moving the client
/// instead of editing the parameter.
///
/// Pass an address to bind. Pass `none` to leave the OS to choose.
async fn bind_probe(url: &str, bind: Option<std::net::IpAddr>) {
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30));
    if let Some(ip) = bind {
        b = b.local_address(ip);
    }
    let client = b.build().expect("client builds");
    let req = client.get(url).header("Range", "bytes=0-1023").header(
        "User-Agent",
        "com.google.android.apps.youtube.vr.oculus/1.65.0 (Linux; U; Android 12; en_US)",
    );
    match req.send().await {
        Ok(r) => println!(
            "  bind {:<40} -> HTTP {}",
            bind.map(|i| i.to_string())
                .unwrap_or_else(|| "(OS-chosen)".into()),
            r.status().as_u16()
        ),
        Err(e) => println!(
            "  bind {:<40} -> {} ({e})",
            bind.map(|i| i.to_string())
                .unwrap_or_else(|| "(OS-chosen)".into()),
            if e.is_connect() {
                "CONNECT FAILED"
            } else if e.is_timeout() {
                "TIMEOUT"
            } else {
                "SEND FAILED"
            }
        ),
    }
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let url = argv.first().cloned().unwrap_or_else(|| {
        eprintln!("usage: fetch_probe <url> [video_id] [--bind <addr,addr,...>]");
        std::process::exit(2);
    });
    let video = argv.get(1).cloned().unwrap_or_else(|| "(unknown)".into());

    let host = url
        .split("://")
        .nth(1)
        .and_then(|r| r.split(['/', '?', '#']).next())
        .unwrap_or("?");
    let bound_ip = url_param(&url, "ip").unwrap_or_else(|| "(none)".into());
    let itag = url_param(&url, "itag").unwrap_or_else(|| "(none)".into());

    println!("video   : {video}");
    println!("host    : {host}");
    println!("itag    : {itag}");
    println!("ip=     : {bound_ip}");
    println!();

    // `--bind a,b,c` re-runs the single request from each listed source address,
    // which is the whole point: one URL, several client identities.
    if let Some(pos) = argv.iter().position(|a| a == "--bind") {
        let list = argv.get(pos + 1).cloned().unwrap_or_default();
        println!("same URL, different client egress (only the ip= address should serve):");
        for a in list.split(',').filter(|s| !s.is_empty()) {
            bind_probe(&url, a.parse().ok()).await;
        }
        println!();
        println!("If exactly one row is 206 and the rest are 403, the binding is real AND");
        println!("the address matters — which is what `local_address` exists to satisfy.");
        return;
    }

    for (i, p) in PROBES.iter().enumerate() {
        let http1 = p.label.starts_with('E');
        let client = build_client(http1);
        let mut req = client.get(&url);
        if let Some(ua) = p.ua {
            req = req.header("User-Agent", ua);
        }
        if p.app_headers {
            // Mirrors inject_stream_headers() exactly.
            req = req
                .header("Referer", "https://www.youtube.com/")
                .header("Origin", "https://www.youtube.com")
                .header("Accept", "*/*")
                .header("Accept-Language", "en-US,en;q=0.9")
                .header("Sec-Fetch-Mode", "no-cors");
        }
        if p.ranged {
            req = req.header("Range", "bytes=0-1023");
        }

        let verdict = match req.send().await {
            Ok(r) => {
                let status = r.status().as_u16();
                // Own the header value before consuming `r` for the body, or the
                // borrow outlives the response.
                let len = r
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string();
                match r.bytes().await {
                    Ok(b) => format!("HTTP {status}  ({len} header, {} bytes body)", b.len()),
                    Err(e) => format!("HTTP {status}  (body read failed: {e})"),
                }
            }
            Err(e) => {
                let kind = if e.is_timeout() {
                    "TIMEOUT"
                } else if e.is_connect() {
                    "CONNECT FAILED"
                } else if e.is_request() {
                    "SEND FAILED"
                } else {
                    "OTHER"
                };
                format!("{kind}: {e}")
            }
        };
        println!("  {}  {verdict}", p.label);
        if i + 1 == PROBES.len() {
            break;
        }
    }

    println!();
    println!("Read it as:");
    println!(
        "  A 206 but B 403  -> it is the HEADERS. Referer/Origin/Sec-Fetch-Mode is the trigger."
    );
    println!(
        "  A 403 but B 206  -> it is the CLIENT. reqwest's TLS/h2 fingerprint is being refused,"
    );
    println!("                     and no header change will fix it.");
    println!("  A 403 and B 403   -> it is not reqwest and not headers; the URL is stale or the");
    println!("                     range/params are refused, and curl 206 on a FRESH url is the control.");
    println!("  C differs from A -> reqwest's own default User-Agent is part of the decision.");
}

/// Pull one query parameter out of a URL without a full parser.
fn url_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(percent_decode(v).unwrap_or_else(|| v.to_string()));
            }
        }
    }
    None
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
