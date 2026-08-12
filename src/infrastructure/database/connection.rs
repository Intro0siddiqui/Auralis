//! Database Connection Management
//!
//! Manages SQLite database connections and initialization.

use rusqlite::{Connection, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

/// Database connection wrapper.
///
/// We use `std::sync::Mutex` (not `tokio::sync::RwLock`) because
/// `rusqlite::Connection` is `Send` but not `Sync` (it contains internal
/// `RefCell`s). A `std::sync::Mutex<T>` is `Sync` whenever `T: Send`, so
/// `Arc<Mutex<Connection>>` is `Send + Sync`, which is what Tauri's
/// `State<...>` requires.
#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

impl Database {
    /// Create a new database connection
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, DatabaseError> {
        let path = path.as_ref();
        info!(path = %path.display(), "Opening database");

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DatabaseError::IoError(e.to_string()))?;
        }

        // Open connection
        let connection =
            Connection::open(path).map_err(|e| DatabaseError::ConnectionError(e.to_string()))?;

        // Enable foreign keys and WAL mode for better performance
        connection
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;
                ",
            )
            .map_err(|e| DatabaseError::ConfigurationError(e.to_string()))?;

        info!("Database connection established");
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Run database migrations
    pub fn run_migrations(&self) -> Result<(), DatabaseError> {
        debug!("Running database migrations");

        let connection = self
            .connection
            .lock()
            .map_err(|e| DatabaseError::ConnectionError(format!("Mutex poisoned: {e}")))?;

        // Create tables
        connection
            .execute_batch(
                r#"
                -- Tracks table
                CREATE TABLE IF NOT EXISTS tracks (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    album_artist TEXT,
                    genre TEXT,
                    year INTEGER,
                    track_number INTEGER,
                    disc_number INTEGER,
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    file_path TEXT NOT NULL UNIQUE,
                    file_size INTEGER NOT NULL DEFAULT 0,
                    format TEXT NOT NULL DEFAULT 'mp3',
                    bitrate INTEGER,
                    sample_rate INTEGER,
                    album_art_path TEXT,
                    date_added TEXT NOT NULL,
                    last_played TEXT,
                    play_count INTEGER NOT NULL DEFAULT 0,
                    is_downloaded INTEGER NOT NULL DEFAULT 0,
                    source_url TEXT
                );

                -- Playlists table
                CREATE TABLE IF NOT EXISTS playlists (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    is_smart INTEGER NOT NULL DEFAULT 0,
                    track_ids TEXT NOT NULL DEFAULT '[]',
                    smart_criteria TEXT
                );

                -- Playlist tracks junction table
                CREATE TABLE IF NOT EXISTS playlist_tracks (
                    playlist_id TEXT NOT NULL,
                    track_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    PRIMARY KEY (playlist_id, track_id),
                    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                    FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
                );

                -- Paired devices table
                CREATE TABLE IF NOT EXISTS paired_devices (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    device_type TEXT NOT NULL,
                    ip_address TEXT,
                    paired_at TEXT NOT NULL,
                    last_sync TEXT,
                    status TEXT NOT NULL DEFAULT 'disconnected',
                    library_version INTEGER NOT NULL DEFAULT 0
                );

                -- Sync changes table
                CREATE TABLE IF NOT EXISTS sync_changes (
                    id TEXT PRIMARY KEY,
                    change_type TEXT NOT NULL,
                    entity_type TEXT NOT NULL,
                    entity_id TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    applied INTEGER NOT NULL DEFAULT 0
                );

                -- Settings table (single row)
                CREATE TABLE IF NOT EXISTS settings (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    data TEXT NOT NULL
                );

                -- Indexes for performance
                CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
                CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
                CREATE INDEX IF NOT EXISTS idx_tracks_date_added ON tracks(date_added);
                CREATE INDEX IF NOT EXISTS idx_tracks_play_count ON tracks(play_count);
                CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position ON playlist_tracks(playlist_id, position);
                CREATE INDEX IF NOT EXISTS idx_sync_changes_applied ON sync_changes(applied);
                "#,
            )
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;

        debug!("Database migrations completed");
        Ok(())
    }

    /// Get a guard for the connection. Note: blocks the current thread.
    pub fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, DatabaseError> {
        self.connection
            .lock()
            .map_err(|e| DatabaseError::ConnectionError(format!("Mutex poisoned: {e}")))
    }
}

/// Database-related errors
#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub enum DatabaseError {
    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Migration error: {0}")]
    MigrationError(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Query error: {0}")]
    QueryError(String),

    #[error("IO error: {0}")]
    IoError(String),
}
