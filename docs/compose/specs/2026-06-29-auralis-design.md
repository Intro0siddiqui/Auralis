# Auralis - Offline Music App Design Spec

## [S1] Problem Statement
Build a cross-platform offline music app that downloads and manages audio from YouTube and Instagram Reels, with full music player capabilities. The app should be lightweight, have low latency, and high audio quality.

## [S2] Solution Overview
**Auralis** is built with Kotlin Multiplatform and Compose Multiplatform (Material Design 3). It targets Mac, Linux, and Android with a single codebase and platform-specific builds.

### Core Value Proposition
- Download music from YouTube and Instagram Reels
- Full offline music player with playlist management
- Import local music from device folders
- Cross-platform with native performance

## [S3] Tech Stack

| Component | Technology | Purpose |
|-----------|------------|---------|
| Language | Kotlin Multiplatform | Shared business logic |
| UI | Compose Multiplatform | Material Design 3 interface |
| Database | SQLDelight | Type-safe SQLite queries |
| Downloader | yt-dlp | YouTube/Instagram media extraction |
| Metadata | FFprobe + ID3 | Audio metadata reading/writing |
| Player | Platform-native APIs | Low-latency audio playback |
| Build | Gradle (KMP) | Multi-platform compilation |

## [S4] Features

### 4.1 Media Downloader
- **YouTube:** Download audio, video, and thumbnails
- **Instagram Reels:** Download audio and video
- **URL Support:** Paste URL to download
- **Quality Selection:** Choose audio quality (128k, 192k, 256k, 320k)
- **Format Options:** MP3 (default), FLAC, AAC, OGG, WAV, M4A

### 4.2 Music Player
- Play/pause, next/previous, seek
- Queue management
- Repeat (off, one, all) and shuffle
- Background playback
- Notification controls (Android)
- Sleep timer

### 4.3 Library Manager
- **Playlists:** Create, edit, delete, reorder
- **Folders:** Organize by folders
- **Search:** Search by title, artist, album, filename
- **Sort:** By name, date added, duration, size
- **Filter:** By format, playlist, folder

### 4.4 Metadata Editor
- Edit: Title, Artist, Album, Genre, Year, Track Number
- Edit: Cover art (from thumbnail or file)
- Batch editing support
- Auto-fetch metadata from YouTube/Instagram

### 4.5 Local Import
- Scan device folders for music files
- Add individual files or entire folders
- Auto-detect metadata from existing files
- Exclude already-imported files

## [S5] Architecture

```
┌─────────────────────────────────────────────────────┐
│              Compose Multiplatform UI               │
│            (Material Design 3 Theme)                │
├─────────────────────────────────────────────────────┤
│                ViewModel Layer                      │
│  ┌────────────┬────────────┬────────────┐          │
│  │  PlayerVM  │ LibraryVM  │ DownloadVM │          │
│  └────────────┴────────────┴────────────┘          │
├─────────────────────────────────────────────────────┤
│              Repository / Use Case Layer            │
│  ┌────────────┬────────────┬────────────┐          │
│  │  Player    │  Library   │ Download   │          │
│  │  Service   │  Service   │ Service    │          │
│  └────────────┴────────────┴────────────┘          │
├─────────────────────────────────────────────────────┤
│                 Data Layer                          │
│  ┌────────────┬────────────┬────────────┐          │
│  │  SQLDelight│  File      │ Metadata   │          │
│  │  Database  │  Manager   │ Parser     │          │
│  └────────────┴────────────┴────────────┘          │
├─────────────────────────────────────────────────────┤
│              Platform-Specific Code                 │
│  ┌────────────┬────────────┬────────────┐          │
│  │  Android   │    Mac     │   Linux    │          │
│  │  (KMP)     │  (Native)  │  (Native)  │          │
│  └────────────┴────────────┴────────────┘          │
└─────────────────────────────────────────────────────┘
```

## [S6] Data Model

### Track
```kotlin
data class Track(
    id: Long,
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Int?,
    trackNumber: Int?,
    duration: Long,        // milliseconds
    filePath: String,
    thumbnailPath: String?,
    format: String,        // mp3, flac, etc.
    size: Long,            // bytes
    dateAdded: Long,       // timestamp
    source: String?,       // youtube, instagram, local
    sourceUrl: String?
)
```

### Playlist
```kotlin
data class Playlist(
    id: Long,
    name: String,
    description: String?,
    thumbnailPath: String?,
    dateCreated: Long,
    dateModified: Long
)
```

### PlaylistTrack (junction)
```kotlin
data class PlaylistTrack(
    playlistId: Long,
    trackId: Long,
    position: Int,
    dateAdded: Long
)
```

## [S7] Storage Structure

```
~/.auralis/
├── music/              # Audio files
│   ├── youtube/
│   │   └── {video_id}.mp3
│   ├── instagram/
│   │   └── {reel_id}.mp3
│   └── local/
│       └── {imported_files}
├── thumbnails/         # Cover art and thumbnails
│   ├── {track_id}.jpg
│   └── {track_id}.png
├── playlists/          # Playlist metadata (backup)
└── database/
    └── auralis.db      # SQLite database
```

## [S8] Platform Distribution

| Platform | Format | Notes |
|----------|--------|-------|
| Android | APK / Google Play | Standard Android distribution |
| Mac | .dmg / .app | Native macOS bundle |
| Linux | Standalone binary | No dependencies, self-contained |

## [S9] Audio Format Support

| Format | Default | Notes |
|--------|---------|-------|
| MP3 | Yes (320kbps) | Universal compatibility |
| FLAC | Optional | Lossless quality |
| AAC | Optional | Efficient compression |
| OGG | Optional | Open format |
| WAV | Optional | Uncompressed |
| M4A | Optional | Apple ecosystem |

## [S10] Error Handling

- **Download failures:** Retry with exponential backoff, show error to user
- **Network errors:** Queue downloads for later, show offline indicator
- **File system errors:** Graceful handling, suggest alternative locations
- **Corrupted files:** Detect and mark as invalid, offer re-download
- **Permission errors:** Guide user to grant permissions

## [S11] Testing Strategy

- **Unit tests:** Business logic, data models, repositories
- **Integration tests:** Database operations, file operations
- **UI tests:** Compose UI interactions
- **Platform tests:** Audio playback, file system access
- **Manual testing:** Download flows, player controls, cross-platform verification
