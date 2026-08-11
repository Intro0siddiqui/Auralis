# Auralis v2 — Goals & Roadmap

This document outlines the feature goals for Auralis v2. **Current priority is perfecting the core features before adding new ones.**

---

## Priority 1: Perfect Current Features (Immediate Focus)

These features are already scaffolded but not wired up. The goal is to make them fully functional.

### 1.1 Music Library
- [ ] Wire database repositories (TrackRepository, PlaylistRepository, SettingsRepository, SyncRepository)
- [ ] Implement file system scanner with metadata extraction (lofty)
- [ ] Connect library commands to actual database queries
- [ ] Full-text search via SQLite FTS5

### 1.2 Audio Playback
- [ ] Wire AudioPlayer (rodio) to Tauri state
- [ ] Implement play, pause, stop, seek, set_volume
- [ ] Queue management (add, remove, clear, next, previous)
- [ ] Support all formats: MP3, FLAC, WAV, AAC, OGG, OPUS
- [ ] Shuffle and repeat modes

### 1.3 Media Downloading
- [ ] Wire yt-dlp sidecar to Tauri state
- [ ] Async download execution with progress tracking
- [ ] Persist download history to database
- [ ] Pause, resume, cancel downloads

### 1.4 Playlists
- [ ] Persist playlists to database
- [ ] CRUD operations (create, update, delete)
- [ ] Add/remove/reorder tracks
- [ ] Smart playlists (criteria-based auto-generation)

### 1.5 P2P Device Sync
- [ ] Implement libp2p networking (mDNS + Noise + GossipSub)
- [ ] QR code pairing (PIN-based trust establishment)
- [ ] Auto-discovery on LAN (after initial QR pairing)
- [ ] Music file transfer between paired devices
- [ ] Library metadata sync

### 1.6 Settings & UI
- [ ] Persist settings to database
- [ ] Wire settings commands
- [ ] Themes (light/dark) via CSS variables
- [ ] Album art display (extracted via lofty)

---

## Priority 2: Extended Features (After Core is Stable)

These features extend the app's capabilities once core features are solid.

| Feature | Description |
|---------|-------------|
| **Equalizer** | Audio DSP customization via rodio/symphonia |
| **Lyrics** | Fetch from embedded tags or online API |
| **Sleep Timer** | Auto-stop playback after duration |
| **Statistics** | Play count, listening history, insights |
| **Import/Export** | M3U/PLS playlist format support |
| **File Watcher** | Auto-rescan library on file changes |
| **Queue Management** | Separate persistent queue vs playlists |
| **Podcast Support** | RSS feed + download manager |
| **Cloud Sync** | Optional WebDAV/S3 backup |

---

## Target Platforms

| Platform | Status | Notes |
|----------|--------|-------|
| **Linux** | Scaffolded | Primary dev target |
| **Windows** | Scaffolded | CI builds configured |
| **macOS** | Scaffolded | x86_64 + aarch64 |
| **Android** | Scaffolded | Tauri Android v2 |

---

## Success Criteria

A feature is "done" when:
1. It compiles without warnings (`cargo clippy` clean)
2. It has unit/integration tests
3. It works on all target platforms
4. It handles errors gracefully (no `unwrap()` in production paths)
5. It has been verified manually

---

## Current State Summary

| Area | Status |
|------|--------|
| Architecture | Solid (DDD with clean layer separation) |
| Database schema | Exists, needs wiring |
| Command handlers | All stubs, need implementation |
| Frontend (HTMX) | Exists, functional |
| Build system | Works, needs optimization |
| CI/CD | Configured, needs optimization |
| Tests | Minimal, need expansion |