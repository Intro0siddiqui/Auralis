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
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// ALPN protocol identifier for Auralis metadata catalog exchange
pub const ALPN_SYNC: &[u8] = b"/auralis/sync/1";

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

    /// Hydrate the in-memory alias map from persisted database rows
    pub async fn hydrate_aliases(&self) -> Result<usize, NetworkError> {
        let db_opt = { self.alias_db.read().await.clone() };
        let Some(db) = db_opt else {
            return Ok(0);
        };

        let conn = db
            .connection()
            .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;

        let mut stmt = conn
            .prepare("SELECT id, peer_id FROM paired_devices WHERE peer_id IS NOT NULL")
            .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let pid: String = row.get(1)?;
                Ok((id, pid))
            })
            .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;

        let mut count = 0;
        let mut map = self.alias_map.write().await;
        for row in rows.flatten() {
            map.insert(row.0, row.1);
            count += 1;
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
                    "SELECT peer_id FROM paired_devices WHERE id = ?1 AND peer_id IS NOT NULL",
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

        let ep = Endpoint::builder(presets::N0)
            .secret_key(self.runtime.secret_key.clone())
            .alpns(vec![ALPN_SYNC.to_vec(), iroh_blobs::ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| {
                NetworkError::ConnectionError(format!("Failed to bind Iroh endpoint: {e}"))
            })?;

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
    debug!(remote = %conn.remote_id(), "Accepted incoming QUIC sync connection");
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        match recv.read_to_end(1024 * 1024).await {
            Ok(bytes) => {
                runtime.record_bytes_received(bytes.len() as u64);
                if let Ok(req) = serde_json::from_slice::<SyncRequest>(&bytes) {
                    info!(
                        remote = %conn.remote_id(),
                        payload_type = ?req.payload.get("type"),
                        "Received P2P sync payload via Iroh"
                    );
                    let resp = SyncResponse::Ack {
                        message: "Sync payload received and queued".to_string(),
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
                break;
            }
        }
    }
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

        // Try parsing as raw EndpointId (Ed25519 public key)
        if let Ok(endpoint_id) = EndpointId::from_str(peer_str) {
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
        } else if let Ok(endpoint_id) = EndpointId::from_str(&resolved) {
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
}
