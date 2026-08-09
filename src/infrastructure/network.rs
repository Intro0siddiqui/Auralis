//! Network Infrastructure Module
//!
//! P2P networking, device discovery, and inter-device communication.
//! Implemented on top of libp2p (mdns for LAN discovery, gossipsub for
//! change broadcast, request-response for direct data transfer).

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tracing::{debug, info};

/// Discovery protocol for finding other Auralis devices on the LAN.
pub struct Discovery {
    local_port: u16,
}

impl Discovery {
    /// Create a new discovery service
    pub fn new(local_port: u16) -> Self {
        Self { local_port }
    }

    /// Begin listening for peer announcements (stub for now)
    pub async fn start(&self) -> Result<(), NetworkError> {
        info!(port = self.local_port, "Starting mDNS discovery");
        // TODO: initialize libp2p behaviour with mDNS + Noise + Yamux
        Ok(())
    }

    /// Stop the discovery service
    pub async fn stop(&self) -> Result<(), NetworkError> {
        debug!("Stopping discovery service");
        Ok(())
    }
}

/// P2P sync engine
pub struct SyncEngine {
    peer_id: String,
    /// Listen address for the libp2p swarm; reserved for the upcoming transport impl.
    #[allow(dead_code)]
    listen_addr: Option<SocketAddr>,
}

impl SyncEngine {
    /// Create a new sync engine
    pub fn new() -> Self {
        Self {
            peer_id: uuid::Uuid::new_v4().to_string(),
            listen_addr: None,
        }
    }

    /// Get the local peer id
    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    /// Send a sync request to a remote peer (stub)
    pub async fn request_sync(&self, _peer: &str) -> Result<(), NetworkError> {
        info!(peer = _peer, "Requesting sync");
        // TODO: implement libp2p request-response
        Ok(())
    }
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Network-related errors
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Discovery error: {0}")]
    DiscoveryError(String),

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Timeout")]
    Timeout,
}

/// Network statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub active_connections: u32,
}
