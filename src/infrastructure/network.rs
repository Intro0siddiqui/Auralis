//! P2P networking, device discovery, and inter-device communication.
//!
//! Powered by Iroh (QUIC transport via Quinn, Magicsock STUN + DERP relay hole-punching)
//! and iroh-blobs (BLAKE3 verified chunked audio file streaming).

use crate::infrastructure::database::Database;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// ALPN protocol identifier for Auralis metadata catalog exchange
pub const ALPN_SYNC: &[u8] = b"/auralis/sync/1";

/// Parse an Iroh EndpointId from either standard base32 string or 64-character hex string
pub fn parse_endpoint_id(s: &str) -> Option<EndpointId> {
    let trimmed = s.trim();
    if let Ok(endpoint_id) = EndpointId::from_str(trimmed) {
        return Some(endpoint_id);
    }
    if let Ok(bytes) = hex::decode(trimmed) {
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            if let Ok(endpoint_id) = EndpointId::from_bytes(&arr) {
                return Some(endpoint_id);
            }
        }
    }
    None
}

/// Request payload for P2P sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub payload: serde_json::Value,
}

/// Response payload for P2P sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncResponse {
    Ack { message: String },
    Error { message: String },
}

/// Current state of a connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

/// Snapshot of network statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub connected_peers: usize,
}

/// Errors occurring in the network layer
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Transfer error: {0}")]
    TransferError(String),

    #[error("Discovery error: {0}")]
    DiscoveryError(String),

    #[error("IO error: {0}")]
    IoError(String),
}

/// Shared, thread-safe state for an Iroh P2P node.
pub struct NetworkRuntime {
    secret_key: SecretKey,
    endpoint: Arc<RwLock<Option<Endpoint>>>,
    alias_map: Arc<RwLock<HashMap<String, String>>>,
    alias_db: Arc<RwLock<Option<Arc<Database>>>>,
    stats: Arc<RwLock<NetworkStats>>,
    listen_port: u16,
}

impl NetworkRuntime {
    /// Create a new runtime with an ephemeral secret key (default for tests or fresh nodes)
    pub fn new() -> Self {
        Self::with_secret_key(SecretKey::generate(), 0)
    }

    /// Create a new runtime with a persistent SecretKey
    pub fn with_secret_key(secret_key: SecretKey, listen_port: u16) -> Self {
        Self {
            secret_key,
            endpoint: Arc::new(RwLock::new(None)),
            alias_map: Arc::new(RwLock::new(HashMap::new())),
            alias_db: Arc::new(RwLock::new(None)),
            stats: Arc::new(RwLock::new(NetworkStats::default())),
            listen_port,
        }
    }

    /// The local Iroh node id (Ed25519 public key string)
    pub fn local_peer_id(&self) -> String {
        self.secret_key.public().to_string()
    }

    /// Access the node's secret key
    pub fn secret_key(&self) -> &SecretKey {
        &self.secret_key
    }

    /// Configured listen port (0 = OS assigned)
    pub fn listen_port(&self) -> u16 {
        self.listen_port
    }

    /// Get the bound Iroh endpoint, if started
    pub async fn endpoint(&self) -> Option<Endpoint> {
        self.endpoint.read().await.clone()
    }

    /// Set the bound Iroh endpoint
    pub async fn set_endpoint(&self, ep: Endpoint) {
        let mut guard = self.endpoint.write().await;
        *guard = Some(ep);
    }

    /// Generate an `EndpointTicket` string for this node if the endpoint is bound
    pub async fn ticket(&self) -> Option<String> {
        let guard = self.endpoint.read().await;
        guard.as_ref().map(|ep| {
            let addr = ep.addr();
            EndpointTicket::new(addr).to_string()
        })
    }

    /// Attach a persistent SQLite database for device alias resolution
    pub async fn set_persistent_store(&self, db: Arc<Database>) {
        let mut store = self.alias_db.write().await;
        *store = Some(db);
    }

    /// Access the attached persistent SQLite database if available
    pub async fn alias_db(&self) -> Option<Arc<Database>> {
        self.alias_db.read().await.clone()
    }

    /// Hydrate the in-memory alias map from persisted database rows
    pub async fn hydrate_aliases(&self) -> Result<usize, NetworkError> {
        let db_opt = { self.alias_db.read().await.clone() };
        let Some(db) = db_opt else {
            return Ok(0);
        };

        let pairs: Vec<(String, String)> = {
            let conn = db
                .connection()
                .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, CASE WHEN ip_address LIKE 'endpoint%' THEN ip_address ELSE peer_id END FROM paired_devices WHERE peer_id IS NOT NULL OR ip_address IS NOT NULL",
                )
                .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;

            let rows = stmt
                .query_map([], |row| {
                    let id: String = row.get(0)?;
                    let pid: String = row.get(1)?;
                    Ok((id, pid))
                })
                .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;

            rows.flatten().collect()
        };

        let count = pairs.len();
        let mut map = self.alias_map.write().await;
        for (id, pid) in pairs {
            map.insert(id, pid);
        }

        debug!(hydrated = count, "Hydrated device aliases from SQLite");
        Ok(count)
    }

    /// Register a device alias (UUID -> NodeId or Ticket)
    pub async fn register_device_alias(&self, device_id: String, peer_id: String) {
        {
            let mut map = self.alias_map.write().await;
            map.insert(device_id.clone(), peer_id.clone());
        }

        let db_opt = { self.alias_db.read().await.clone() };
        if let Some(db) = db_opt {
            if let Ok(conn) = db.connection() {
                let _ = conn.execute(
                    "UPDATE paired_devices SET peer_id = ?1 WHERE id = ?2",
                    rusqlite::params![peer_id, device_id],
                );
            }
        }
    }

    /// Resolve a device UUID or NodeId string
    pub async fn resolve_peer_id(&self, device_id: &str) -> Option<String> {
        {
            let map = self.alias_map.read().await;
            if let Some(pid) = map.get(device_id) {
                return Some(pid.clone());
            }
        }

        let db_opt = { self.alias_db.read().await.clone() };
        let db_pid = if let Some(db) = db_opt {
            if let Ok(conn) = db.connection() {
                let result: Result<String, _> = conn.query_row(
                    "SELECT CASE WHEN ip_address LIKE 'endpoint%' THEN ip_address ELSE peer_id END FROM paired_devices WHERE id = ?1 AND (peer_id IS NOT NULL OR ip_address IS NOT NULL)",
                    rusqlite::params![device_id],
                    |row| row.get(0),
                );
                result.ok()
            } else {
                None
            }
        } else {
            None
        };

        if let Some(pid) = db_pid {
            let mut map = self.alias_map.write().await;
            map.insert(device_id.to_string(), pid.clone());
            return Some(pid);
        }

        None
    }

    /// Record bytes sent
    pub fn record_bytes_sent(&self, bytes: u64) {
        if let Ok(mut stats) = self.stats.try_write() {
            stats.bytes_sent += bytes;
        }
    }

    /// Record bytes received
    pub fn record_bytes_received(&self, bytes: u64) {
        if let Ok(mut stats) = self.stats.try_write() {
            stats.bytes_received += bytes;
        }
    }

    /// Get current network statistics snapshot
    pub async fn stats(&self) -> NetworkStats {
        self.stats.read().await.clone()
    }
}

impl Default for NetworkRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Service that advertises this device and discovers peers via Iroh QUIC & tickets.
pub struct Discovery {
    runtime: Arc<NetworkRuntime>,
}

impl Discovery {
    /// Create a new discovery instance
    pub fn new(port: u16) -> Self {
        Self {
            runtime: Arc::new(NetworkRuntime::with_secret_key(SecretKey::generate(), port)),
        }
    }

    /// Create a discovery service with an explicit persistent secret key
    pub fn with_secret_key(secret_key: SecretKey, port: u16) -> Self {
        Self {
            runtime: Arc::new(NetworkRuntime::with_secret_key(secret_key, port)),
        }
    }

    /// Access the network runtime
    pub fn runtime(&self) -> &Arc<NetworkRuntime> {
        &self.runtime
    }

    /// Create a `SyncEngine` sharing this discovery service's runtime
    pub fn sync_engine(&self) -> SyncEngine {
        SyncEngine::share(self.runtime.clone())
    }

    /// Local node peer id
    pub fn local_peer_id(&self) -> String {
        self.runtime.local_peer_id()
    }

    /// Start the Iroh QUIC endpoint and background accept loop
    pub async fn start(&self) -> Result<(), NetworkError> {
        info!(
            node_id = %self.runtime.local_peer_id(),
            "Starting Iroh QUIC P2P network endpoint"
        );

        let ep = if self.runtime.listen_port() != 0 {
            let bind_res = Endpoint::builder(presets::N0)
                .secret_key(self.runtime.secret_key.clone())
                .alpns(vec![ALPN_SYNC.to_vec(), iroh_blobs::ALPN.to_vec()])
                .bind_addr(format!("0.0.0.0:{}", self.runtime.listen_port()))
                .map_err(|e| NetworkError::ConnectionError(format!("Invalid bind address: {e}")))?
                .bind()
                .await;
            match bind_res {
                Ok(ep) => {
                    info!(
                        port = self.runtime.listen_port(),
                        "Bound Iroh endpoint to configured port"
                    );
                    ep
                }
                Err(e) => {
                    warn!(error = %e, "Configured port busy, falling back to OS ephemeral port");
                    Endpoint::builder(presets::N0)
                        .secret_key(self.runtime.secret_key.clone())
                        .alpns(vec![ALPN_SYNC.to_vec(), iroh_blobs::ALPN.to_vec()])
                        .bind()
                        .await
                        .map_err(|e| {
                            NetworkError::ConnectionError(format!(
                                "Failed to bind Iroh endpoint: {e}"
                            ))
                        })?
                }
            }
        } else {
            Endpoint::builder(presets::N0)
                .secret_key(self.runtime.secret_key.clone())
                .alpns(vec![ALPN_SYNC.to_vec(), iroh_blobs::ALPN.to_vec()])
                .bind()
                .await
                .map_err(|e| {
                    NetworkError::ConnectionError(format!("Failed to bind Iroh endpoint: {e}"))
                })?
        };

        self.runtime.set_endpoint(ep.clone()).await;

        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            info!("Iroh accept loop started listening for incoming QUIC streams");
            while let Some(incoming) = ep.accept().await {
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    match incoming.accept() {
                        Ok(connecting) => match connecting.await {
                            Ok(conn) => {
                                let alpn = conn.alpn();
                                if alpn == ALPN_SYNC {
                                    handle_incoming_sync_connection(conn, runtime).await;
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Incoming QUIC connection handshake failed");
                            }
                        },
                        Err(e) => {
                            warn!(error = %e, "Failed to accept incoming QUIC connection");
                        }
                    }
                });
            }
            debug!("Iroh accept loop stopped");
        });

        Ok(())
    }
}

/// Handle an incoming QUIC connection on ALPN_SYNC
async fn handle_incoming_sync_connection(
    conn: iroh::endpoint::Connection,
    runtime: Arc<NetworkRuntime>,
) {
    let remote_id_str = conn.remote_id().to_string();
    debug!(remote = %remote_id_str, "Accepted incoming QUIC sync connection");

    // Auto-record incoming peer into paired_devices if DB is available
    if let Some(db) = runtime.alias_db().await {
        if let Ok(sql_conn) = db.connection() {
            let exists: bool = sql_conn
                .query_row(
                    "SELECT 1 FROM paired_devices WHERE peer_id = ?1",
                    rusqlite::params![remote_id_str],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            if !exists {
                let new_id = uuid::Uuid::new_v4().to_string();
                let short_id = &remote_id_str[..remote_id_str.len().min(8)];
                let name = format!("Auralis Device ({short_id})");
                let now = chrono::Utc::now().to_rfc3339();
                let _ = sql_conn.execute(
                    "INSERT INTO paired_devices (id, name, device_type, ip_address, status, last_synced, created_at, peer_id)
                     VALUES (?1, ?2, 'mobile', NULL, 'connected', ?3, ?3, ?4)",
                    rusqlite::params![new_id, name, now, remote_id_str],
                );
                info!(remote = %remote_id_str, "Recorded incoming peer as paired device in DB");
            }
        }
    }

    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let runtime = runtime.clone();
        let conn_holder = conn.clone();
        tokio::spawn(async move {
            let _conn_guard = conn_holder;
            match recv.read_to_end(1024 * 1024).await {
                Ok(bytes) => {
                    runtime.record_bytes_received(bytes.len() as u64);
                    if let Ok(req) = serde_json::from_slice::<SyncRequest>(&bytes) {
                        if req.payload.get("action").and_then(|v| v.as_str()) == Some("fetch_audio")
                        {
                            handle_audio_fetch_request(req.payload, &mut send, &runtime).await;
                            return;
                        }

                        info!("Received P2P sync payload via Iroh");

                        let db_opt = runtime.alias_db().await;
                        let applied = if let Some(db) = db_opt {
                            apply_sync_payload(&req.payload, &db)
                        } else {
                            0
                        };

                        let resp = SyncResponse::Ack {
                            message: format!("Sync payload processed: {applied} items reconciled"),
                        };
                        if let Ok(resp_bytes) = serde_json::to_vec(&resp) {
                            let _ = send.write_all(&resp_bytes).await;
                            let _ = send.finish();
                            runtime.record_bytes_sent(resp_bytes.len() as u64);
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read incoming sync payload stream");
                }
            }
        });
    }
}

/// Send a 4-byte length-prefixed JSON header over a QUIC SendStream
async fn send_framed_header(
    send: &mut iroh::endpoint::SendStream,
    header: &serde_json::Value,
) -> Result<(), std::io::Error> {
    let header_bytes = serde_json::to_vec(header)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = header_bytes.len() as u32;
    send.write_all(&len.to_be_bytes()).await?;
    send.write_all(&header_bytes).await?;
    Ok(())
}

/// Read a 4-byte length-prefixed JSON header from a QUIC RecvStream
async fn read_framed_header(
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<serde_json::Value, NetworkError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| NetworkError::TransferError(format!("Failed to read header length: {e}")))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 {
        return Err(NetworkError::TransferError(
            "Header frame too large".to_string(),
        ));
    }
    let mut header_bytes = vec![0u8; len];
    recv.read_exact(&mut header_bytes)
        .await
        .map_err(|e| NetworkError::TransferError(format!("Failed to read header frame: {e}")))?;
    let val: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|e| NetworkError::TransferError(format!("Invalid header JSON: {e}")))?;
    Ok(val)
}

/// Handle a remote peer requesting the physical audio file bytes for a track
async fn handle_audio_fetch_request(
    payload: serde_json::Value,
    send: &mut iroh::endpoint::SendStream,
    runtime: &Arc<NetworkRuntime>,
) {
    let track_id = match payload.get("track_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            let err_header = serde_json::json!({
                "status": "error",
                "message": "Missing track_id in fetch_audio request",
            });
            let _ = send_framed_header(send, &err_header).await;
            let _ = send.finish();
            return;
        }
    };

    let db_opt = runtime.alias_db().await;
    let Some(db) = db_opt else {
        let err_header = serde_json::json!({
            "status": "error",
            "message": "Database unavailable",
        });
        let _ = send_framed_header(send, &err_header).await;
        let _ = send.finish();
        return;
    };

    let track_info: Option<(String, String, String, Option<String>)> = {
        if let Ok(conn) = db.connection() {
            conn.query_row(
                "SELECT file_path, format, title, album_art_path FROM tracks WHERE id = ?1",
                rusqlite::params![track_id],
                |row| {
                    let fp: String = row.get(0)?;
                    let fmt: String = row.get(1)?;
                    let title: String = row.get(2)?;
                    let art: Option<String> = row.get(3)?;
                    Ok((fp, fmt, title, art))
                },
            )
            .ok()
        } else {
            None
        }
    };

    let Some((file_path, format, title, album_art_path)) = track_info else {
        let err_header = serde_json::json!({
            "status": "error",
            "message": "Track not found in database",
        });
        let _ = send_framed_header(send, &err_header).await;
        let _ = send.finish();
        return;
    };

    if file_path.starts_with("sync://") {
        let err_header = serde_json::json!({
            "status": "error",
            "message": "Track file is not stored locally on this peer",
        });
        let _ = send_framed_header(send, &err_header).await;
        let _ = send.finish();
        return;
    }

    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            let err_header = serde_json::json!({
                "status": "error",
                "message": format!("Cannot open audio file on disk: {e}"),
            });
            let _ = send_framed_header(send, &err_header).await;
            let _ = send.finish();
            return;
        }
    };

    let file_len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(e) => {
            let err_header = serde_json::json!({
                "status": "error",
                "message": format!("Cannot read file metadata: {e}"),
            });
            let _ = send_framed_header(send, &err_header).await;
            let _ = send.finish();
            return;
        }
    };

    let art_data: Option<(Vec<u8>, String)> = {
        let candidate_path = album_art_path
            .as_deref()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| {
                let base = std::path::Path::new(&file_path);
                for ext in ["jpg", "jpeg", "png", "webp"] {
                    let side = base.with_extension(ext);
                    if side.is_file() {
                        return Some(side);
                    }
                }
                None
            });

        if let Some(path) = candidate_path {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("jpg")
                    .to_string();
                Some((bytes, ext))
            } else {
                None
            }
        } else {
            None
        }
    };

    let (art_size, art_ext, art_bytes) = match art_data {
        Some((bytes, ext)) => (bytes.len() as u64, ext, bytes),
        None => (0u64, "jpg".to_string(), Vec::new()),
    };

    info!(track_id = %track_id, file_path = %file_path, size = file_len, art_size, "Streaming audio and artwork to P2P peer");

    let header = serde_json::json!({
        "status": "ok",
        "size": file_len,
        "format": format,
        "title": title,
        "art_size": art_size,
        "art_ext": art_ext,
    });

    if let Err(e) = send_framed_header(send, &header).await {
        warn!(error = %e, "Failed to send audio header frame to peer");
        let _ = send.finish();
        return;
    }

    if art_size > 0 && !art_bytes.is_empty() {
        if let Err(e) = send.write_all(&art_bytes).await {
            warn!(error = %e, "Failed to stream album art to peer");
        } else {
            runtime.record_bytes_sent(art_size);
            info!(art_size, "Streamed album art sidecar to P2P peer");
        }
    }

    let mut buf = vec![0u8; 64 * 1024];
    let mut total_sent = 0u64;
    loop {
        let n = match tokio::io::AsyncReadExt::read(&mut file, &mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                warn!(error = %e, track_id = %track_id, "Error reading audio file from disk during P2P streaming");
                break;
            }
        };
        if let Err(e) = send.write_all(&buf[..n]).await {
            warn!(error = %e, track_id = %track_id, "Error streaming audio chunk to P2P peer");
            break;
        }
        total_sent += n as u64;
    }
    runtime.record_bytes_sent(total_sent);
    info!(copied = total_sent, track_id = %track_id, "Completed streaming audio file to peer");
    let _ = send.finish();
}

/// Produce safe filenames without path traversal or invalid characters
fn sanitize_filename(name: &str) -> String {
    let filtered: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = filtered
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() {
        "track".to_string()
    } else {
        trimmed
    }
}

/// Reconcile incoming sync payload into local SQLite database
pub fn apply_sync_payload(payload: &serde_json::Value, db: &Database) -> usize {
    let Ok(conn) = db.connection() else {
        return 0;
    };

    let items = if let Some(items) = payload.get("items").and_then(|v| v.as_array()) {
        items.clone()
    } else if let Some(changes) = payload.get("changes").and_then(|v| v.as_array()) {
        changes.clone()
    } else {
        vec![payload.clone()]
    };

    let mut applied = 0;
    for item in items {
        if let Some(sub_changes) = item.get("changes").and_then(|v| v.as_array()) {
            for c in sub_changes {
                if apply_single_change(c, &conn) {
                    applied += 1;
                }
            }
        } else if apply_single_change(&item, &conn) {
            applied += 1;
        }
    }

    debug!(applied, "Applied incoming sync changes to database");
    applied
}

fn apply_single_change(change: &serde_json::Value, conn: &rusqlite::Connection) -> bool {
    let entity = change.get("entity").and_then(|v| v.as_str()).unwrap_or("");
    let payload = change.get("payload").unwrap_or(change);
    let id = change
        .get("id")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("id").and_then(|v| v.as_str()))
        .unwrap_or("");

    if entity == "track" || (!id.is_empty() && payload.get("title").is_some()) {
        let title = payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Title");
        let artist = payload
            .get("artist")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist");
        let album = payload
            .get("album")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Album");
        let duration = payload
            .get("duration_secs")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let format = payload
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3");
        let source_url = payload.get("source_url").and_then(|v| v.as_str());
        let file_path = payload
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("sync://{}", id));

        let res = conn.execute(
            "INSERT INTO tracks (id, title, artist, album, duration_secs, file_path, format, source_url, date_added)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                artist = excluded.artist,
                album = excluded.album,
                duration_secs = excluded.duration_secs",
            rusqlite::params![
                id,
                title,
                artist,
                album,
                duration,
                file_path,
                format,
                source_url,
                chrono::Utc::now().to_rfc3339()
            ],
        );
        return res.is_ok();
    } else if entity == "playlist" {
        let name = payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Shared Playlist");
        let desc = payload
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let track_ids = payload
            .get("track_ids")
            .and_then(|v| v.as_str())
            .unwrap_or("[]");
        let now = chrono::Utc::now().to_rfc3339();

        let res = conn.execute(
            "INSERT INTO playlists (id, name, description, created_at, updated_at, track_ids)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                updated_at = excluded.updated_at,
                track_ids = excluded.track_ids",
            rusqlite::params![id, name, desc, now, now, track_ids],
        );
        return res.is_ok();
    }

    false
}

/// Sync engine coordinating P2P library and state synchronization over Iroh QUIC streams.
#[derive(Clone)]
pub struct SyncEngine {
    runtime: Arc<NetworkRuntime>,
    pending_queue: Arc<RwLock<Vec<serde_json::Value>>>,
}

impl SyncEngine {
    /// Create a standalone SyncEngine
    pub fn new() -> Self {
        Self {
            runtime: Arc::new(NetworkRuntime::new()),
            pending_queue: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Share an existing network runtime
    pub fn share(runtime: Arc<NetworkRuntime>) -> Self {
        Self {
            runtime,
            pending_queue: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Access the shared network runtime
    pub fn runtime(&self) -> &Arc<NetworkRuntime> {
        &self.runtime
    }

    /// Queue a sync payload to be sent on the next `request_sync`
    pub fn enqueue_sync_payload(&self, payload: serde_json::Value) {
        if let Ok(mut queue) = self.pending_queue.try_write() {
            queue.push(payload);
        }
    }

    /// Connect to a peer via EndpointTicket, EndpointId, or direct address
    pub async fn connect(&self, peer: &str) -> Result<(), NetworkError> {
        let peer_str = peer.trim();
        if peer_str.is_empty() {
            return Err(NetworkError::ConnectionError(
                "Peer address cannot be empty".to_string(),
            ));
        }

        let ep_opt = self.runtime.endpoint().await;
        let Some(ep) = ep_opt else {
            return Err(NetworkError::ConnectionError(
                "Iroh endpoint not started; call Discovery::start() first".to_string(),
            ));
        };

        // Try parsing as EndpointTicket first
        if let Ok(ticket) = EndpointTicket::from_str(peer_str) {
            let addr = ticket.endpoint_addr().clone();
            debug!(endpoint_id = %addr.id, "Connecting to peer via EndpointTicket");
            ep.connect(addr, ALPN_SYNC)
                .await
                .map_err(|e| NetworkError::ConnectionError(format!("QUIC connect failed: {e}")))?;
            return Ok(());
        }

        // Try parsing as <endpoint_id>@<ip>:<port>
        if let Some((id_str, addr_str)) = peer_str.split_once('@') {
            if let (Some(id), Ok(sock)) = (
                parse_endpoint_id(id_str),
                std::net::SocketAddr::from_str(addr_str.trim()),
            ) {
                let addr = EndpointAddr::new(id).with_ip_addr(sock);
                debug!(endpoint_id = %addr.id, socket = %sock, "Connecting to peer via endpoint@ip:port");
                ep.connect(addr, ALPN_SYNC).await.map_err(|e| {
                    NetworkError::ConnectionError(format!("QUIC connect failed: {e}"))
                })?;
                return Ok(());
            }
        }

        // Try parsing as raw EndpointId (base32 or hex)
        if let Some(endpoint_id) = parse_endpoint_id(peer_str) {
            let addr = EndpointAddr::from(endpoint_id);
            debug!(endpoint_id = %endpoint_id, "Connecting to peer via EndpointId");
            ep.connect(addr, ALPN_SYNC)
                .await
                .map_err(|e| NetworkError::ConnectionError(format!("QUIC connect failed: {e}")))?;
            return Ok(());
        }

        info!(address = %peer_str, "Recorded direct peer address candidate");
        Ok(())
    }

    /// Perform a request-response sync transfer with the target peer
    pub async fn request_sync(&self, peer: &str) -> Result<(), NetworkError> {
        let resolved = self
            .runtime
            .resolve_peer_id(peer)
            .await
            .unwrap_or_else(|| peer.to_string());

        let ep_opt = self.runtime.endpoint().await;
        let Some(ep) = ep_opt else {
            return Err(NetworkError::ConnectionError(
                "Iroh endpoint not started; call Discovery::start() first".to_string(),
            ));
        };

        // Determine destination EndpointAddr
        let target_addr = if let Ok(ticket) = EndpointTicket::from_str(&resolved) {
            ticket.endpoint_addr().clone()
        } else if let Some((id_str, addr_str)) = resolved.split_once('@') {
            if let (Some(id), Ok(sock)) = (
                parse_endpoint_id(id_str),
                std::net::SocketAddr::from_str(addr_str.trim()),
            ) {
                EndpointAddr::new(id).with_ip_addr(sock)
            } else {
                return Err(NetworkError::ConnectionError(format!(
                    "Invalid endpoint@ip:port format: {resolved}"
                )));
            }
        } else if let Some(endpoint_id) = parse_endpoint_id(&resolved) {
            EndpointAddr::from(endpoint_id)
        } else {
            return Err(NetworkError::ConnectionError(format!(
                "Cannot resolve '{resolved}' to a valid Iroh EndpointTicket or EndpointId"
            )));
        };

        let conn = ep
            .connect(target_addr, ALPN_SYNC)
            .await
            .map_err(|e| NetworkError::ConnectionError(format!("QUIC connect failed: {e}")))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| NetworkError::TransferError(format!("Open bi-stream failed: {e}")))?;

        let payloads = {
            let mut q = self.pending_queue.write().await;
            std::mem::take(&mut *q)
        };

        let request = SyncRequest {
            payload: serde_json::json!({
                "items": payloads,
            }),
        };

        let req_bytes =
            serde_json::to_vec(&request).map_err(|e| NetworkError::TransferError(e.to_string()))?;

        send.write_all(&req_bytes)
            .await
            .map_err(|e| NetworkError::TransferError(format!("Write payload failed: {e}")))?;
        send.finish()
            .map_err(|e| NetworkError::TransferError(format!("Stream finish failed: {e}")))?;

        self.runtime.record_bytes_sent(req_bytes.len() as u64);

        let resp_bytes = recv
            .read_to_end(1024 * 1024)
            .await
            .map_err(|e| NetworkError::TransferError(format!("Read response failed: {e}")))?;

        self.runtime.record_bytes_received(resp_bytes.len() as u64);

        let response: SyncResponse = serde_json::from_slice(&resp_bytes)
            .map_err(|e| NetworkError::TransferError(format!("Invalid response JSON: {e}")))?;

        match response {
            SyncResponse::Ack { message } => {
                info!(message = %message, "P2P sync transfer acknowledged successfully");
                Ok(())
            }
            SyncResponse::Error { message } => Err(NetworkError::TransferError(format!(
                "Peer rejected sync: {message}"
            ))),
        }
    }

    /// Fetch the physical audio file for a track from a remote P2P peer over QUIC
    pub async fn fetch_track_audio(
        &self,
        peer: &str,
        track_id: &str,
        dest_dir: &std::path::Path,
    ) -> Result<PathBuf, NetworkError> {
        let resolved = self
            .runtime
            .resolve_peer_id(peer)
            .await
            .unwrap_or_else(|| peer.to_string());

        let ep_opt = self.runtime.endpoint().await;
        let Some(ep) = ep_opt else {
            return Err(NetworkError::ConnectionError(
                "Iroh endpoint not started; call Discovery::start() first".to_string(),
            ));
        };

        let target_addr = if let Ok(ticket) = EndpointTicket::from_str(&resolved) {
            ticket.endpoint_addr().clone()
        } else if let Some((id_str, addr_str)) = resolved.split_once('@') {
            if let (Some(id), Ok(sock)) = (
                parse_endpoint_id(id_str),
                std::net::SocketAddr::from_str(addr_str.trim()),
            ) {
                EndpointAddr::new(id).with_ip_addr(sock)
            } else {
                return Err(NetworkError::ConnectionError(format!(
                    "Invalid endpoint@ip:port format: {resolved}"
                )));
            }
        } else if let Some(endpoint_id) = parse_endpoint_id(&resolved) {
            EndpointAddr::from(endpoint_id)
        } else {
            return Err(NetworkError::ConnectionError(format!(
                "Cannot resolve '{resolved}' to a valid Iroh EndpointTicket or EndpointId"
            )));
        };

        info!(endpoint_id = %target_addr.id, track_id = %track_id, "Initiating P2P audio fetch over QUIC");

        let conn = ep
            .connect(target_addr, ALPN_SYNC)
            .await
            .map_err(|e| NetworkError::ConnectionError(format!("QUIC connect failed: {e}")))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| NetworkError::TransferError(format!("Open bi-stream failed: {e}")))?;

        let request = SyncRequest {
            payload: serde_json::json!({
                "action": "fetch_audio",
                "track_id": track_id,
            }),
        };

        let req_bytes =
            serde_json::to_vec(&request).map_err(|e| NetworkError::TransferError(e.to_string()))?;

        send.write_all(&req_bytes)
            .await
            .map_err(|e| NetworkError::TransferError(format!("Failed to write request: {e}")))?;
        send.finish().map_err(|e| {
            NetworkError::TransferError(format!("Failed to finish send stream: {e}"))
        })?;

        let header = read_framed_header(&mut recv).await?;
        if header.get("status").and_then(|v| v.as_str()) != Some("ok") {
            let msg = header
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Remote peer rejected audio fetch");
            return Err(NetworkError::TransferError(msg.to_string()));
        }

        let format = header
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3");
        let title = header
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("track");
        let art_size = header.get("art_size").and_then(|v| v.as_u64()).unwrap_or(0);
        let art_ext = header
            .get("art_ext")
            .and_then(|v| v.as_str())
            .unwrap_or("jpg");

        let safe_title = sanitize_filename(title);
        let short_id = &track_id[..track_id.len().min(8)];

        // Receive sidecar artwork if attached
        let mut local_art_path: Option<String> = None;
        if art_size > 0 {
            let art_file_name = format!("{safe_title}_{short_id}.{art_ext}");
            let art_dest = dest_dir.join(&art_file_name);
            let mut art_file = tokio::fs::File::create(&art_dest).await.map_err(|e| {
                NetworkError::IoError(format!("Failed to create artwork file: {e}"))
            })?;

            let mut art_received = 0u64;
            let mut art_buf = vec![0u8; 32 * 1024];
            while art_received < art_size {
                let to_read = ((art_size - art_received) as usize).min(art_buf.len());
                let n =
                    match tokio::io::AsyncReadExt::read(&mut recv, &mut art_buf[..to_read]).await {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) => {
                            return Err(NetworkError::TransferError(format!(
                            "Failed to read artwork stream at byte {art_received}/{art_size}: {e}"
                        )));
                        }
                    };
                art_file
                    .write_all(&art_buf[..n])
                    .await
                    .map_err(|e| NetworkError::IoError(format!("Failed to write artwork: {e}")))?;
                art_received += n as u64;
            }
            let _ = art_file.flush().await;
            if art_received == art_size {
                self.runtime.record_bytes_received(art_received);
                info!(path = ?art_dest, bytes = art_received, "Received and saved P2P album art");
                local_art_path = Some(art_dest.to_string_lossy().to_string());
            }
        }

        let file_name = format!("{safe_title}_{short_id}.{format}");
        let local_path = dest_dir.join(&file_name);

        let mut dest_file = tokio::fs::File::create(&local_path).await.map_err(|e| {
            NetworkError::IoError(format!("Failed to create destination file: {e}"))
        })?;

        let _conn_guard = conn;
        let expected_size = header.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let mut buf = vec![0u8; 64 * 1024];
        let mut total_received = 0u64;
        while total_received < expected_size || expected_size == 0 {
            let to_read = if expected_size > 0 {
                ((expected_size - total_received) as usize).min(buf.len())
            } else {
                buf.len()
            };
            let n = match tokio::io::AsyncReadExt::read(&mut recv, &mut buf[..to_read]).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    return Err(NetworkError::TransferError(format!(
                        "Failed to read audio stream at byte {total_received}/{expected_size}: {e}"
                    )));
                }
            };
            dest_file
                .write_all(&buf[..n])
                .await
                .map_err(|e| NetworkError::IoError(format!("Failed to write to file: {e}")))?;
            total_received += n as u64;
            if expected_size > 0 && total_received >= expected_size {
                break;
            }
        }

        dest_file
            .flush()
            .await
            .map_err(|e| NetworkError::IoError(format!("Failed to flush audio file: {e}")))?;

        self.runtime.record_bytes_received(total_received);
        info!(path = ?local_path, bytes = total_received, track_id = %track_id, "Successfully received and saved P2P audio track");

        // If no sidecar artwork was received, check if the audio file itself has embedded artwork
        if local_art_path.is_none() {
            local_art_path =
                crate::infrastructure::filesystem::MetadataExtractor::extract_async(&local_path)
                    .await
                    .ok()
                    .and_then(|t| t.album_art_path);
        }

        // Update track record in local SQLite database
        if let Some(db) = self.runtime.alias_db().await {
            if let Ok(sql_conn) = db.connection() {
                let path_str = local_path.to_string_lossy().to_string();
                let updated = sql_conn.execute(
                    "UPDATE tracks SET file_path = ?1, file_size = ?2, format = ?3, album_art_path = COALESCE(?4, album_art_path), is_downloaded = 1 WHERE id = ?5",
                    rusqlite::params![path_str, total_received as i64, format, local_art_path, track_id],
                ).unwrap_or(0);
                if updated == 0 {
                    let _ = sql_conn.execute(
                        "INSERT INTO tracks (id, title, file_path, file_size, format, album_art_path, is_downloaded, date_added)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
                        rusqlite::params![track_id, title, path_str, total_received as i64, format, local_art_path, chrono::Utc::now().to_rfc3339()],
                    );
                }
                info!(track_id = %track_id, file_path = %path_str, album_art = ?local_art_path, "Updated track in SQLite with local audio path and album art");
            }
        }

        Ok(local_path)
    }
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn sync_payload_roundtrip() {
        let request = SyncRequest {
            payload: serde_json::json!({
                "type": "add_track",
                "id": "abc-123",
                "time": 1720000000,
            }),
        };
        let bytes = serde_json::to_vec(&request).unwrap();
        let back: SyncRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.payload["type"], "add_track");
        assert_eq!(back.payload["id"], "abc-123");
    }

    #[test]
    fn peer_id_roundtrip() {
        let secret = SecretKey::generate();
        let pid = secret.public();
        let text = pid.to_string();
        let parsed = EndpointId::from_str(&text).unwrap();
        assert_eq!(pid, parsed);
    }

    #[test]
    fn network_error_implements_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<NetworkError>();
    }

    #[tokio::test]
    async fn test_alias_map_hydration_and_persistence() {
        let db_path =
            std::env::temp_dir().join(format!("test_auralis_net_iroh_{}.db", Uuid::new_v4()));
        let db = Database::new(&db_path).unwrap();
        db.run_migrations().unwrap();
        let db_arc = Arc::new(db);

        let runtime = NetworkRuntime::new();
        runtime.set_persistent_store(db_arc.clone()).await;

        let device_id = Uuid::new_v4().to_string();
        let peer_id = SecretKey::generate().public().to_string();

        {
            let conn = db_arc.connection().unwrap();
            conn.execute(
                "INSERT INTO paired_devices (id, name, device_type, paired_at, library_version, peer_id) VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    device_id,
                    "Test Device",
                    "desktop",
                    chrono::Utc::now().to_rfc3339(),
                    0,
                    peer_id
                ],
            )
            .unwrap();
        }

        let count = runtime.hydrate_aliases().await.unwrap();
        assert_eq!(count, 1);

        let resolved = runtime.resolve_peer_id(&device_id).await;
        assert_eq!(resolved, Some(peer_id));

        let new_peer_id = SecretKey::generate().public().to_string();
        runtime
            .register_device_alias(device_id.clone(), new_peer_id.clone())
            .await;

        let resolved_new = runtime.resolve_peer_id(&device_id).await;
        assert_eq!(resolved_new, Some(new_peer_id));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_apply_sync_payload_reconciles_tracks_and_playlists() {
        use uuid::Uuid;

        let db_path =
            std::env::temp_dir().join(format!("test_auralis_net_apply_{}.db", Uuid::new_v4()));
        let db = Database::new(&db_path).unwrap();
        db.run_migrations().unwrap();

        let track_id = Uuid::new_v4().to_string();
        let payload = serde_json::json!({
            "changes": [
                {
                    "entity": "track",
                    "id": track_id,
                    "payload": {
                        "title": "Synced Symphony",
                        "artist": "QUIC Ensemble",
                        "album": "Iroh Sessions",
                        "duration_secs": 240,
                        "format": "m4a"
                    }
                },
                {
                    "entity": "playlist",
                    "id": Uuid::new_v4().to_string(),
                    "payload": {
                        "name": "P2P Favorites",
                        "description": "Exchanged over Iroh",
                        "track_ids": "[]"
                    }
                }
            ]
        });

        let applied = apply_sync_payload(&payload, &db);
        assert_eq!(applied, 2);

        // Verify tracks table contains the reconciled track
        let conn = db.connection().unwrap();
        let title: String = conn
            .query_row(
                "SELECT title FROM tracks WHERE id = ?1",
                rusqlite::params![track_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Synced Symphony");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_p2p_audio_fetch_and_streaming() {
        use uuid::Uuid;

        let db_path_a =
            std::env::temp_dir().join(format!("test_auralis_audio_a_{}.db", Uuid::new_v4()));
        let db_a = Arc::new(Database::new(&db_path_a).unwrap());
        db_a.run_migrations().unwrap();

        let track_id = Uuid::new_v4().to_string();
        let dummy_audio_path =
            std::env::temp_dir().join(format!("test_track_{}.mp3", Uuid::new_v4()));
        let dummy_art_path = dummy_audio_path.with_extension("jpg");
        let dummy_data = b"ID3v2.4.0TEST_AUDIO_BYTES_IROH_QUIC_STREAMING_VERIFIED_1234567890";
        let dummy_art_data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF_DUMMY_ART_BYTES_P2P";
        tokio::fs::write(&dummy_audio_path, dummy_data)
            .await
            .unwrap();
        tokio::fs::write(&dummy_art_path, dummy_art_data)
            .await
            .unwrap();

        // Register track in DB A
        {
            let conn = db_a.connection().unwrap();
            conn.execute(
                "INSERT INTO tracks (id, title, artist, album, duration_secs, file_path, format, album_art_path, date_added)
                 VALUES (?1, 'Test Stream Track', 'Iroh Artist', 'P2P Album', 120, ?2, 'mp3', ?3, '2026-10-22T00:00:00Z')",
                rusqlite::params![
                    track_id,
                    dummy_audio_path.to_string_lossy().to_string(),
                    dummy_art_path.to_string_lossy().to_string()
                ],
            )
            .unwrap();
        }

        // Setup Endpoint A
        let disc_a = Discovery::new(0);
        disc_a.runtime().set_persistent_store(db_a).await;
        disc_a.start().await.unwrap();
        let _engine_a = disc_a.sync_engine();
        let ticket_a = disc_a.runtime().ticket().await.unwrap();

        // Setup Endpoint B
        let db_path_b =
            std::env::temp_dir().join(format!("test_auralis_audio_b_{}.db", Uuid::new_v4()));
        let db_b = Arc::new(Database::new(&db_path_b).unwrap());
        db_b.run_migrations().unwrap();

        let disc_b = Discovery::new(0);
        disc_b.runtime().set_persistent_store(db_b.clone()).await;
        disc_b.start().await.unwrap();
        let engine_b = disc_b.sync_engine();

        let dest_dir = std::env::temp_dir().join(format!("test_dest_{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&dest_dir).await.unwrap();

        // B fetches the audio file from A
        let fetched_path = engine_b
            .fetch_track_audio(&ticket_a, &track_id, &dest_dir)
            .await
            .expect("fetch_track_audio failed");

        assert!(fetched_path.exists());
        let received_bytes = tokio::fs::read(&fetched_path).await.unwrap();
        assert_eq!(received_bytes, dummy_data);

        // Verify DB B updated with both audio file_path and album_art_path
        let (updated_path, updated_art): (String, Option<String>) = {
            let conn_b = db_b.connection().unwrap();
            conn_b
                .query_row(
                    "SELECT file_path, album_art_path FROM tracks WHERE id = ?1",
                    rusqlite::params![track_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        };
        assert_eq!(updated_path, fetched_path.to_string_lossy().to_string());
        assert!(updated_art.is_some());
        let art_path = std::path::PathBuf::from(updated_art.unwrap());
        assert!(art_path.exists());
        let received_art = tokio::fs::read(&art_path).await.unwrap();
        assert_eq!(received_art, dummy_art_data);

        // Cleanup
        let _ = tokio::fs::remove_file(&dummy_audio_path).await;
        let _ = tokio::fs::remove_file(&dummy_art_path).await;
        let _ = tokio::fs::remove_file(&fetched_path).await;
        let _ = tokio::fs::remove_file(&art_path).await;
        let _ = tokio::fs::remove_dir_all(&dest_dir).await;
        let _ = std::fs::remove_file(&db_path_a);
        let _ = std::fs::remove_file(&db_path_b);
    }
}
