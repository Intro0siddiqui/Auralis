//! Repository Interfaces
//!
//! Defines the data access abstractions for the domain layer.
//! Implementations are provided in the infrastructure layer.

mod playlist_repository;
mod settings_repository;
mod sync_repository;
mod track_repository;

pub use playlist_repository::PlaylistRepository;
pub use settings_repository::SettingsRepository;
pub use sync_repository::SyncRepository;
pub use track_repository::TrackRepository;
