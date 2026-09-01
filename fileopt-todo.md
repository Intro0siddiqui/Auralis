# 📐 Auralis Codebase: File Optimization & Monolithic Decomposition Roadmap

This document outlines the planned structural refactorings to reduce monolithic file sizes, extract single-responsibility modules, simplify oversized methods, and improve code maintainability across the Auralis Rust backend and JavaScript frontend.

---

## 📊 Summary of Large Files Identified for Modularization

| Subsystem | File | Current LOC | Size | Target Architecture |
|---|---|---|---|---|
| **Database** | `src/infrastructure/database/repositories.rs` | ~1,455 LOC | 50 KB | Split into repository domain modules (`tracks.rs`, `playlists.rs`, `settings.rs`, `sync.rs`, `mappers.rs`). |
| **Commands** | `src/commands/library.rs` | ~1,272 LOC | 49 KB | Split into `commands/library/{crud.rs, scanner.rs, formatting.rs}`. |
| **Network** | `src/infrastructure/network.rs` | ~1,216 LOC | 44 KB | Split into `infrastructure/network/{behaviour.rs, swarm.rs, aliases.rs, sync_engine.rs}`. |
| **Frontend UI** | `ui/js/modules/views.js` | ~1,111 LOC | 42 KB | Extract modal managers (`modals.js`) and track context menus (`context-menu.js`). |
| **Downloader** | `src/infrastructure/media/downloader.rs` | ~1,087 LOC | 43 KB | Extract stream client retry engine and audio stream validation modules. |
| **Frontend Player** | `ui/js/player.js` | ~1,100 LOC | 45 KB | Extract reusable `SliderController` for player-bar and full-screen progress/volume sliders. |
| **YouTube Resolver** | `ui/js/youtube.js` | ~868 LOC | 36 KB | Extract `nativeFetch` HTTP bridge and decompose monolithic 540-line `resolve()` method. |

---

## 🎯 Phase 1: Rust Backend Modularization

- [ ] **1. Decompose Database Repositories (`src/infrastructure/database/repositories.rs`)**
  - Extract `TrackRepository` and query builders into `repositories/tracks.rs`.
  - Extract `PlaylistRepository` into `repositories/playlists.rs`.
  - Extract `SettingsRepository` into `repositories/settings.rs`.
  - Extract `SyncRepository` and device pairing records into `repositories/sync.rs`.
  - Extract SQL row mappers and escaping helpers into `repositories/mappers.rs`.

- [ ] **2. Decompose Network Swarm & Libp2p Engine (`src/infrastructure/network.rs`)**
  - Extract libp2p composite `NetworkBehaviour` into `network/behaviour.rs`.
  - Extract central `Swarm` event handling and loop into `network/swarm.rs`.
  - Extract peer alias resolution and device tracking into `network/aliases.rs`.
  - Extract P2P request-response file transfer engine into `network/sync_engine.rs`.

- [ ] **3. Decompose Library Commands (`src/commands/library.rs`)**
  - Extract track CRUD and batch queries into `commands/library/crud.rs`.
  - Extract directory scan orchestration into `commands/library/scanner.rs`.
  - Extract HTML formatting and tag rendering into `commands/library/formatting.rs`.

- [ ] **4. Decompose Media Downloader (`src/infrastructure/media/downloader.rs`)**
  - Extract HTTP Range streaming and exponential backoff retry client into `downloader/stream.rs`.
  - Extract post-download audio header & duration validation into `downloader/validator.rs`.

---

## 🎨 Phase 2: Frontend Module & Component Decomposition

- [ ] **1. Decompose Frontend Views (`ui/js/modules/views.js`)**
  - Extract Tag Editor and Playlist Selection modals into `ui/js/modules/modals.js`.
  - Extract 3-dots floating context menu into `ui/js/modules/context-menu.js`.
  - Retain view loaders and navigation routing in `views.js`.

- [ ] **2. Decompose Player Controller (`ui/js/player.js`)**
  - Create `SliderController` (`ui/js/utils/slider.js`) encapsulating touch/mouse dragging, client coordinate calculations, step snapping, and percent normalization.
  - Extract Full-Screen player overlay wiring into `ui/js/modules/player-fullscreen.js`.
  - Consolidate transport controls into unified player state machine.

- [ ] **3. Decompose YouTube Resolver (`ui/js/youtube.js`)**
  - Extract `nativeFetch` / Tauri HTTP IPC bridge into shared `ui/js/utils/http.js`.
  - Split `resolve()` into modular pipeline stages:
    - `resolveDirectAudio(url)`
    - `resolveInnerTubeStream(videoId, opts)`
    - `decipherStreamUrl(format, client, opts)`
    - `buildClientHeaders(winningClient)`

---

## 🛡️ Guiding Principles for Modularization

1. **Documentation & Tests Integrity**: Every extracted module must maintain all unit and regression tests.
2. **Zero Functional Regression**: All Tauri commands, HTMX swaps, and YouTube stream resolution pipelines must behave identically.
3. **No Circular Dependencies**: Ensure clear inward dependency flow between services, repositories, and commands.
