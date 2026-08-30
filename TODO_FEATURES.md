# 📋 Auralis v2 Feature Roadmap & Capability TODO

This document tracks all missing, stubbed, or partially wired features across the Auralis codebase (Desktop and Android targets), prioritized into actionable implementation milestones.

## 🚨 Milestone 0: Download & Stream Integrity Engine Overhaul (Independent High-Priority Focus)
> **Goal:** Eliminate partial/truncated audio downloads, enforce strict stream integrity validation, auto-resume broken HTTP streams, and prevent truncated audio playback.

- [x] **1. Strict Stream Completion & Content-Length Verification**
  - **Location:** `src/infrastructure/media/downloader.rs`
  - **Status:** ✅ Implemented with RFC 7233 header parsing and Content-Length verification.
- [x] **2. Automatic Stream Reconnection & HTTP Range Retry Loop**
  - **Location:** `src/infrastructure/media/downloader.rs`
  - **Status:** ✅ Implemented with 5-attempt exponential backoff, Range resumption, and 206/200/416 handling.
- [x] **3. Post-Download Audio Container & Duration Integrity Validation**
  - **Location:** `src/infrastructure/media/downloader.rs`, `src/infrastructure/filesystem/metadata.rs`
  - **Status:** ✅ Implemented via `lofty` probe, property inspection, and ±5s tolerance check.
- [x] **4. Atomic File Publication & Corrupt Partial Cleanup**
  - **Location:** `src/infrastructure/media/downloader.rs`
  - **Status:** ✅ Implemented via `.tmp/<id>.part` staging, atomic rename, and automatic cleanup on error/cancel.
- [x] **5. Duration Mismatch Diagnostic & Auto-Repair in Player**
  - **Location:** `src/infrastructure/media/player.rs`, `src/commands/playback.rs`
  - **Status:** ✅ Implemented mismatch detection (>5s vs decoder properties) and auto-reconciliation to database and in-memory queue.

---

## 🎯 Milestone 1: High-Impact Core Completeness & Missing UI Wiring
> **Goal:** Connect existing backend Tauri commands to the frontend UI and resolve major visual/metadata gaps.

- [x] **1. Embedded Album Artwork Extraction & Caching**
  - **Location:** `src/infrastructure/filesystem/metadata.rs`
  - **Status:** ✅ Implemented with `lofty` `CoverFront`/`Other` extraction, 128-bit hash deduplication, and cache directory storage.
- [x] **2. Track Context Menu ("3-Dots" Menu)**
  - **Location:** `ui/js/modules/views.js` (`renderTrackRows`), `ui/styles/components.css`
  - **Status:** ✅ Implemented glassmorphic context menu with Play Next, Add to Queue, Add to Playlist picker, Go to Artist/Album catalog filtering, Edit Metadata, and Delete Track with confirmation.
- [x] **3. Now Playing Queue Manager Panel**
  - **Location:** `ui/partials/player-full.html`, `ui/js/player.js`
  - **Status:** ✅ Implemented animated slide-out Queue Drawer in fullscreen player and sidebar panel with Remove track, Clear queue, active track indicator, and reactive `playback:queue`/`playback:track` updates.
- [x] **4. In-App Track Metadata / Tag Editor**
  - **Location:** `src/infrastructure/filesystem/metadata.rs` (`write_metadata`), `src/commands/library.rs` (`update_track_metadata`), `ui/partials/modal-tag-editor.html`, `ui/js/modules/library.js`
  - **Status:** ✅ Implemented `write_metadata` lofty tag writer (ID3v2, MP4, FLAC, Vorbis), synchronized SQLite persistence, and glassmorphic tag editor modal dialog.
- [x] **5. Batch / Playlist Downloader UI**
  - **Location:** `ui/partials/download.html`, `ui/js/modules/downloads.js`, `ui/js/youtube.js`
  - **Status:** ✅ Implemented YouTube playlist URL detection, interactive playlist preview table with select/deselect-all checkboxes, and sequential batch download pipeline with PO-token 403 auto-retry.

---

## 🚀 Milestone 2: Search, Scalability & Dynamic Playlists
> **Goal:** Optimize library discovery and automate smart catalog organization.

- [ ] **1. Dynamic Smart Playlist Engine & UI Builder**
  - **Location:** `src/domain/models/playlist.rs`, `src/commands/playlists.rs:390`
  - **Action:**
    - Refactor `get_playlist` to dynamically evaluate `SmartPlaylistCriteria` on the fly against SQLite instead of saving static track ID arrays.
    - Build a "Create Smart Playlist" modal with filter criteria: Genre, Year Range, Minimum Play Count, Rating, Date Added, and Sort Order.
- [ ] **2. SQLite FTS5 Full-Text Search Optimization**
  - **Location:** `src/infrastructure/database/repositories.rs:84`
  - **Problem:** Search currently uses `LIKE '%query%'` wildcards causing full table scans.
  - **Action:**
    - Create an SQLite `fts_tracks` virtual table using FTS5 (or trigram tokenizer) indexing `title`, `artist`, `album`.
    - Add SQLite triggers on `tracks` INSERT/UPDATE/DELETE to keep FTS index synchronized.
    - Update `TrackRepository::find_all` to execute BM25 ranked queries.
- [ ] **3. Virtualized Scrolling for Large Libraries (1,000+ tracks)**
  - **Location:** `ui/js/modules/views.js`, `ui/styles/components.css`
  - **Action:** Implement a virtual windowing list in `#library-track-list` that only renders visible rows + buffer to maintain 60fps scrolling on mobile devices.
- [ ] **4. SQLite Reader/Writer Concurrency Optimization**
  - **Location:** `src/infrastructure/database/mod.rs`
  - **Action:** Separate reader connections from writer connections with connection pooling or run heavy metadata batch insertions in dedicated `tokio::task::spawn_blocking` pools to prevent UI lock during storage scans.

---

## 🌐 Milestone 3: Ecosystem, Audio DSP & Platform Compliance
> **Goal:** Interoperability, advanced audio processing, and production app store compliance.

- [ ] **1. Real libp2p P2P Pairing Handshake Protocol**
  - **Location:** `src/domain/services/sync_service.rs:248`, `src/infrastructure/network.rs`
  - **Problem:** Pairing PIN validation is currently a local stub.
  - **Action:**
    - Implement a libp2p Request-Response protocol exchange (`PairingRequest`, `PairingResponse`).
    - Device B transmits entered PIN to Device A; Device A validates against its `active_pairing` and responds with authorization token.
- [ ] **2. Playlist Import & Export (M3U / M3U8 / PLS)**
  - **Location:** `src/commands/playlists.rs`
  - **Action:**
    - Export any playlist to standard `.m3u8` with relative or absolute file paths.
    - Import `.m3u` / `.m3u8` playlists by resolving referenced audio file paths in library.
- [ ] **3. Multi-Band Equalizer & Audio DSP Effects**
  - **Location:** `src/infrastructure/media/player.rs`
  - **Action:**
    - Implement a 5-band or 10-band biquad equalizer filter chain in the audio pipeline using `rodio` / `dasp`.
    - Provide presets: Flat, Bass Boost, Vocal, Acoustic, Rock, Electronic.
    - Save EQ settings in SQLite and persist across sessions.
- [ ] **4. Synchronized Lyrics Support (.LRC / USLT)**
  - **Location:** `src/infrastructure/filesystem/metadata.rs`, `ui/partials/player-full.html`
  - **Action:**
    - Parse embedded USLT ID3 frames or `.lrc` sidecar files matching the audio track filename.
    - Add a toggleable Lyrics view in the full-screen player that scrolls in sync with `playback:progress`.
- [ ] **5. macOS Gatekeeper & Windows Authenticode Signing**
  - **Location:** `.github/workflows/build.yml`, `tauri.conf.json`
  - **Action:**
    - macOS: Configure `APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` in GitHub Secrets and enable notarization.
    - Windows: Configure Azure Trusted Signing or code signing certificate in `bundle.windows.signCommand`.
- [ ] **6. Android Low-Memory Background Resilience**
  - **Location:** `src/infrastructure/media/background_service.rs`
  - **Action:** Ensure notification media controls (`play`, `pause`, `next`, `previous`, `seek`) act directly on the native `AudioPlayer` instance without dropping events when the Webview activity is killed by Android low-memory management.
