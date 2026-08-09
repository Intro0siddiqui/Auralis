# Auralis v2 — Specification Document

## 1. Project Overview

### Project Name
**Auralis** — A lightweight, offline-first music player with integrated media downloading and P2P synchronization.

### Project Type
Cross-platform desktop and mobile application built with Rust and Tauri.

### Core Feature Summary
A privacy-focused music player that enables users to download audio from YouTube and Instagram, manage local music libraries, and synchronize playback across devices via LAN-based P2P connectivity—all while maintaining a minimal binary footprint under 50MB.

### Target Users
- Music enthusiasts who want offline access to their downloaded audio content
- Privacy-conscious users seeking a lightweight, open-source music solution
- Users who frequently download and organize audio from video platforms
- Small teams or households wanting simple music library sharing without cloud services

---

## 2. Technology Stack & Choices

### Core Language
**Rust** (100% of application logic)

Rationale: Rust provides memory safety without garbage collection, enabling predictable performance and minimal runtime overhead. The language's strong type system and ownership model prevent data races and memory leaks, which are critical for a media application handling concurrent I/O operations.

### UI Framework
**HTMX + Vanilla HTML/CSS**

Rationale: HTMX enables declarative, server-side-driven UI updates by swapping HTML fragments via HTTP requests. This approach eliminates the complexity of client-side JavaScript frameworks while maintaining interactivity. The UI remains lightweight, fast, and accessible, with graceful degradation when JavaScript is unavailable or disabled.

### Desktop Runtime
**Tauri v2** (WebView2 on Windows, WebKitGTK on Linux/macOS)

Rationale: Tauri leverages the operating system's native WebView, avoiding the overhead of bundled JavaScript runtimes. The framework produces binaries under 50MB—significantly smaller than JVM-based solutions that typically exceed 100MB. Tauri v2 introduces improved mobile support, unifying the desktop and Android codebases.

### Mobile Runtime
**Tauri Android v2**

Rationale: A single Tauri-based codebase targets both desktop and Android platforms, eliminating the need for separate Kotlin and Swift implementations. This reduces maintenance burden and ensures feature parity across platforms.

### Database
**SQLite via `rusqlite`**

Rationale: SQLite provides proven reliability for local storage with zero configuration overhead. The `rusqlite` crate offers idiomatic Rust bindings with compile-time query verification, ensuring type safety and preventing SQL injection vulnerabilities.

### Media Engine
**yt-dlp (sidecar executable) orchestrated by Rust backend**

Rationale: yt-dlp remains the most reliable and actively maintained tool for extracting audio from YouTube and Instagram. By running yt-dlp as a sidecar process managed by Rust's async runtime, we maintain proven downloading logic while gaining Rust-level control over process lifecycle, error handling, and resource management. Alternative integration via `ytdl-rs` can be explored for environments where external dependencies are undesirable.

### Audio Playback
**rodio (Rust audio library)**

Rationale: `rodio` provides a simple, efficient interface for audio playback on all major platforms. It supports various audio formats out of the box and integrates naturally with Rust's ownership model.

### P2P Networking
**libp2p (Rust implementation)**

Rationale: libp2p is a modular, extensible networking stack that handles peer discovery, connection management, and data transfer. For LAN-based sync, we utilize mDNS for zero-configuration device discovery and TCP/QUIC for reliable data transmission.

### Architecture Pattern
**Clean Architecture with Command-Query Responsibility Segregation (CQRS)**

Rationale: Clean Architecture separates concerns into distinct layers—Presentation (HTMX templates), Application (Tauri commands), Domain (business logic), and Infrastructure (database, file system, network). CQRS further separates read operations (queries) from write operations (commands), optimizing each for their specific use cases.

### Key Dependencies (Cargo.toml)

```toml
[dependencies]
tauri = { version = "2", features = ["devtools"] }
tauri-plugin-shell = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.32", features = ["bundled"] }
tokio = { version = "1", features = ["full"] }
rodio = "0.19"
lofty = "0.21"          # Audio metadata parsing (ID3, Vorbis comments)
uuid = { version = "1", features = ["v4"] }
libp2p = { version = "0.54", features = ["mdns", "tcp", "quic", "serde"] }
hmac = "0.12"           # For sync authentication
sha2 = "0.10"           # For sync authentication
base64 = "0.22"         # For QR code encoding
image = "0.25"          # QR code generation
qrcode = "0.14"         # QR code rendering
tracing = "0.1"         # Structured logging
tracing-subscriber = "0.3"
thiserror = "2"
dirs = "5"              # Platform-specific directories
glob = "0.3"            # File pattern matching
```

---

## 3. Feature List

### Media Download
- Download audio from YouTube videos/playlists in MP3, FLAC, AAC, OGG, WAV, or M4A formats
- Download audio from Instagram posts and reels
- Progress tracking with pause/resume capability
- Automatic metadata extraction and album art embedding
- Queue management for multiple downloads

### Music Player
- Full audio playback with play, pause, seek, skip, previous controls
- Volume control with mute toggle
- Repeat modes: off, one, all
- Shuffle playback
- Queue management: add, remove, reorder tracks
- Now Playing display with album art and track information
- Background playback on desktop (minimize to system tray)
- Keyboard shortcuts for common actions

### Local Library
- Automatic scanning of configured directories for audio files
- Manual folder import with recursive scanning
- Library organization by: All Songs, Albums, Artists, Folders
- Search functionality across track title, artist, album
- Sorting options: name, date added, duration, file size

### Playlists
- Create, rename, delete playlists
- Add/remove tracks from playlists
- Reorder tracks within playlists
- Playlist export/import (JSON format)
- "Recently Added" and "Most Played" smart playlists

### Metadata Editing
- Edit track title, artist, album, year, genre
- Album art import and removal
- Batch editing for multiple tracks

### P2P Synchronization
- LAN-based device discovery via mDNS
- Pairing via QR code scan or 6-digit PIN entry
- Library sync: share new tracks between paired devices
- Playback state sync: continue listening on another device
- Conflict resolution: last-write-wins with optional manual review
- Encrypted data transfer using HMAC-SHA256 authentication

### Settings
- Audio output device selection
- Download location configuration
- Library scan paths management
- Theme selection (light, dark, system)
- Language selection (i18n support)
- Audio format preferences for downloads
- P2P sync preferences (auto-accept pairing, sync on WiFi only)

### System Integration
- System tray with playback controls (desktop)
- Media key support (play/pause, next, previous)
- Native file dialogs for folder selection
- Notifications for download completion

---

## 4. UI/UX Design Direction

### Overall Visual Style
**Minimalist with subtle depth** — Clean interfaces with ample whitespace, soft shadows for card-based layouts, and a focus on content (album art, track lists) over chrome. The design draws inspiration from modern music applications like Spotify and Apple Music but with a lighter, more utilitarian aesthetic.

### Color Scheme
- **Primary**: Deep indigo (#4F46E5) for interactive elements and active states
- **Background Dark**: Rich charcoal (#1A1A2E) with subtle blue undertones for dark mode
- **Background Light**: Warm off-white (#FAFAFA) for light mode
- **Surface Dark**: Elevated surface (#252542) with slight luminance increase
- **Surface Light**: Clean white (#FFFFFF) for cards and panels
- **Text Primary**: High contrast white (#FFFFFF) in dark mode, near-black (#1F2937) in light mode
- **Text Secondary**: Muted gray (#9CA3AF) for metadata and secondary information
- **Accent**: Warm coral (#F97316) for progress indicators and download status
- **Success**: Emerald green (#10B981) for completed downloads and sync success
- **Error**: Rose red (#EF4444) for errors and destructive actions

### Layout Approach
**Sidebar + Main Content + Now Playing Bar** — A persistent left sidebar for navigation (Library, Playlists, Downloads, Settings), a central content area that adapts to the selected view, and a fixed bottom bar displaying the currently playing track with playback controls. The layout is responsive, collapsing to a bottom tab navigation on smaller screens.

### Typography
- **Headings**: Inter (600-700 weight) — Modern, highly legible sans-serif
- **Body**: Inter (400-500 weight) — Consistent with headings for unified feel
- **Monospace**: JetBrains Mono — For technical information (file paths, bitrate)

### Iconography
**Phosphor Icons (Regular weight)** — A cohesive, open-source icon set with consistent visual weight. Used throughout for navigation, actions, and status indicators.

### Interactions
- **Hover states**: Subtle background color shift on interactive elements
- **Active states**: Scale reduction (0.98) with color change
- **Transitions**: 150ms ease-out for most UI transitions
- **Loading states**: Skeleton placeholders with shimmer animation
- **Empty states**: Illustrated messages with actionable suggestions

### Views

#### Library View
- Grid/List toggle for track display
- Album art thumbnails (grid) or compact rows (list)
- Quick actions on hover: play, add to queue, add to playlist
- Sort and filter toolbar

#### Now Playing View
- Large album art display
- Track metadata with edit capability
- Progress bar with seek functionality
- Volume slider
- Playback controls centered below progress

#### Downloads View
- Active downloads with progress bars
- Completed downloads with status indicators
- Batch actions: pause all, resume all, clear completed

#### P2P Sync View
- Paired devices list with connection status
- Sync progress indicators
- QR code display for receiving pair requests
- Manual PIN entry form

#### Settings View
- Grouped settings sections with clear headers
- Toggle switches for boolean options
- Dropdown selects for enumerated choices
- Directory pickers for path settings

---

## 5. Project Structure

```
auralis/
├── src/                          # Rust source code
│   ├── main.rs                   # Application entry point
│   ├── lib.rs                    # Library root (re-exports)
│   ├── commands/                 # Tauri command handlers
│   │   ├── mod.rs
│   │   ├── library.rs            # Library management commands
│   │   ├── playback.rs           # Playback control commands
│   │   ├── downloads.rs          # Download management commands
│   │   ├── playlists.rs          # Playlist CRUD commands
│   │   ├── sync.rs               # P2P sync commands
│   │   └── settings.rs           # Settings commands
│   ├── domain/                   # Business logic (framework-agnostic)
│   │   ├── mod.rs
│   │   ├── models/               # Domain entities
│   │   │   ├── mod.rs
│   │   │   ├── track.rs          # Track entity
│   │   │   ├── playlist.rs       # Playlist entity
│   │   │   ├── artist.rs         # Artist entity
│   │   │   └── album.rs          # Album entity
│   │   ├── services/             # Business logic services
│   │   │   ├── mod.rs
│   │   │   ├── library_service.rs # Library management logic
│   │   │   ├── playback_service.rs# Playback state machine
│   │   │   ├── download_service.rs# Download orchestration
│   │   │   └── sync_service.rs   # P2P sync logic
│   │   └── repositories/         # Data access interfaces
│   │       ├── mod.rs
│   │       ├── track_repository.rs
│   │       ├── playlist_repository.rs
│   │       └── settings_repository.rs
│   ├── infrastructure/           # External integrations
│   │   ├── mod.rs
│   │   ├── database/             # SQLite implementation
│   │   │   ├── mod.rs
│   │   │   ├── connection.rs     # Connection pool management
│   │   │   ├── migrations.rs     # Schema migrations
│   │   │   └── queries/         # SQL queries
│   │   │       ├── mod.rs
│   │   │       ├── tracks.rs
│   │   │       ├── playlists.rs
│   │   │       └── settings.rs
│   │   ├── filesystem/           # File system operations
│   │   │   ├── mod.rs
│   │   │   ├── scanner.rs        # Library directory scanner
│   │   │   └── metadata.rs       # Audio metadata extraction
│   │   ├── media/                # Media processing
│   │   │   ├── mod.rs
│   │   │   ├── downloader.rs     # yt-dlp wrapper
│   │   │   └── player.rs         # rodio wrapper
│   │   └── network/              # Networking
│   │       ├── mod.rs
│   │       ├── p2p.rs            # libp2p implementation
│   │       └── discovery.rs      # mDNS device discovery
│   └── templates/                # HTMX HTML templates
│       ├── mod.rs
│       ├── layout.html           # Base layout
│       ├── library.html          # Library view
│       ├── player.html           # Now playing
│       ├── downloads.html        # Downloads view
│       ├── playlists.html        # Playlists view
│       ├── sync.html             # P2P sync view
│       └── settings.html         # Settings view
├── src-tauri/                    # Tauri configuration
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── icons/                    # Application icons
│   └── capabilities/             # Tauri permissions
├── templates/                    # HTMX templates (separate for web development)
│   ├── layout.html
│   ├── library.html
│   ├── player.html
│   ├── downloads.html
│   ├── playlists.html
│   ├── sync.html
│   └── settings.html
├── static/                       # Static assets
│   ├── css/
│   │   ├── main.css              # Core styles
│   │   ├── variables.css         # CSS custom properties
│   │   ├── components.css        # Component styles
│   │   └── views.css             # View-specific styles
│   └── js/
│       └── app.js                # Minimal vanilla JS for HTMX
├── migrations/                   # Database migrations
│   └── 001_initial_schema.sql
├── tests/                        # Integration tests
│   ├── library_tests.rs
│   ├── playback_tests.rs
│   └── sync_tests.rs
├── build.rs                      # Build script
├── Cargo.toml                    # Workspace manifest
└── SPEC.md                       # This document
```

---

## 6. Data Models

### Track
```rust
struct Track {
    id: Uuid,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    genre: Option<String>,
    year: Option<i32>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    duration_secs: u32,
    file_path: String,
    file_size: u64,
    format: AudioFormat,
    bitrate: Option<u32>,
    sample_rate: Option<u32>,
    album_art_path: Option<String>,
    date_added: DateTime<Utc>,
    last_played: Option<DateTime<Utc>>,
    play_count: u32,
    is_downloaded: bool,
    source_url: Option<String>,
}

enum AudioFormat {
    Mp3,
    Flac,
    Aac,
    Ogg,
    Wav,
    M4a,
}
```

### Playlist
```rust
struct Playlist {
    id: Uuid,
    name: String,
    description: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    track_ids: Vec<Uuid>,  // Ordered list
    is_smart: bool,
    smart_criteria: Option<SmartPlaylistCriteria>,
}
```

### SyncState
```rust
struct SyncState {
    device_id: Uuid,
    device_name: String,
    last_sync: DateTime<Utc>,
    sync_version: u64,
    pending_changes: Vec<SyncChange>,
}

struct SyncChange {
    id: Uuid,
    change_type: ChangeType,
    entity_type: EntityType,
    entity_id: Uuid,
    payload: serde_json::Value,
    timestamp: DateTime<Utc>,
}
```

---

## 7. API Design (Tauri Commands)

### Library Commands
- `get_tracks(filter: TrackFilter) -> Vec<Track>`
- `get_track(id: Uuid) -> Option<Track>`
- `update_track_metadata(id: Uuid, metadata: TrackMetadataUpdate) -> Result<Track>`
- `delete_tracks(ids: Vec<Uuid>) -> Result<()>`
- `scan_library_paths() -> Result<ScanSummary>`
- `search_tracks(query: String) -> Vec<Track>`

### Playback Commands
- `play(track_id: Uuid) -> Result<NowPlaying>`
- `pause() -> Result<()>`
- `resume() -> Result<()>`
- `stop() -> Result<()>`
- `next() -> Result<Option<NowPlaying>>`
- `previous() -> Result<Option<NowPlaying>>`
- `seek(position_secs: u32) -> Result<()>`
- `set_volume(level: f32) -> Result<()>`  // 0.0 to 1.0
- `set_repeat_mode(mode: RepeatMode) -> Result<()>`
- `set_shuffle(enabled: bool) -> Result<()>`
- `get_now_playing() -> Option<NowPlaying>`
- `get_queue() -> Vec<Track>`

### Download Commands
- `download_audio(url: String, format: AudioFormat) -> Result<Uuid>`
- `download_playlist(url: String, format: AudioFormat) -> Result<Vec<Uuid>>`
- `pause_download(id: Uuid) -> Result<()>`
- `resume_download(id: Uuid) -> Result<()>`
- `cancel_download(id: Uuid) -> Result<()>`
- `get_download_progress() -> Vec<DownloadProgress>`

### Playlist Commands
- `get_playlists() -> Vec<Playlist>`
- `create_playlist(name: String) -> Result<Playlist>`
- `update_playlist(id: Uuid, update: PlaylistUpdate) -> Result<Playlist>`
- `delete_playlist(id: Uuid) -> Result<()>`
- `add_tracks_to_playlist(playlist_id: Uuid, track_ids: Vec<Uuid>) -> Result<()>`
- `remove_tracks_from_playlist(playlist_id: Uuid, track_ids: Vec<Uuid>) -> Result<()>`
- `reorder_playlist_tracks(playlist_id: Uuid, track_ids: Vec<Uuid>) -> Result<()>`

### Sync Commands
- `get_paired_devices() -> Vec<PairedDevice>`
- `start_pairing() -> Result<PairingInfo>`  // Returns QR code data and PIN
- `complete_pairing(pin: String) -> Result<PairedDevice>`
- `unpair_device(device_id: Uuid) -> Result<()>`
- `sync_with_device(device_id: Uuid) -> Result<SyncResult>`
- `get_sync_status() -> SyncStatus`

### Settings Commands
- `get_settings() -> Settings`
- `update_settings(settings: SettingsUpdate) -> Result<Settings>`

---

## 8. Non-Functional Requirements

### Performance
- Application startup time: < 2 seconds on modern hardware
- Library scan rate: > 100 tracks per second for metadata extraction
- UI response time: < 100ms for user interactions
- Memory usage: < 200MB during normal operation

### Binary Size
- Target: < 50MB for desktop executables
- Achieved through Tauri WebView approach and aggressive optimization

### Accessibility
- Full keyboard navigation support
- Screen reader compatibility for all interactive elements
- High contrast mode support
- Respects system font scaling

### Security
- No telemetry or external data collection
- Local-only data storage by default
- Encrypted P2P communication with mutual authentication
- Safe handling of user-provided URLs for downloads

### Reliability
- Graceful handling of corrupted audio files
- Automatic recovery from interrupted downloads
- Data integrity verification for library files
- Crash resilience with automatic state recovery
