//! Auralis Library Root
//!
//! This module re-exports all public components of the Auralis library,
//! providing a clean API surface for the Tauri application and potential
//! future integrations (CLI, embedded use, etc.).

// ============================================================================
// Domain Layer - Business Logic
// ============================================================================
pub mod domain;

// Re-export domain types for convenient access
pub use domain::models::{
    Album, AlbumArt, AppearanceSettings, Artist, AudioFormat, AudioSettings, DownloadProgress,
    DownloadSettings, DownloadStatus, LibrarySettings, LibraryView, NowPlaying, PairedDevice,
    PairingInfo, Playlist, RepeatMode, ScanSummary, Settings, SmartPlaylistCriteria,
    SmartSortField, SyncChange, SyncSettings, SyncStatus, ThemeMode, Track, TrackFilter,
    TrackMetadataUpdate, TrackSortField,
};
pub use domain::services::{
    DownloadService, LibraryService, PlaybackService, PlaylistService, SettingsService, SyncService,
};

// ============================================================================
// Infrastructure Layer - External Integrations
// ============================================================================
pub mod infrastructure;

// Re-export infrastructure types
pub use infrastructure::database::Database;
pub use infrastructure::media::{AudioPlayer, Downloader};

// ============================================================================
// Application Layer - Tauri Commands
// ============================================================================
pub mod commands;

// ============================================================================
// Presentation Layer - Templates
// ============================================================================
pub mod templates;

// ============================================================================
// Main Application Builder
// ============================================================================
use tauri::Manager;
use tracing::info;

/// Auralis Application Builder
///
/// This struct provides a fluent interface for building and configuring
/// the Tauri application with all necessary plugins, commands, and
/// global state management.
pub struct AuralisApp;

impl AuralisApp {
    /// Build the Tauri application with all configurations
    pub fn build() -> tauri::Result<tauri::App> {
        info!("Building Auralis application");

        tauri::Builder::default()
            .plugin(tauri_plugin_shell::init())
            .setup(|app| {
                info!("Running Tauri setup phase");

                // Initialize database
                let app_handle = app.handle().clone();
                if let Err(e) = Self::init_database(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize database");
                }

                // Initialize audio player
                if let Err(e) = Self::init_audio_player(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize audio player");
                }

                // Load settings
                if let Err(e) = Self::load_settings(&app_handle) {
                    tracing::error!(error = %e, "Failed to load settings");
                }

                info!("Setup phase completed successfully");
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                // Library commands
                commands::library::get_tracks,
                commands::library::get_track,
                commands::library::update_track_metadata,
                commands::library::delete_tracks,
                commands::library::scan_library_paths,
                commands::library::search_tracks,
                // Playback commands
                commands::playback::play,
                commands::playback::pause,
                commands::playback::resume,
                commands::playback::stop,
                commands::playback::next_track,
                commands::playback::previous_track,
                commands::playback::seek,
                commands::playback::set_volume,
                commands::playback::set_repeat_mode,
                commands::playback::set_shuffle,
                commands::playback::get_now_playing,
                commands::playback::get_queue,
                commands::playback::add_to_queue,
                commands::playback::remove_from_queue,
                commands::playback::clear_queue,
                // Download commands
                commands::downloads::download_audio,
                commands::downloads::download_playlist,
                commands::downloads::pause_download,
                commands::downloads::resume_download,
                commands::downloads::cancel_download,
                commands::downloads::get_download_progress,
                // Playlist commands
                commands::playlists::get_playlists,
                commands::playlists::get_playlist,
                commands::playlists::create_playlist,
                commands::playlists::update_playlist,
                commands::playlists::delete_playlist,
                commands::playlists::add_tracks_to_playlist,
                commands::playlists::remove_tracks_from_playlist,
                commands::playlists::reorder_playlist_tracks,
                // Sync commands
                commands::sync::get_paired_devices,
                commands::sync::start_pairing,
                commands::sync::complete_pairing,
                commands::sync::unpair_device,
                commands::sync::sync_with_device,
                commands::sync::get_sync_status,
                // Settings commands
                commands::settings::get_settings,
                commands::settings::update_settings,
                // Template commands
                commands::templates::render_template,
                commands::templates::render_partial,
            ])
            .build(tauri::generate_context!())
    }

    /// Initialize the database connection and run migrations
    fn init_database(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing database");

        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {e}"))?;

        std::fs::create_dir_all(&app_data_dir)?;

        let db_path = app_data_dir.join("auralis.db");
        info!(path = %db_path.display(), "Database path configured");

        // Open the database and run migrations, then store it in app state
        let db = Database::new(&db_path).map_err(|e| format!("Failed to open database: {e:?}"))?;
        db.run_migrations()
            .map_err(|e| format!("Failed to run migrations: {e:?}"))?;
        app_handle.manage(db);

        Ok(())
    }

    /// Initialize the audio player
    fn init_audio_player(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing audio player");

        let player = AudioPlayer::new()?;
        app_handle.manage(player);

        Ok(())
    }

    /// Load application settings
    fn load_settings(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading application settings");

        let settings = Settings::load()?;
        app_handle.manage(settings);

        Ok(())
    }
}

/// Mobile entry point for Android/iOS
#[tauri::mobile_entry_point]
fn main() {
    AuralisApp::build()
        .map_err(|e| eprintln!("Failed to build Auralis: {e}"))
        .expect("Failed to initialize Auralis application");
}
}

