//! Network Infrastructure Module
//!
//! P2P networking, device discovery, and inter-device communication.
//! Implemented on top of libp2p (mdns for LAN discovery, gossipsub for
//! change broadcast, request-response for direct data transfer).
//!
//! The swarm runs inside a single background tokio task. Commands are
//! pushed to it over an mpsc channel and events are handled inline, which
//! keeps the mDNS/gossipsub/request-response machinery off the async-trait
//! interface that callers (Tauri commands) see.

use futures::StreamExt;
use libp2p::core::{ConnectedPoint, Multiaddr};
use libp2p::identity::Keypair;
use libp2p::request_response;
use libp2p::swarm::{Swarm, SwarmEvent};
use libp2p::{gossipsub, mdns};
use libp2p::{PeerId, StreamProtocol};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Protocol name used for direct peer-to-peer sync transfers.
pub const SYNC_PROTOCOL_NAME: &str = "/auralis/sync/1";
/// Gossipsub topic used to broadcast change announcements to the mesh.
pub const SYNC_TOPIC: &str = "auralis/sync";

/// A sync payload request, serialized with `serde_json` on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub payload: serde_json::Value,
}

/// Reply to a [`SyncRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub ok: bool,
    pub message: String,
}

/// Combined libp2p behaviour used by every Auralis node.
///
/// The derive macro generates the `SwarmBehaviourEvent` enum aggregating the
/// events of all three sub-behaviours.
#[derive(libp2p::swarm::NetworkBehaviour)]
pub struct SwarmBehaviour {
    mdns: mdns::tokio::Behaviour,
    gossipsub: gossipsub::Behaviour,
    request_response: request_response::json::Behaviour<SyncRequest, SyncResponse>,
}

/// Commands sent from the public API to the background swarm task.
#[allow(dead_code)]
enum NetworkCommand {
    Dial {
        peer_id: Option<PeerId>,
        address: Multiaddr,
    },
    RequestSync {
        peer_id: PeerId,
        request: SyncRequest,
        reply: oneshot::Sender<Result<(), NetworkError>>,
    },
    Publish {
        topic: String,
        data: Vec<u8>,
    },
    Shutdown,
}

/// Shared, thread-safe state for a libp2p node.
///
/// A single runtime backs both the [`Discovery`] and (optionally) a
/// [`SyncEngine`] so that mDNS-discovered peers can be dialed and used for
/// request-response sync over the same swarm.
pub struct NetworkRuntime {
    keypair: Keypair,
    local_peer_id: PeerId,
    peers: Arc<RwLock<HashMap<PeerId, Vec<Multiaddr>>>>,
    connections: Arc<RwLock<HashMap<PeerId, ConnectionState>>>,
    command_tx: TokioMutex<Option<mpsc::Sender<NetworkCommand>>>,
    task: StdMutex<Option<JoinHandle<()>>>,
    stats: StdMutex<NetworkStats>,
    last_received: RwLock<Option<SyncRequest>>,
    last_gossip: RwLock<Option<Vec<u8>>>,
    /// UUID (PairedDevice.id) -> real libp2p PeerId mapping.
    ///
    /// Kept in-memory for fast runtime lookups (`RwLock`) but hydrated from
    /// the `paired_devices.peer_id` column on startup and persisted on every
    /// `register_device_alias` when a persistent store is attached. This fixes
    /// the HIGH deficiency where aliases were lost on restart.
    device_aliases: RwLock<HashMap<String, PeerId>>,
    /// Optional persistent backing for aliases (`paired_devices.peer_id`).
    /// `None` means in-memory only (e.g., before `init_network` wires the DB).
    alias_db: RwLock<Option<Arc<crate::infrastructure::database::Database>>>,
}

impl NetworkRuntime {
    fn new() -> Self {
        let keypair = Keypair::generate_ed25519();
        let local_peer_id = keypair.public().to_peer_id();
        Self {
            keypair,
            local_peer_id,
            peers: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            command_tx: TokioMutex::new(None),
            task: StdMutex::new(None),
            stats: StdMutex::new(NetworkStats::default()),
            last_received: RwLock::new(None),
            last_gossip: RwLock::new(None),
            device_aliases: RwLock::new(HashMap::new()),
            alias_db: RwLock::new(None),
        }
    }

    /// Attach a persistent store for alias hydration/persistence.
    /// Call once after DB is ready; eagerly hydrates existing aliases.
    pub async fn set_persistent_store(&self, db: Arc<crate::infrastructure::database::Database>) {
        *self.alias_db.write().await = Some(db.clone());
        // best-effort hydrate; failures are logged but not fatal
        if let Err(e) = self.hydrate_aliases_from_db(&db).await {
            warn!(error = %e, "Failed to hydrate device aliases from DB");
        }
    }

    /// Load all `peer_id` values from `paired_devices` into the in-memory map.
    /// Keeps the `RwLock` for runtime speed but survives restarts.
    pub async fn hydrate_aliases_from_db(
        &self,
        db: &crate::infrastructure::database::Database,
    ) -> Result<usize, NetworkError> {
        // Use try_connection where appropriate (MEDIUM): non-blocking attempt
        // for this background hydration; fallback to blocking lock if contended.
        // Collect into vec first so we don't hold the MutexGuard across an await.
        let pairs: Vec<(String, String)> = {
            // Prefer try_connection to avoid blocking the async runtime.
            let conn_guard = match db.try_connection() {
                Some(g) => g,
                None => db
                    .connection()
                    .map_err(|e| NetworkError::ConnectionError(e.to_string()))?,
            };
            // Gracefully handle missing column (pre-migration DBs)
            let mut stmt = match conn_guard
                .prepare("SELECT id, peer_id FROM paired_devices WHERE peer_id IS NOT NULL AND peer_id != ''")
            {
                Ok(s) => s,
                Err(e) => {
                    debug!(error = %e, "paired_devices.peer_id column missing; skipping hydration (migration will add it)");
                    return Ok(0);
                }
            };
            let rows = stmt
                .query_map([], |row| {
                    let id: String = row.get(0)?;
                    let pid: Option<String> = row.get(1)?;
                    Ok((id, pid))
                })
                .map_err(|e| NetworkError::ConnectionError(e.to_string()))?;
            let mut out = Vec::new();
            for r in rows {
                if let Ok((id, Some(pid_str))) = r {
                    // Validate peer_id eagerly; skip malformed entries
                    if PeerId::from_str(&pid_str).is_ok() {
                        out.push((id, pid_str));
                    } else {
                        warn!(device_id = %id, "Skipping invalid peer_id in DB");
                    }
                }
            }
            out
        }; // MutexGuard dropped here, before await

        let mut count = 0usize;
        if !pairs.is_empty() {
            let mut map = self.device_aliases.write().await;
            for (id, pid_str) in pairs {
                if let Ok(pid) = PeerId::from_str(&pid_str) {
                    map.insert(id.clone(), pid);
                    map.insert(id.to_ascii_lowercase(), pid);
                    count += 1;
                }
            }
        }
        info!(count, "Hydrated device aliases from DB");
        Ok(count)
    }

    /// Public helper to hydrate from the attached store (if any).
    pub async fn hydrate_aliases(&self) -> Result<usize, NetworkError> {
        let db = self.alias_db.read().await.clone();
        if let Some(db) = db {
            self.hydrate_aliases_from_db(&db).await
        } else {
            Ok(0)
        }
    }

    /// The local libp2p peer id.
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Remembers `addr` as a reachable address of `peer`.
    async fn add_address(&self, peer_id: PeerId, addr: Multiaddr) {
        let mut peers = self.peers.write().await;
        let addrs = peers.entry(peer_id).or_default();
        if !addrs.contains(&addr) {
            addrs.push(addr);
        }
    }

    /// Drops an expired `addr` for `peer`.
    async fn remove_address(&self, peer_id: PeerId, addr: &Multiaddr) {
        let mut peers = self.peers.write().await;
        if let Some(addrs) = peers.get_mut(&peer_id) {
            addrs.retain(|a| a != addr);
        }
    }

    async fn addresses_of(&self, peer_id: &PeerId) -> Vec<Multiaddr> {
        self.peers
            .read()
            .await
            .get(peer_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn set_connection(&self, peer_id: PeerId, state: ConnectionState) {
        let mut conns = self.connections.write().await;
        conns.insert(peer_id, state);
    }

    /// Register an in-memory mapping from a PairedDevice UUID string to its
    /// real libp2p PeerId discovered via mDNS/gossip. Call after pairing or
    /// when a device announces `device_id -> peer_id` over gossipsub.
    ///
    /// Persists to `paired_devices.peer_id` when a persistent store is attached
    /// (keeps `RwLock` for runtime but survives restarts). Uses `try_connection`
    /// where appropriate (MEDIUM) to avoid blocking the async runtime.
    pub async fn register_device_alias(&self, device_id: String, peer_id: PeerId) {
        // Keep in-memory map hydrated (both original and lowercased for UUID case-insensitivity)
        {
            let mut map = self.device_aliases.write().await;
            map.insert(device_id.clone(), peer_id);
            map.insert(device_id.to_ascii_lowercase(), peer_id);
        }

        // Persist to DB if attached
        let db_opt = self.alias_db.read().await.clone();
        if let Some(db) = db_opt {
            let peer_str = peer_id.to_string();
            // Brief blocking section: never hold across await
            let result = {
                // Prefer non-blocking try_connection for persistence
                let conn_guard = if let Some(g) = db.try_connection() {
                    Some(g)
                } else {
                    match db.connection() {
                        Ok(g) => Some(g),
                        Err(e) => {
                            warn!(error = %e, "Failed to acquire DB connection for alias persistence");
                            None
                        }
                    }
                };
                if let Some(conn) = conn_guard {
                    // UPDATE handles existing paired_devices rows; no INSERT needed
                    // (pairing creates the row via SyncRepository). Use both cases.
                    let res = conn.execute(
                        "UPDATE paired_devices SET peer_id = ?1 WHERE id = ?2 OR lower(id) = lower(?2)",
                        rusqlite::params![peer_str, device_id],
                    );
                    match res {
                        Ok(0) => {
                            debug!(device_id = %device_id, "No paired_devices row to persist peer_id (device not yet paired)");
                            Ok::<_, String>(())
                        }
                        Ok(_) => Ok(()),
                        Err(e) => Err(e.to_string()),
                    }
                } else {
                    Err("DB contention".to_string())
                }
            };
            if let Err(e) = result {
                warn!(device_id = %device_id, error = %e, "Failed to persist device alias");
            } else {
                debug!(device_id = %device_id, peer_id = %peer_str, "Persisted device alias");
            }
        }
    }

    /// Resolve a `device_id` (UUID string) or a raw PeerId string to a real
    /// PeerId. Checks the alias table first, then the DB (if alias missing),
    /// then tries to parse as PeerId. DB fallback ensures `request_sync` works
    /// after restarts even before the in-memory map is hydrated for a peer.
    pub async fn resolve_peer_id(&self, id: &str) -> Option<PeerId> {
        // 1. In-memory check
        {
            let map = self.device_aliases.read().await;
            if let Some(pid) = map.get(id) {
                return Some(*pid);
            }
            let lower = id.to_ascii_lowercase();
            if lower != id {
                if let Some(pid) = map.get(&lower) {
                    return Some(*pid);
                }
            }
        }
        // 2. DB fallback (if persistent store attached) — enables survival across restarts
        if let Some(db) = self.alias_db.read().await.clone() {
            let db_result: Option<PeerId> = {
                // Scope the MutexGuard so it's dropped before any await
                let conn_guard = match db.try_connection() {
                    Some(g) => Some(g),
                    None => db.connection().ok(),
                };
                if let Some(conn) = conn_guard {
                    // Try exact then lowercased id via SQL lower()
                    let peer_opt: Option<String> = (|| {
                        let mut stmt = conn
                            .prepare(
                                "SELECT peer_id FROM paired_devices WHERE id = ?1 OR lower(id) = lower(?1) LIMIT 1",
                            )
                            .ok()?;
                        stmt.query_row(rusqlite::params![id], |row| row.get::<_, Option<String>>(0))
                            .ok()
                            .flatten()
                    })();
                    if let Some(s) = peer_opt {
                        PeerId::from_str(&s).ok()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(pid) = db_result {
                // Cache for next lookup
                let mut map = self.device_aliases.write().await;
                map.insert(id.to_string(), pid);
                map.insert(id.to_ascii_lowercase(), pid);
                return Some(pid);
            }
        }
        // 3. Raw PeerId string
        PeerId::from_str(id).ok()
    }

    /// Convenience: alias-aware addresses lookup — resolves `id` then fetches
    /// known multiaddrs.
    pub async fn addresses_of_alias(&self, id: &str) -> Vec<Multiaddr> {
        if let Some(pid) = self.resolve_peer_id(id).await {
            self.addresses_of(&pid).await
        } else {
            Vec::new()
        }
    }

    /// Builds the swarm behaviour from a fresh session and a command channel.
    /// The receiver is moved into the background task by the caller.
    #[allow(clippy::type_complexity)]
    fn build(
        &self,
        listen_port: u16,
    ) -> Result<
        (
            Swarm<SwarmBehaviour>,
            mpsc::Sender<NetworkCommand>,
            mpsc::Receiver<NetworkCommand>,
        ),
        NetworkError,
    > {
        let swarm = build_swarm(&self.keypair, listen_port)?;
        let (tx, rx) = mpsc::channel(64);
        Ok((swarm, tx, rx))
    }

    /// Returns the discovered peers and their addresses.
    pub async fn discovered_peers(&self) -> Vec<(String, Vec<String>)> {
        let peers = self.peers.read().await;
        let mut out: Vec<(String, Vec<String>)> = peers
            .iter()
            .map(|(pid, addrs)| {
                (
                    pid.to_string(),
                    addrs.iter().map(|a| a.to_string()).collect(),
                )
            })
            .collect();
        out.sort();
        out
    }

    /// Returns the per-peer connection state for the UI.
    pub async fn connections(&self) -> Vec<(String, ConnectionState)> {
        let conns = self.connections.read().await;
        let mut out: Vec<(String, ConnectionState)> = conns
            .iter()
            .map(|(pid, state)| (pid.to_string(), *state))
            .collect();
        out.sort();
        out
    }

    /// Snapshot of network statistics.
    pub async fn stats(&self) -> NetworkStats {
        let mut stats = self.stats.lock().map(|g| g.clone()).unwrap_or_default();
        let conns = self.connections.read().await;
        stats.active_connections = conns
            .values()
            .filter(|s| **s == ConnectionState::Connected)
            .count() as u32;
        stats.discovered_peers = self.peers.read().await.len() as u32;
        stats
    }

    /// The most recently received sync request.
    pub async fn last_received_sync(&self) -> Option<SyncRequest> {
        self.last_received.read().await.clone()
    }

    /// The most recently received gossipsub message.
    pub async fn last_gossip_message(&self) -> Option<Vec<u8>> {
        self.last_gossip.read().await.clone()
    }

    #[allow(dead_code)]
    fn record_bytes_received(&self, n: u64) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.bytes_received = stats.bytes_received.saturating_add(n);
        }
    }

    /// Record bytes sent over the network.
    fn record_bytes_sent(&self, n: u64) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.bytes_sent = stats.bytes_sent.saturating_add(n);
        }
    }
}

/// Discovery protocol for finding other Auralis devices on the LAN.
pub struct Discovery {
    local_port: u16,
    runtime: Arc<NetworkRuntime>,
}

impl Discovery {
    /// Create a new discovery service.
    pub fn new(local_port: u16) -> Self {
        Self {
            local_port,
            runtime: Arc::new(NetworkRuntime::new()),
        }
    }

    /// The shared network runtime backing this discovery service.
    pub fn runtime(&self) -> Arc<NetworkRuntime> {
        self.runtime.clone()
    }

    /// A [`SyncEngine`] that shares this node's swarm and peer identity.
    pub fn sync_engine(&self) -> SyncEngine {
        SyncEngine::share(self.runtime.clone())
    }

    /// Begin listening for peer announcements on the local network.
    ///
    /// Spawns a background task that owns the libp2p swarm and processes
    /// mDNS/gossipsub/request-response events until [`Discovery::stop`] is
    /// called. Calling this again while running is a no-op.
    pub async fn start(&self) -> Result<(), NetworkError> {
        if self
            .runtime
            .task
            .lock()
            .map(|t| t.is_some())
            .unwrap_or(false)
        {
            debug!("Discovery already running");
            return Ok(());
        }

        let (mut swarm, tx, command_rx) = self.runtime.build(self.local_port)?;

        let topic = gossipsub::IdentTopic::new(SYNC_TOPIC);
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&topic)
            .map_err(|e| NetworkError::ProtocolError(e.to_string()))?;

        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", self.local_port)
            .parse()
            .map_err(|e| NetworkError::IoError(format!("{e}")))?;
        swarm
            .listen_on(listen_addr)
            .map_err(|e| NetworkError::IoError(e.to_string()))?;

        let runtime = self.runtime.clone();
        let handle = tokio::spawn(run_swarm(runtime.clone(), swarm, command_rx));
        if let Ok(mut task) = runtime.task.lock() {
            *task = Some(handle);
        }
        *runtime.command_tx.lock().await = Some(tx);

        info!(
            port = self.local_port,
            peer_id = %runtime.local_peer_id,
            "mDNS discovery started"
        );
        Ok(())
    }

    /// Stop the discovery service, aborting the background swarm task.
    pub async fn stop(&self) -> Result<(), NetworkError> {
        let handle = {
            let task = self
                .runtime
                .task
                .lock()
                .map(|mut t| t.take())
                .unwrap_or(None);
            task
        };
        if let Some(handle) = handle {
            handle.abort();
            info!("Discovery stopped");
        }
        Ok(())
    }

    /// Dial a discovered peer by its multiaddr (optionally including `/p2p/<peer>`).
    pub async fn connect(&self, peer_address: &str) -> Result<(), NetworkError> {
        self.send_command(NetworkCommand::Dial {
            peer_id: None,
            address: peer_address
                .parse()
                .map_err(|e| NetworkError::ProtocolError(format!("Invalid multiaddr: {e}")))?,
        })
        .await
    }

    /// Publish `data` on the given gossipsub topic.
    pub async fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), NetworkError> {
        self.send_command(NetworkCommand::Publish {
            topic: topic.to_string(),
            data,
        })
        .await
    }

    /// Convenience alias for [`Discovery::publish`] on the default sync topic.
    pub async fn broadcast(&self, data: Vec<u8>) -> Result<(), NetworkError> {
        self.publish(SYNC_TOPIC, data).await
    }

    /// List discovered peers (peer id + addresses) for the UI.
    pub async fn discovered_peers(&self) -> Vec<(String, Vec<String>)> {
        self.runtime.discovered_peers().await
    }

    /// Current connection per-peer state for the UI.
    pub async fn connection_state(&self) -> Vec<(String, ConnectionState)> {
        self.runtime.connections().await
    }

    /// Network statistics.
    pub async fn stats(&self) -> NetworkStats {
        self.runtime.stats().await
    }

    /// Bind a PairedDevice UUID to a discovered PeerId (in-memory alias).
    pub async fn link_device_peer(
        &self,
        device_id: &str,
        peer_id_str: &str,
    ) -> Result<(), NetworkError> {
        let pid = PeerId::from_str(peer_id_str)
            .map_err(|_| NetworkError::PeerNotFound(peer_id_str.to_string()))?;
        self.runtime
            .register_device_alias(device_id.to_string(), pid)
            .await;
        self.runtime
            .register_device_alias(device_id.to_ascii_lowercase(), pid)
            .await;
        Ok(())
    }

    async fn send_command(&self, command: NetworkCommand) -> Result<(), NetworkError> {
        let tx = self
            .runtime
            .command_tx
            .lock()
            .await
            .clone()
            .ok_or_else(|| {
                NetworkError::ConnectionError(
                    "network runtime not started; call start() first".to_string(),
                )
            })?;
        tx.send(command)
            .await
            .map_err(|e| NetworkError::SendError(e.to_string()))
    }
}

/// P2P sync engine.
///
/// Sends queued sync requests to a peer over the request-response protocol.
/// When created through [`Discovery::sync_engine`] it shares the discovery
/// node's swarm so that mDNS-discovered peers are directly reachable.
pub struct SyncEngine {
    peer_id: String,
    runtime: Arc<NetworkRuntime>,
    queue: StdMutex<Vec<serde_json::Value>>,
}

impl Clone for SyncEngine {
    fn clone(&self) -> Self {
        Self {
            peer_id: self.peer_id.clone(),
            runtime: self.runtime.clone(),
            queue: StdMutex::new(Vec::new()),
        }
    }
}

impl SyncEngine {
    /// Create a standalone sync engine with its own keypair/swarm.
    pub fn new() -> Self {
        let runtime = NetworkRuntime::new();
        let peer_id = runtime.local_peer_id.to_string();
        Self {
            peer_id,
            runtime: Arc::new(runtime),
            queue: StdMutex::new(Vec::new()),
        }
    }

    fn share(runtime: Arc<NetworkRuntime>) -> Self {
        let peer_id = runtime.local_peer_id.to_string();
        Self {
            peer_id,
            runtime,
            queue: StdMutex::new(Vec::new()),
        }
    }

    /// Get the local peer id.
    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    /// The shared network runtime.
    pub fn runtime(&self) -> Arc<NetworkRuntime> {
        self.runtime.clone()
    }

    /// Queue a sync payload to be sent on the next [`SyncEngine::request_sync`].
    pub fn enqueue_sync_payload(&self, payload: serde_json::Value) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(payload);
        }
    }

    /// Dial a discovered peer by multiaddr.
    pub async fn connect(&self, peer_address: &str) -> Result<(), NetworkError> {
        let tx = self
            .runtime
            .command_tx
            .lock()
            .await
            .clone()
            .ok_or_else(|| {
                NetworkError::ConnectionError("network runtime not started".to_string())
            })?;
        tx.send(NetworkCommand::Dial {
            peer_id: None,
            address: peer_address
                .parse()
                .map_err(|e| NetworkError::ProtocolError(format!("Invalid multiaddr: {e}")))?,
        })
        .await
        .map_err(|e| NetworkError::SendError(e.to_string()))
    }

    /// Bind a PairedDevice UUID to its real PeerId after discovery/pairing.
    /// Required because `PairedDevice.id` is a UUID, not a PeerId.
    pub async fn link_device_peer(
        &self,
        device_id: &str,
        peer_id_str: &str,
    ) -> Result<(), NetworkError> {
        let pid = PeerId::from_str(peer_id_str)
            .map_err(|_| NetworkError::PeerNotFound(peer_id_str.to_string()))?;
        self.runtime
            .register_device_alias(device_id.to_string(), pid)
            .await;
        // also store lowercased variant for UUID case-insensitivity
        self.runtime
            .register_device_alias(device_id.to_ascii_lowercase(), pid)
            .await;
        Ok(())
    }

    /// Send a sync request to a discovered peer and await its response.
    ///
    /// `peer` may be a raw libp2p PeerId string **or** a PairedDevice UUID that
    /// was previously bound via `link_device_peer` / `Discovery::link_device_peer`.
    /// The alias table is consulted before parsing as PeerId, fixing the
    /// `PeerId::from_str(UUID) -> PeerNotFound` bug. Returns `Err(PeerNotFound)`
    /// if the peer has not been discovered via mDNS and no alias exists.
    pub async fn request_sync(&self, peer: &str) -> Result<(), NetworkError> {
        let peer_id = if let Some(pid) = self.runtime.resolve_peer_id(peer).await {
            pid
        } else {
            return Err(NetworkError::PeerNotFound(peer.to_string()));
        };

        let addresses = if peer == self.peer_id || peer_id.to_string() == self.peer_id {
            vec![]
        } else {
            self.runtime.addresses_of(&peer_id).await
        };
        if addresses.is_empty() {
            return Err(NetworkError::PeerNotFound(peer.to_string()));
        }

        let queue: Vec<serde_json::Value> = {
            let q = self.queue.lock().map(|g| g.clone()).unwrap_or_default();
            q
        };
        let request = SyncRequest {
            payload: serde_json::json!({ "queued": queue.len(), "changes": queue }),
        };

        let tx = self
            .runtime
            .command_tx
            .lock()
            .await
            .clone()
            .ok_or_else(|| {
                NetworkError::ConnectionError("network runtime not started".to_string())
            })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(NetworkCommand::RequestSync {
            peer_id,
            request,
            reply: reply_tx,
        })
        .await
        .map_err(|e| NetworkError::SendError(e.to_string()))?;

        match tokio::time::timeout(Duration::from_secs(30), reply_rx).await {
            Ok(Ok(result)) => {
                if result.is_ok() {
                    if let Ok(mut q) = self.queue.lock() {
                        q.clear();
                    }
                }
                result
            }
            Ok(Err(_)) => Err(NetworkError::ConnectionError(
                "sync channel closed before a response".to_string(),
            )),
            Err(_) => Err(NetworkError::Timeout),
        }
    }
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection state for a peer, exposed to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConnectionState {
    Disconnected,
    Dialing,
    Connected,
}

/// Network-related errors.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Discovery error: {0}")]
    DiscoveryError(String),

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    #[error("Send error: {0}")]
    SendError(String),

    #[error("I/O error: {0}")]
    IoError(String),

    #[error("Timeout")]
    Timeout,
}

/// Network statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub active_connections: u32,
    pub discovered_peers: u32,
}

impl NetworkStats {
    /// Record bytes sent over the network.
    pub fn record_bytes_sent(&mut self, n: u64) {
        self.bytes_sent = self.bytes_sent.saturating_add(n);
    }

    /// Record bytes received from the network.
    pub fn record_bytes_received(&mut self, n: u64) {
        self.bytes_received = self.bytes_received.saturating_add(n);
    }
}

/// Builds the fully configured libp2p swarm for a node.
///
/// Uses the tokio provider: a TCP transport secured with Noise and
/// multiplexed with Yamux, plus mDNS, gossipsub and a JSON request-response
/// protocol. The behaviour is available for inspection at the `network`
/// module level for tests and tooling.
pub fn build_swarm(
    keypair: &Keypair,
    _listen_port: u16,
) -> Result<Swarm<SwarmBehaviour>, NetworkError> {
    let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|e| NetworkError::DiscoveryError(e.to_string()))?
        .with_behaviour(
            |key| -> Result<SwarmBehaviour, Box<dyn std::error::Error + Send + Sync + 'static>> {
                let local_peer_id = key.public().to_peer_id();

                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

                let gossip_cfg = gossipsub::ConfigBuilder::default()
                    .flood_publish(true)
                    .max_transmit_size(1024 * 1024)
                    .build()
                    .map_err(|e| e.to_string())?;
                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossip_cfg,
                )
                .map_err(|e| e.to_string())?;

                let request_response =
                    request_response::json::Behaviour::<SyncRequest, SyncResponse>::new(
                        [(
                            StreamProtocol::new(SYNC_PROTOCOL_NAME),
                            request_response::ProtocolSupport::Full,
                        )],
                        request_response::Config::default(),
                    );

                Ok(SwarmBehaviour {
                    mdns,
                    gossipsub,
                    request_response,
                })
            },
        )
        .map_err(|e| NetworkError::DiscoveryError(e.to_string()))?
        .build();

    Ok(swarm)
}

/// Background event loop that owns the swarm until a `Shutdown` command or
/// the command channel closes.
async fn run_swarm(
    runtime: Arc<NetworkRuntime>,
    mut swarm: Swarm<SwarmBehaviour>,
    mut command_rx: mpsc::Receiver<NetworkCommand>,
) {
    // Shared pending map so timeout tasks can clean up the entry even when the
    // caller dropped its `reply_rx` after a 30s timeout. Previously a timed-out
    // request leaked forever (never removed until a late response arrived).
    let pending: Arc<
        TokioMutex<
            HashMap<request_response::OutboundRequestId, oneshot::Sender<Result<(), NetworkError>>>,
        >,
    > = Arc::new(TokioMutex::new(HashMap::new()));

    info!(peer_id = %runtime.local_peer_id, "Swarm event loop started");
    loop {
        tokio::select! {
            command = command_rx.recv() => {
                match command {
                    Some(NetworkCommand::Dial { peer_id, address }) => {
                        if let Some(pid) = peer_id {
                            runtime.add_address(pid, address.clone()).await;
                            runtime.set_connection(pid, ConnectionState::Dialing).await;
                        }
                        match swarm.dial(address.clone()) {
                            Ok(()) => debug!(%address, "Dialing peer"),
                            Err(e) => warn!(%address, error = ?e, "Dial rejected"),
                        }
                    }
                    Some(NetworkCommand::RequestSync { peer_id, request, reply }) => {
                        let addrs = runtime.addresses_of(&peer_id).await;
                        for addr in addrs {
                            swarm.add_peer_address(peer_id, addr);
                        }
                        let req_bytes = serde_json::to_vec(&request)
                            .unwrap_or_default()
                            .len() as u64;
                        runtime.record_bytes_sent(req_bytes);
                        let request_id =
                            swarm.behaviour_mut().request_response.send_request(&peer_id, request);
                        {
                            let mut map = pending.lock().await;
                            map.insert(request_id, reply);
                        }
                        // Timeout cleanup: remove pending entry after 30s if no
                        // response / OutboundFailure cleaned it already.
                        let pending_clone = pending.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            let mut map = pending_clone.lock().await;
                            if let Some(sender) = map.remove(&request_id) {
                                let _ = sender.send(Err(NetworkError::Timeout));
                            }
                        });
                    }
                    Some(NetworkCommand::Publish { topic, data }) => {
                        let msg_len = data.len() as u64;
                        runtime.record_bytes_sent(msg_len);
                        let topic = gossipsub::IdentTopic::new(topic);
                        let hash = topic.hash();
                        match swarm.behaviour_mut().gossipsub.publish(hash, data) {
                            Ok(msg_id) => debug!(%msg_id, "Published gossipsub message"),
                            Err(e) => warn!(error = ?e, "Gossipsub publish failed"),
                        }
                    }
                    Some(NetworkCommand::Shutdown) | None => break,
                }
            }
            event = swarm.next() => {
                match event {
                    Some(event) => {
                        handle_swarm_event(&runtime, &mut swarm, &pending, event).await;
                    }
                    None => break,
                }
            }
        }
    }

    if let Ok(mut task) = runtime.task.lock() {
        *task = None;
    }
    info!("Swarm event loop exited");
}

/// Dispatches a single `SwarmEvent`.
async fn handle_swarm_event(
    runtime: &Arc<NetworkRuntime>,
    swarm: &mut Swarm<SwarmBehaviour>,
    pending: &Arc<
        TokioMutex<
            HashMap<request_response::OutboundRequestId, oneshot::Sender<Result<(), NetworkError>>>,
        >,
    >,
    event: SwarmEvent<<SwarmBehaviour as libp2p::swarm::NetworkBehaviour>::ToSwarm>,
) {
    match event {
        SwarmEvent::Behaviour(SwarmBehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
            for (peer_id, addr) in peers {
                debug!(%peer_id, %addr, "Discovered peer via mDNS");
                runtime.add_address(peer_id, addr.clone()).await;
                runtime
                    .set_connection(peer_id, ConnectionState::Dialing)
                    .await;
                if let Err(e) = swarm.dial(addr.clone()) {
                    warn!(%addr, error = ?e, "Failed to dial discovered peer");
                }
            }
        }
        SwarmEvent::Behaviour(SwarmBehaviourEvent::Mdns(mdns::Event::Expired(peers))) => {
            for (peer_id, addr) in peers {
                debug!(%peer_id, %addr, "mDNS record expired");
                runtime.remove_address(peer_id, &addr).await;
            }
        }
        SwarmEvent::Behaviour(SwarmBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            message,
            ..
        })) => {
            info!(topic = %message.topic, len = message.data.len(), "Received gossipsub message");
            // Clone data before acquiring async lock — avoids holding guard across await
            let data = message.data.clone();
            *runtime.last_gossip.write().await = Some(data);
        }
        SwarmEvent::Behaviour(SwarmBehaviourEvent::RequestResponse(
            request_response::Event::Message {
                peer,
                message,
                connection_id: _,
            },
        )) => match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
                info!(%peer, "Received sync request");
                // Clone before await to not hold guard across await
                let req_clone = request.clone();
                *runtime.last_received.write().await = Some(req_clone);
                let response = SyncResponse {
                    ok: true,
                    message: format!("ack from {}", runtime.local_peer_id),
                };
                let _ = swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, response);
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                let reply = {
                    let mut map = pending.lock().await;
                    map.remove(&request_id)
                };
                if let Some(reply) = reply {
                    let result = if response.ok {
                        Ok(())
                    } else {
                        Err(NetworkError::ProtocolError(response.message))
                    };
                    debug!(%peer, "Received sync response");
                    let _ = reply.send(result);
                }
            }
        },
        SwarmEvent::Behaviour(SwarmBehaviourEvent::RequestResponse(
            request_response::Event::OutboundFailure {
                request_id, error, ..
            },
        )) => {
            let reply = {
                let mut map = pending.lock().await;
                map.remove(&request_id)
            };
            if let Some(reply) = reply {
                let _ = reply.send(Err(NetworkError::ConnectionError(format!(
                    "outbound sync failed: {error:?}"
                ))));
            }
        }
        SwarmEvent::Behaviour(SwarmBehaviourEvent::RequestResponse(
            request_response::Event::InboundFailure { error, .. },
        )) => {
            warn!(?error, "Inbound sync failure");
        }
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            if let ConnectedPoint::Dialer { address, .. } = &endpoint {
                runtime.add_address(peer_id, address.clone()).await;
            }
            runtime
                .set_connection(peer_id, ConnectionState::Connected)
                .await;
            debug!(%peer_id, "Connection established");
        }
        SwarmEvent::ConnectionClosed {
            peer_id,
            num_established,
            ..
        } => {
            if num_established == 0 {
                runtime
                    .set_connection(peer_id, ConnectionState::Disconnected)
                    .await;
                debug!(%peer_id, "Connection closed");
            }
        }
        SwarmEvent::Dialing {
            peer_id: Some(peer_id),
            ..
        } => {
            runtime
                .set_connection(peer_id, ConnectionState::Dialing)
                .await;
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            info!(%address, "Listening on address");
        }
        SwarmEvent::ListenerClosed { reason: Err(e), .. } => {
            error!(error = ?e, "Listener closed with error");
        }
        SwarmEvent::OutgoingConnectionError {
            peer_id: Some(peer_id),
            error,
            ..
        } => {
            runtime
                .set_connection(peer_id, ConnectionState::Disconnected)
                .await;
            debug!(%peer_id, error = ?error, "Outgoing connection failed");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let keypair = Keypair::generate_ed25519();
        let pid = keypair.public().to_peer_id();
        let text = pid.to_string();
        let parsed = PeerId::from_str(&text).unwrap();
        assert_eq!(pid, parsed);
    }

    #[test]
    fn network_error_implements_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<NetworkError>();
    }
}
