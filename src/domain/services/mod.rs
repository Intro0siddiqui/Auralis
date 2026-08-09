//! Domain Services Module
//!
//! Contains the business logic services that orchestrate domain operations.
//! Services are designed to be framework-agnostic and testable.

mod download_service;
mod library_service;
mod playback_service;
mod playlist_service;
mod settings_service;
mod sync_service;

pub use download_service::DownloadService;
pub use library_service::LibraryService;
pub use playback_service::PlaybackService;
pub use playlist_service::PlaylistService;
pub use settings_service::SettingsService;
pub use sync_service::SyncService;
