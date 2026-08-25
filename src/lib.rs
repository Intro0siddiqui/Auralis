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
pub use domain::services::SyncService;

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
use std::sync::Arc;
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
            .plugin(tauri_plugin_dialog::init())
            .setup(|app| {
                info!("Running Tauri setup phase");

                let app_handle = app.handle().clone();

                // Seed the Android ndk-context global (no-op on non-Android and
                // when JNI_OnLoad already did it). Must run before audio init.
                if let Err(e) = Self::init_android_context(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize android context");
                }

                // Initialize database
                if let Err(e) = Self::init_database(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize database");
                }

                // Initialize P2P networking (libp2p mDNS discovery)
                if let Err(e) = Self::init_network(&app_handle) {
                    tracing::error!(error = %e, "Failed to initialize network");
                }

                // Initialize sync service (depends on the database + network runtime)
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
                commands::library::pick_folder_and_scan,
                commands::library::pick_audio_files_and_import,
                commands::library::import_audio_file,
                commands::library::set_track_favorite,
                commands::library::media_data_url,
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
                commands::downloads::http_fetch,
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
                // Sync commands
                commands::sync::get_paired_devices,
                commands::sync::start_pairing,
                commands::sync::complete_pairing,
                commands::sync::unpair_device,
                commands::sync::sync_with_device,
                commands::sync::get_sync_status,
                commands::sync::connect_peer_address,
                // Settings commands
                commands::settings::get_settings,
                commands::settings::update_settings,
                // Template commands
                commands::templates::render_template,
                commands::templates::render_partial,
            ])
            .build(tauri::generate_context!())
    }

    /// Seed the Android `ndk_context` global used by the cpal/oboe audio backend.
    ///
    /// `JNI_OnLoad` already seeds this as early as possible, but the global
    /// Android `Application` may not exist yet at `.so` load time. By the time
    /// `setup` runs the `Application` is guaranteed to be live, so retry here as
    /// a belt-and-suspenders fallback. Idempotent (no-op if already seeded) and
    /// a no-op on non-Android targets.
    fn init_android_context(
        _app_handle: &tauri::AppHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(target_os = "android")]
        {
            use std::sync::atomic::Ordering;

            let vm_ptr = android_jni::INITIAL_VM.load(Ordering::SeqCst);
            if vm_ptr.is_null() {
                tracing::warn!(
                    "Android JavaVM was not captured in JNI_OnLoad; ndk_context may be unset"
                );
            } else {
                // SAFETY: `vm_ptr` was captured from the live JavaVM in JNI_OnLoad.
                let Ok(vm) = (unsafe { jni::JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM) })
                else {
                    tracing::warn!("Failed to reconstruct JavaVM from JNI_OnLoad pointer");
                    return Ok(());
                };
                android_jni::try_seed(&vm);
            }
        }
        Ok(())
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

        let sync_engine = app_handle
            .try_state::<crate::infrastructure::network::SyncEngine>()
            .map(|state| state.inner().clone())
            .ok_or("Network not initialized; sync service requires SyncEngine")?;

        // Wire DB to network runtime for alias persistence (HIGH fix)
        // Keeps RwLock for runtime but hydrates from DB and persists on register.
        let db_arc = std::sync::Arc::new(db.clone());
        tauri::async_runtime::block_on(sync_engine.runtime().set_persistent_store(db_arc));

        let service = commands::sync::build_sync_service(&db, sync_engine);

        tauri::async_runtime::block_on(service.init())?;

        app_handle.manage(service);

        Ok(())
    }

    /// Initialize the audio player and spawn the playback watcher
    fn init_audio_player(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing audio player");

        let player = AudioPlayer::new()?;
        let player_arc = Arc::new(player.clone());
        commands::playback::spawn_playback_watcher(app_handle.clone(), player_arc.clone());
        infrastructure::media::background_service::attach(player_arc.clone(), app_handle.clone());
        app_handle.manage(player_arc);
        app_handle.manage(player);

        Ok(())
    }

    /// Load application settings
    fn load_settings(app_handle: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading application settings");

        let app_dir = app_handle
            .path()
            .app_data_dir()
            .or_else(|_| app_handle.path().app_config_dir())
            .unwrap_or_else(|_| std::path::PathBuf::from("."));

        let _ = std::fs::create_dir_all(&app_dir);
        let config_path = app_dir.join("settings.toml");

        let settings = Settings::load_from_path(&config_path).unwrap_or_default();
        let volume = settings.audio.volume.clamp(0.0, 1.0);
        app_handle.manage(settings);
        // Hydrate player volume from persisted settings (player defaults 0.8, fallback clamp)
        if let Some(player_state) = app_handle.try_state::<AudioPlayer>() {
            let player = player_state.inner().clone();
            let _ = tauri::async_runtime::block_on(player.set_volume(volume));
            info!(volume, "Hydrated player volume from settings");
        } else if let Some(player_arc_state) = app_handle.try_state::<Arc<AudioPlayer>>() {
            let player = player_arc_state.inner().clone();
            let _ = tauri::async_runtime::block_on(player.set_volume(volume));
            info!(volume, "Hydrated player volume from settings (arc)");
        }

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

/// Android-only helpers for seeding the `ndk-context` global consumed by the
/// cpal/oboe audio backend.
///
/// Tauri v2's Android backend does not call `ndk_context::initialize_android_context`,
/// but `oboe` (cpal's Android audio backend) panics with "android context was not
/// initialized" if the global is left empty. We are the sole initializer (no
/// `android_activity` / `ndk-glue` in the dep tree), so our `SEEDED` flag is
/// authoritative and prevents the double-seed abort that `ndk_context` asserts
/// against.
#[cfg(target_os = "android")]
mod android_jni {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

    /// Set once we have successfully seeded the ndk-context global.
    static SEEDED: AtomicBool = AtomicBool::new(false);

    /// The live `JavaVM*` captured in `JNI_OnLoad`, so `setup` can retry the seed
    /// later once the `Application` object is guaranteed to exist.
    pub static INITIAL_VM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

    /// Seed the `ndk_context` global from `android.app.ActivityThread.currentApplication()`.
    ///
    /// Returns `true` when the global is (or already was) seeded. Fails silently
    /// and returns `false` when the `Application` is not available yet, leaving the
    /// global untouched — we never write a null context, which would poison the
    /// audio backend.
    pub fn try_seed(vm: &jni::JavaVM) -> bool {
        if SEEDED.load(Ordering::SeqCst) {
            return true;
        }

        // No-op (non-detaching) when the current thread is already attached, as
        // in `setup`.
        let Ok(mut env) = vm.attach_current_thread() else {
            return false;
        };
        let Ok(activity_thread) = env.find_class("android/app/ActivityThread") else {
            return false;
        };
        let Ok(current_application) = env.call_static_method(
            activity_thread,
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        ) else {
            return false;
        };
        let Ok(app_object) = current_application.l() else {
            return false;
        };
        if app_object.is_null() {
            // Application not ready yet — do not poison the global with a null
            // context. `setup` retries once it is guaranteed present.
            return false;
        }

        let Ok(context_global) = env.new_global_ref(app_object) else {
            return false;
        };
        let context_ptr = context_global.as_raw() as *mut c_void;
        let vm_ptr = vm.get_java_vm_pointer() as *mut c_void;
        // Leak the global ref so the Application jobject stays valid for the whole
        // process lifetime (ndk-context only stores the raw pointer).
        std::mem::forget(context_global);
        unsafe {
            ndk_context::initialize_android_context(vm_ptr, context_ptr);
        }
        SEEDED.store(true, Ordering::SeqCst);
        tracing::debug!("ndk_context seeded from ActivityThread.currentApplication()");
        true
    }
}

/// The runtime invokes `JNI_OnLoad` once when the library is loaded, before any
/// command runs. We capture the live `JavaVM` here and attempt an early seed of
/// the `ndk-context` global; if the `Application` isn't present yet, `setup`
/// retries via [`AuralisApp::init_android_context`].
#[cfg(target_os = "android")]
#[allow(non_snake_case)]
#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: jni::JavaVM, _reserved: *mut std::ffi::c_void) -> i32 {
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;

    android_jni::INITIAL_VM.store(vm.get_java_vm_pointer() as *mut c_void, Ordering::SeqCst);
    android_jni::try_seed(&vm);

    jni::sys::JNI_VERSION_1_6
}
