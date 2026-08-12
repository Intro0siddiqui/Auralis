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

    /// Builds the swarm behaviour from a fresh session and a command channel.
    /// The receiver is moved into the background task by the caller.
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

    /// Send a sync request to a discovered peer and await its response.
    ///
    /// The queued payloads are batched into a single request which is sent on
    /// the request-response protorol. Returns `Err(PeerNotFound)` if the peer
    /// has not been discovered via mDNS.
    pub async fn request_sync(&self, peer: &str) -> Result<(), NetworkError> {
        let peer_id =
            PeerId::from_str(peer).map_err(|_| NetworkError::PeerNotFound(peer.to_string()))?;

        let addresses = if peer == self.peer_id {
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
    let mut pending: HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<(), NetworkError>>,
    > = HashMap::new();

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
                        pending.insert(request_id, reply);
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
                        handle_swarm_event(&runtime, &mut swarm, &mut pending, event).await;
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
    pending: &mut HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<(), NetworkError>>,
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
            let mut last = runtime.last_gossip.blocking_write();
            *last = Some(message.data);
        }
        SwarmEvent::Behaviour(SwarmBehaviourEvent::RequestResponse(
            request_response::Event::Message { peer, message },
        )) => match message {
            request_response::Message::Request {
                request, channel, ..
            } => {
                info!(%peer, "Received sync request");
                let mut last = runtime.last_received.blocking_write();
                *last = Some(request.clone());
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
                if let Some(reply) = pending.remove(&request_id) {
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
            if let Some(reply) = pending.remove(&request_id) {
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
        SwarmEvent::Dialing { peer_id, .. } => {
            if let Some(peer_id) = peer_id {
                runtime
                    .set_connection(peer_id, ConnectionState::Dialing)
                    .await;
            }
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            info!(%address, "Listening on address");
        }
        SwarmEvent::ListenerClosed { reason, .. } => {
            if let Err(e) = reason {
                error!(error = ?e, "Listener closed with error");
            }
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
