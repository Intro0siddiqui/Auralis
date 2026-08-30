# 📋 Auralis v2 Feature Roadmap & Capability TODO

This document tracks all missing, stubbed, or partially wired features across the Auralis codebase (Desktop and Android targets), prioritized into actionable implementation milestones.

## 🚨 Milestone 0: Download & Stream Integrity Engine Overhaul (Independent High-Priority Focus)
> **Goal:** Eliminate partial/truncated audio downloads, enforce strict stream integrity validation, auto-resume broken HTTP streams, and prevent truncated audio playback.

- [ ] **1. Strict Stream Completion & Content-Length Verification**
  - **Location:** `src/infrastructure/media/downloader.rs:478-523`
  - **Problem:** When an HTTP stream drops or terminates prematurely (e.g. at 27 seconds of a 3-minute song), the downloader encounters EOF (`chunk_opt == None`), exits the stream loop, and immediately marks the download as `Completed` without checking if all bytes were received.
  - **Action:**
    - If `total_bytes` is known from Content-Length or stream metadata, verify `downloaded_bytes == total_bytes`.
    - If `downloaded_bytes < total_bytes`, flag as `IncompleteStream` and trigger automatic Range-based retry before marking complete.
- [ ] **2. Automatic Stream Reconnection & HTTP Range Retry Loop**
  - **Location:** `src/infrastructure/media/downloader.rs:460-515`
  - **Problem:** Transient mobile connection drops or YouTube googlevideo stream resets cause immediate failure or truncated partial files.
  - **Action:**
    - Implement an exponential backoff retry loop (up to 3-5 attempts) inside `run_stream`.
    - Automatically issue `Range: bytes=<downloaded_bytes>-` on reconnection to seamlessly append remaining bytes to the temporary file.
    - If the server responds with 403 Forbidden on retry, request an updated PO-token / client rotation from `youtube.js` before retrying the range request.
- [ ] **3. Post-Download Audio Container & Duration Integrity Validation**
  - **Location:** `src/infrastructure/media/downloader.rs:515-525`, `src/infrastructure/filesystem/metadata.rs`
  - **Problem:** Truncated audio files with intact container headers are indexed into SQLite with full metadata duration (e.g. 3m:15s), but player only plays 27s/1m.
  - **Action:**
    - Before moving file from `.tmp` to final destination and before publishing to MediaStore, probe the downloaded file with `lofty` / `rodio::Decoder`.
    - Check that the actual decoded audio stream duration matches the expected duration within a small tolerance (e.g., ±2 seconds).
    - Check for decoding corruption / truncated container frames (e.g. missing MP4 `moov` or truncated AAC packets).
- [ ] **4. Atomic File Publication & Corrupt Partial Cleanup**
  - **Location:** `src/infrastructure/media/downloader.rs:190-210`, `android_downloads.rs`
  - **Action:**
    - Stream directly to a dedicated `.part` / `.tmp` file (`app_data_dir/downloads/.tmp/<id>.part`).
    - Only rename to final destination filename (`app_data_dir/downloads/<title>.<ext>`) once audio integrity and duration validation pass 100%.
    - If a download fails, is cancelled, or fails validation after max retries, automatically delete the incomplete `.part` file so corrupt tracks never enter the user library or MediaStore.
- [ ] **5. Duration Mismatch Diagnostic & Auto-Repair in Player**
  - **Location:** `src/infrastructure/media/player.rs:154-165`
  - **Action:**
    - When `AudioPlayer::play` detects a severe duration mismatch between DB and actual decoded stream (e.g. DB says 180s, stream only has 27s), log a clear diagnostic warning and update track record to reflect true playable length or flag track for re-download.

---

## 🎯 Milestone 1: High-Impact Core Completeness & Missing UI Wiring
> **Goal:** Connect existing backend Tauri commands to the frontend UI and resolve major visual/metadata gaps.

- [ ] **1. Embedded Album Artwork Extraction & Caching**
  - **Location:** `src/infrastructure/filesystem/metadata.rs:58`
  - **Problem:** Currently only checks for sidecar files (`<audio>.jpg`). Does not extract embedded ID3/FLAC/MP4 APIC picture frames.
  - **Action:**
    - Update `MetadataExtractor` to read `tag.pictures()` via `lofty`.
    - Hash picture bytes and write to cache: `app_data_dir/artwork_cache/<hash>.jpg`.
    - Set `Track.album_art_path` to the cached image path.
- [ ] **2. Track Context Menu ("3-Dots" Menu)**
  - **Location:** `ui/js/modules/views.js:799` (`renderTrackRows`), `ui/partials/`
  - **Action:** Add a dropdown/popover menu to each track row offering:
    - [ ] *Play Next* (insert at current index + 1 in queue)
    - [ ] *Add to Queue* (wire to `commands::playback::add_to_queue`)
    - [ ] *Add to Playlist* (open playlist selector dialog)
    - [ ] *Go to Artist* / *Go to Album* (navigate and filter catalog)
    - [ ] *Edit Metadata* (open tag editor modal)
    - [ ] *Delete Track* (wire to `commands::library::delete_tracks`)
- [ ] **3. Now Playing Queue Manager Panel**
  - **Location:** `ui/partials/player-full.html`, `ui/partials/queue.html`
  - **Action:**
    - Build a slide-out queue drawer displaying active track and upcoming list.
    - Wire "Remove" button per track (`commands::playback::remove_from_queue`).
    - Wire "Clear Queue" button (`commands::playback::clear_queue`).
    - Support drag-and-drop reordering.
- [ ] **4. In-App Track Metadata / Tag Editor**
  - **Location:** `src/commands/library.rs:69` (`update_track_metadata`), `ui/partials/modal-tag-editor.html`
  - **Action:**
    - Create a modal dialog allowing users to edit Title, Artist, Album, Genre, Year, and Track Number.
    - Save updates via `update_track_metadata` command and write tags to file with `lofty`.
- [ ] **5. Batch / Playlist Downloader UI**
  - **Location:** `src/commands/downloads.rs:91` (`download_playlist`), `ui/partials/download.html`
  - **Action:** Extend the Download view to detect YouTube playlist URLs, display track list confirmation, and trigger `download_playlist` in the background.

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
