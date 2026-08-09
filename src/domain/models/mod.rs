//! Domain Models Module
//!
//! This module contains all core domain entities and value objects
//! used throughout the Auralis application. These types are designed
//! to be pure business logic with no infrastructure dependencies.

mod album;
mod artist;
mod download;
mod playlist;
mod settings;
mod sync;
mod track;

pub use album::{Album, AlbumArt};
pub use artist::Artist;
pub use download::{DownloadProgress, DownloadStatus};
pub use playlist::{Playlist, SmartPlaylistCriteria, SmartSortField};
pub use settings::{
    AppearanceSettings, AudioSettings, DownloadSettings, LibrarySettings, Settings,
    SyncSettings, ThemeMode, LibraryView,
};
pub use sync::{ChangeType, DeviceStatus, DeviceType, EntityType, PairingInfo, PairedDevice, SyncChange, SyncStatus};
pub use track::{AudioFormat, RepeatMode, Track, TrackFilter, TrackMetadataUpdate, TrackSortField};

/// Playback state information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NowPlaying {
    pub track: Track,
    pub position_secs: u32,
    pub is_playing: bool,
    pub volume: f32,
    pub repeat_mode: RepeatMode,
    pub shuffle_enabled: bool,
}

/// Library scan summary
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanSummary {
    pub tracks_added: u32,
    pub tracks_updated: u32,
    pub tracks_removed: u32,
    pub errors: Vec<String>,
}
