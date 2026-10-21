# 📋 Auralis Feature Roadmap & Capability Backlog

> [!NOTE]
> **Developer & Agent Notice**: This backlog tracks active and upcoming features. For architectural patterns, CI pipelines, and developer guidelines, see [**`AGENTS.md`**](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md).

---

## 🎯 Active Milestones & Priorities

### 📦 Milestone 1: Codebase Modularization (Active)
> **Goal:** Decompose the 9 monolithic source files exceeding 1,500 LOC into clean, single-responsibility modules with zero functional regressions.

- [x] **1. Extract P2P Sync from Frontend Downloads**
  - **Location:** `ui/js/modules/downloads.js` $\to$ `ui/js/modules/sync.js`, `ui/js/bridge.js`
  - **Status:** ✅ Completed (`syncMethods` composed onto Bridge; 205/205 JS tests pass).
- [x] **2. Modularize Master Stylesheet**
  - **Location:** `ui/styles/components.css` $\to$ `ui/styles/components/*.css`
  - **Status:** ✅ Completed (9 domain stylesheets imported via `@import`; 100% selector parity).
- [ ] **3. Decouple Unit Tests from Core Rust Files**
  - **Location:** `src/infrastructure/media/downloader.rs`, `src/commands/playback.rs`, `src/infrastructure/media/player.rs`
  - **Action:** Move 3,419 lines of `mod tests` into dedicated `tests.rs` modules, slashing production file sizes by 30%–45%.
- [ ] **4. Split Database Repositories**
  - **Location:** `src/infrastructure/database/repositories.rs`
  - **Action:** Split into `repositories/track.rs`, `repositories/playlist.rs`, `repositories/settings.rs`, and `repositories/sync.rs`.
- [ ] **5. Extract Resume Diagnostics from Playback Commands**
  - **Location:** `src/commands/playback.rs` $\to$ `commands/playback/diagnostics.rs`
  - **Action:** Decouple the 730-line `ResumeLog` and `ResumeProbe` state machine from command handlers.

---

### 🌐 Milestone 2: P2P Sync Modernization (`Iroh` + `Iroh-Blobs`)
> **Goal:** Replace `libp2p` with `Iroh` to enable seamless, battery-friendly, End-to-End Encrypted (E2EE) library synchronization and audio file transfers.

- [ ] **1. Dependency Migration (`Cargo.toml`)**
  - **Action:** Remove `libp2p` (stripping 150+ transitive dependencies); add `iroh` and `iroh-blobs`.
- [ ] **2. Persistent Node Identity**
  - **Location:** `src/infrastructure/network/endpoint.rs`, `src/infrastructure/database/repositories/sync.rs`
  - **Action:** Generate and persist an Ed25519 `SecretKey` in SQLite so every device maintains a permanent `NodeId` across restarts.
- [ ] **3. QR Code & Ticket-Based Device Pairing**
  - **Location:** `src/domain/services/sync_service.rs`, `ui/partials/sync.html`, `ui/js/modules/sync.js`
  - **Action:** Generate and scan compact `EndpointTicket` strings (containing NodeId, LAN addresses, and DERP relay fallback) via QR code or text paste.
- [ ] **4. Two-Phase Library & Audio File Transfer**
  - **Phase 1 (Catalog Sync):** Exchange track metadata, playlists, and favorites over QUIC stream `/auralis/sync/1`. Each track includes its content-verified `blake3_hash`.
  - **Phase 2 (Verified File Streaming):** Download missing audio files via `iroh-blobs` directly into `app_data_dir/downloads` and public storage with built-in resumable streaming.

---

### 🎶 Milestone 3: Dynamic Smart Playlists & Library Management
> **Goal:** Automate library organization and support playlist interoperability.

- [ ] **1. Dynamic Smart Playlist Engine**
  - **Location:** `src/domain/models/playlist.rs`, `src/commands/playlists.rs`
  - **Action:** Refactor `get_playlist` to dynamically evaluate `SmartPlaylistCriteria` against SQLite on the fly (Genre, Year, Min Play Count, Date Added).
- [ ] **2. Built-in Smart Presets**
  - **Action:** Add pre-configured smart playlists: *"Recently Added"*, *"Most Played"*, *"Favorites"*.
- [ ] **3. Playlist Import & Export (M3U / M3U8)**
  - **Location:** `src/commands/playlists.rs`
  - **Action:** Export playlists to standard `.m3u8` files and import external `.m3u8` files matching local audio tracks.
- [ ] **4. Virtualized Track List Scrolling**
  - **Location:** `ui/js/modules/views.js`
  - **Action:** Implement a virtual DOM windowing list for large libraries (1,000+ tracks) to maintain 60fps scrolling on mobile.

---

### 🎛️ Milestone 4: Audio DSP, Equalizer & Synchronized Lyrics
> **Goal:** Advanced audio shaping and synchronized playback enhancements.

- [ ] **1. Multi-Band Equalizer & DSP Presets**
  - **Location:** `src/infrastructure/media/player.rs`
  - **Action:** Implement a 5-band or 10-band biquad equalizer filter chain in `rodio` with presets (Flat, Bass Boost, Vocal, Rock) and SQLite persistence.
- [ ] **2. Synchronized Lyrics Support (.LRC & USLT)**
  - **Location:** `src/infrastructure/filesystem/metadata.rs`, `ui/partials/player-full.html`
  - **Action:** Parse embedded USLT ID3 tags or `.lrc` sidecar files; render synchronized scrolling lyrics in the fullscreen player.

---

### 🔐 Milestone 5: Production Distribution & Code Signing
> **Goal:** Production desktop store compliance and warning-free installation.

- [ ] **1. macOS Gatekeeper Notarization**
  - **Location:** `.github/workflows/build.yml`
  - **Action:** Configure Apple Developer ID certificate and notarization credentials in CI.
- [ ] **2. Windows Authenticode Signing**
  - **Location:** `.github/workflows/build.yml`, `tauri.conf.json`
  - **Action:** Integrate Azure Trusted Signing or code signing certificate in `bundle.windows.signCommand`.

---

## ✅ Completed Core Milestones (Verified in Production)

<details>
<summary><b>Click to expand completed foundational capabilities</b></summary>

- [x] **Direct YouTube Streaming Proxy (`StreamProxy`)**: Loopback proxy on `127.0.0.1:<port>/stream/<id>` streaming YouTube audio with custom headers (`UA`, `Referer`, `Origin`) to HTML5 `<audio>` with zero disk writes.
- [x] **1 MB 403 CDN Barrier Discovery & Fix**: Solved adaptive stream token caps by routing downloads and streaming through the `ANDROID` progressive muxed client (itag 18).
- [x] **Dynamic Night Mode Synchronization**: Real-time app theme tracking matching Android/OS system dark/light mode toggles.
- [x] **Background Playback on Android**: JNI-driven foreground `MediaPlaybackService` with lockscreen & notification media controls.
- [x] **Dual-Save Media Storage**: Downloads saved to internal sandboxed storage and published publicly to `Download/Auralis/` via Android `MediaStore`.
- [x] **Strict Stream Verification & Resume**: HTTP Range resumption, atomic `.part` rename, and lofty container duration integrity validation.
- [x] **Queue Management & Auto-Advance**: Sample-accurate seeking, queue drawer, and 250ms progress watcher events.
- [x] **Album Artwork Extraction & Caching**: Lofty extraction of embedded cover art with deduplicated thumbnail cache.
- [x] **Tag Editor & Metadata Writing**: In-app ID3v2/MP4/FLAC tag writing with lofty.
- [x] **16 KB Android Page Alignment**: CI-enforced ELF LOAD segment alignment for Android 15/16.

</details>
