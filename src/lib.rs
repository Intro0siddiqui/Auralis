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
    /// Run the Tauri application
    pub fn run() {
        info!("Running Auralis application");

        match Self::build() {
            Ok(app) => {
                app.run(|_app, _event| {});
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to build Auralis application");
            }
        }
    }

    /// Build the Tauri application with all configurations
    fn build() -> tauri::Result<tauri::App> {
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

                // Initialize sync service (depends on the database)
                if let Err(e) = Self::init_sync_service(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize sync service");
                }

                // Initialize audio player
                if let Err(e) = Self::init_audio_player(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize audio player");
                }

                // Load settings
                if let Err(e) = Self::load_settings(&app_handle) {
                    tracing::error!(error = %e, "Failed to load settings");
                }

                // Initialize downloader
                if let Err(e) = Self::init_downloader(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize downloader");
                }

                // Initialize P2P networking (libp2p mDNS discovery)
                if let Err(e) = Self::init_network(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize network");
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
                commands::downloads::list_downloads,
                // Playlist commands
                commands::playlists::get_playlists,
                commands::playlists::get_playlist,
                commands::playlists::create_playlist,
                commands::playlists::update_playlist,
                commands::playlists::delete_playlist,
                commands::playlists::add_tracks_to_playlist,
                commands::playlists::remove_tracks_from_playlist,
                commands::playlists::reorder_playlist_tracks,
                commands::playlists::create_smart_playlist,
                commands::playlists::render_playlists,
                commands::playlists::render_playlist_detail,
                // Sync commands
                commands::sync::get_paired_devices,
                commands::sync::start_pairing,
                commands::sync::complete_pairing,
                commands::sync::unpair_device,
                commands::sync::sync_with_device,
                commands::sync::get_sync_status,
                commands::sync::render_sync,
                // Settings commands
                commands::settings::get_settings,
                commands::settings::update_settings,
                commands::settings::render_settings,
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

    /// Initialize the sync service with database-backed repositories
    fn init_sync_service(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing sync service");

        let db = app_handle
            .try_state::<Database>()
            .map(|state| state.inner().clone())
            .ok_or("Database not initialized; sync service requires it")?;

        let service = commands::sync::build_sync_service(&db);

        tauri::async_runtime::block_on(service.init())?;

        app_handle.manage(service);

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

    /// Initialize the media downloader
    fn init_downloader(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing media downloader");

        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {e}"))?;

        let download_dir = app_data_dir.join("downloads");
        std::fs::create_dir_all(&download_dir)?;

        let downloader = Downloader::new(download_dir);
        app_handle.manage(downloader);

        Ok(())
    }

    /// Initialize P2P networking: starts libp2p mDNS discovery so this node
    /// is advertised on the LAN and can discover + dial other Auralis devices.
    fn init_network(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing P2P network");

        let discovery = crate::infrastructure::network::Discovery::new(0);
        tauri::async_runtime::block_on(discovery.start())
            .map_err(|e| format!("Failed to start network discovery: {e}"))?;

        let sync_engine = discovery.sync_engine();
        app_handle.manage(discovery);
        app_handle.manage(sync_engine);

        Ok(())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    AuralisApp::run()
}
