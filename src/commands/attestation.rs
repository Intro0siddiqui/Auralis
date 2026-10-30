//! Attestation & PO Token Harvester
//!
//! Provides native WebView capture for YouTube Proof-of-Origin (PO) tokens
//! as outlined in AGENTS.md §4.7.7. When BotGuard emulation produces a cold-start
//! fallback, this harvester launches a hidden WebView to capture the authentic
//! PO token directly from YouTube's native watch page context.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
#[cfg(desktop)]
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::oneshot;
use tracing::info;
#[cfg(desktop)]
use tracing::warn;

/// Payload received when a PO token is captured by the harvester hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestedPoToken {
    #[serde(rename = "videoId", alias = "video_id")]
    pub video_id: String,
    #[serde(rename = "poToken", alias = "po_token")]
    pub po_token: String,
    #[serde(rename = "visitorData", alias = "visitor_data", default)]
    pub visitor_data: Option<String>,
    #[serde(default)]
    pub source: String,
}

static PENDING_HARVESTS: LazyLock<Mutex<HashMap<String, oneshot::Sender<HarvestedPoToken>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// IPC handler called by the injected harvester hook inside the hidden WebView.
#[tauri::command]
pub async fn record_harvested_po_token(payload: HarvestedPoToken) -> Result<(), String> {
    info!(
        video_id = %payload.video_id,
        token_len = payload.po_token.len(),
        "Received harvested PO token from WebView hook"
    );

    let mut map = PENDING_HARVESTS
        .lock()
        .map_err(|e| format!("Lock error: {e}"))?;

    // Try finding by exact video_id or matching any single pending harvest if video_id was empty
    let sender = map.remove(&payload.video_id).or_else(|| {
        if map.len() == 1 {
            let key = map.keys().next().cloned()?;
            map.remove(&key)
        } else {
            None
        }
    });

    if let Some(tx) = sender {
        let _ = tx.send(payload);
    }

    Ok(())
}

/// Spawns a hidden WebView to load YouTube watch page and harvest an authentic PO token.
#[tauri::command]
pub async fn harvest_po_token(
    #[allow(unused_variables)] app: AppHandle,
    video_id: String,
) -> Result<HarvestedPoToken, String> {
    info!(video_id = %video_id, "Initiating PO token harvest via hidden WebView");

    #[cfg(desktop)]
    let (tx, rx) = oneshot::channel();
    #[cfg(not(desktop))]
    let (tx, _rx) = oneshot::channel();
    {
        let mut map = PENDING_HARVESTS
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        map.insert(video_id.clone(), tx);
    }

    #[cfg(desktop)]
    {
        use tauri::WebviewUrl;
        use tauri::WebviewWindowBuilder;

        let url_str = format!("https://www.youtube.com/watch?v={}", video_id);
        let parsed_url: tauri::Url = url_str.parse().map_err(|e| format!("Invalid URL: {e}"))?;
        let window_label = format!("po-harvester-{}", uuid::Uuid::new_v4());

        let hook_script = r#"
            (() => {
                function notify(poToken, visitorData, vid) {
                    if (!poToken) return;
                    try {
                        const payload = {
                            videoId: vid || window.location.search.replace(/.*v=([^&]+).*/, '$1') || '',
                            poToken: poToken,
                            visitorData: visitorData || (window.ytcfg ? window.ytcfg.get('VISITOR_DATA') : null),
                            source: 'webview_harvest'
                        };
                        if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
                            window.__TAURI_INTERNALS__.invoke('record_harvested_po_token', { payload });
                        }
                    } catch (_) {}
                }

                const _origFetch = window.fetch;
                window.fetch = async function(...args) {
                    const [res, cfg] = args;
                    if (cfg && cfg.body && typeof cfg.body === 'string') {
                        try {
                            const parsed = JSON.parse(cfg.body);
                            const pot = parsed?.serviceIntegrityDimensions?.poToken;
                            if (pot) notify(pot, null, parsed?.videoId);
                        } catch (_) {}
                    }
                    return _origFetch.apply(this, args);
                };

                const _origSend = XMLHttpRequest.prototype.send;
                XMLHttpRequest.prototype.send = function(body) {
                    if (body && typeof body === 'string') {
                        try {
                            const parsed = JSON.parse(body);
                            const pot = parsed?.serviceIntegrityDimensions?.poToken;
                            if (pot) notify(pot, null, parsed?.videoId);
                        } catch (_) {}
                    }
                    return _origSend.call(this, body);
                };
            })();
        "#;

        let window =
            WebviewWindowBuilder::new(&app, &window_label, WebviewUrl::External(parsed_url))
                .visible(false)
                .initialization_script(hook_script)
                .build()
                .map_err(|e| format!("Failed to create harvester window: {e}"))?;

        // Wait up to 15 seconds for token capture
        let result = tokio::time::timeout(Duration::from_secs(15), rx).await;

        // Clean up hidden window immediately
        let _ = window.destroy();

        // Clean up pending map if timed out
        if let Ok(mut map) = PENDING_HARVESTS.lock() {
            map.remove(&video_id);
        }

        match result {
            Ok(Ok(token)) => {
                info!(video_id = %video_id, "Successfully harvested PO token via hidden WebView");
                Ok(token)
            }
            Ok(Err(_)) => Err("Harvest channel dropped without token".to_string()),
            Err(_) => {
                warn!(video_id = %video_id, "PO token harvest timed out after 15s");
                Err("PO token harvest timed out".to_string())
            }
        }
    }

    #[cfg(not(desktop))]
    {
        // On non-desktop targets (Android), fallback gracefully until native Android WebView is hooked
        if let Ok(mut map) = PENDING_HARVESTS.lock() {
            map.remove(&video_id);
        }
        Err("Native WebView harvester currently enabled on desktop platforms".to_string())
    }
}
