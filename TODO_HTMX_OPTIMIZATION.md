# ⚡ Auralis Frontend: JavaScript-to-HTMX & Rust Refactoring Roadmap

This document outlines the strategy for reducing excessive client-side JavaScript by offloading catalog rendering, sorting, filtering, and state binding to native Rust SQLite queries and server-rendered HTMX partials.

---

## 📊 Summary of JavaScript Redundancy Audit

- **Current Footprint:** ~5,200 LOC (~235 KB) of JavaScript across `ui/js/`.
- **Target Reduction:** Eliminate **~1,200 to 1,500 LOC (~60% of UI JS logic)**.
- **Goal:** Replace heavy DOM string concatenation (`.innerHTML = ...`), browser memory caching (`this.tracks`, `this._albumMap`), and client-side array sorting with instant, native server-rendered HTMX partials.

---

## 🎯 Phase 1: High-Impact View Offloading (Catalog & Lists) — ✅ COMPLETED
> **Goal:** Stop transferring massive JSON arrays to browser memory; deliver pre-rendered HTML cards and rows directly from SQLite.

- [x] **1. Albums & Artists Grid Generation (`views.js`, `commands/library.rs`)**
  - **Accomplished:**
    - [x] Added SQLite aggregation queries in Rust (`get_albums_grid_html` and `get_artists_grid_html`).
    - [x] Removed `this._albumMap` and `this._artistMap` memory caches from `views.js`.
    - [x] Converted `loadAlbumsView` and `loadArtistsView` to declarative single-call HTML rendering.

- [x] **2. Library Track Sorting, Filtering & Row Generation (`views.js`, `commands/library.rs`)**
  - **Accomplished:**
    - [x] Added `get_library_tracks_html` backend command supporting `sort_by`, `downloaded_only`, `artist`, `album`, and `search`.
    - [x] Removed client-side `Array.prototype.sort()` and manual DOM row generation from `views.js`.
    - [x] Connected filter bar controls directly to backend queries.

- [x] **3. Home Shelves & Recommendations (`views.js`, `commands/library.rs`, `home.html`)**
  - **Accomplished:**
    - [x] Added `get_home_shelves_html` command in Rust: pre-renders "Recently Added" shelf cards and "Continue Listening" rows directly from SQLite.
    - [x] Removed client-side slicing and manual card/row construction from `views.js`.

- [x] **4. In-Library Search Debouncing (`views.js`, `commands/library.rs`, `search.html`)**
  - **Accomplished:**
    - [x] Added `get_search_results_html` backend command.
    - [x] Refactored `loadSearchView` in `views.js` to render search results via backend query.


---

## 🛠️ Phase 2: Form & State Binding Simplification — ✅ COMPLETED
> **Goal:** Replace manual input listeners and JSON getters with standard server-rendered HTML and streamlined live bindings.

- [x] **1. Settings Form Hydration & Live Binding (`views.js`, `settings.html`)**
  - **Accomplished:**
    - [x] Replaced 210+ lines of manual input querying in `views.js` with a concise input binding loop and unified change/click listeners (~130 LOC removed).
    - [x] Streamlined instant theme switching (`setTheme`) and settings persistence.

- [x] **2. Queue Panel Rendering (`player.js`, `commands/playback.rs`)**
  - **Accomplished:**
    - [x] Added `get_queue_html` command in Rust: pre-renders Now Playing card and upcoming track rows with batch `find_by_ids` query.
    - [x] Refactored `renderQueuePanel` in `player.js` to invoke `get_queue_html` and removed `renderQueueTrackRow` + manual string concatenation (~70 LOC removed).

- [x] **3. P2P Sync Devices View (`views.js`)**
  - **Accomplished:**
    - [x] Streamlined sync view state rendering and removed duplicate memory buffers.

---

## 🧹 Phase 3: DOM Listeners & Memory Cleanup — ✅ COMPLETED
> **Goal:** Eliminate redundant listeners, MutationObservers, and duplicate memory caches.

- [x] **1. Eliminate `MutationObserver` in Full-Screen Modal (`player.js`)**
  - **Accomplished:** Replaced heavy `MutationObserver` on `#overlay-root` with clean `htmx:afterSwap` event-driven hydration.
- [x] **2. Consolidate Event Delegation (`views.js`, `core.js`)**
  - **Accomplished:** Relies on unified global event delegation in `core.js` for `[data-role="play-row"]` and `[data-role="play-card"]`.
- [x] **3. Clean Up Bridge Memory Cache**
  - **Accomplished:** Removed duplicate `this.tracks`, `this._albumMap`, `this._artistMap`, and `this.currentSettings` caches from `Bridge`.

---

## 🛡️ Areas Where JavaScript MUST Be Preserved (Do Not Refactor)

1. **YouTube InnerTube Resolver & BotGuard (`youtube.js`, `po_token.js`):** Deciphers audio streams and mints PO-tokens in-browser without external binary sidecars (`yt-dlp`/`ffmpeg`).
2. **High-Frequency 250ms Audio Scrubbing (`player.js`):** Smooth drag-scrubbing on `<div class="progress-track">` requires instant local DOM updates to avoid IPC latency.
3. **Web Audio & MediaSession API (`player.js`):** Browser platform API for OS hardware keys and lockscreen controls.
4. **Android SAF Scoped Storage Ingestion (`library.js`):** Reading local files as base64 byte chunks via `FileReader` across Tauri IPC.
5. **Keyboard Shortcuts (`player.js`):** Local keydown interception for `Space` (Play/Pause), `ArrowLeft`/`ArrowRight` (Seek), `KeyS` (Shuffle), `KeyR` (Repeat).
